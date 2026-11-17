use crate::{handle, poll, AgentState, Backend, PrivateConfig, Shared};
use lan_transport::{read_frame, write_frame, Action, Identity, Reply, Request, MAX_FRAME, MAX_REQUEST, PIPE_NAME};
use std::{ffi::{c_void, OsString}, sync::{Arc, OnceLock}, time::Duration};
use tokio::{net::windows::named_pipe::{NamedPipeServer, ServerOptions}, sync::{watch, Mutex, Semaphore}};
use windows_service::{define_windows_service, service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType}, service_control_handler::{self, ServiceControlHandlerResult}, service_dispatcher};
use windows_sys::Win32::{Foundation::LocalFree, Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES}};
use std::os::windows::io::AsRawHandle;

pub const SERVICE_NAME: &str = "ComunicacaoLanAgent";
static CONFIG: OnceLock<PrivateConfig> = OnceLock::new();
define_windows_service!(ffi_service_main, service_main);

pub fn dispatch(config: PrivateConfig) -> Result<(), String> {
    CONFIG.set(config).map_err(|_| "Configuração duplicada.")?;
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|_| "Inicie este modo pelo Gerenciador de Serviços do Windows.".into())
}

fn service_main(_arguments: Vec<OsString>) {
    let (stop_tx, stop_rx) = watch::channel(false);
    let status = match service_control_handler::register(SERVICE_NAME, move |control| {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => { let _ = stop_tx.send(true); ServiceControlHandlerResult::NoError }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    }) { Ok(status) => status, Err(_) => return };
    let report = |current_state, exit_code| status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS, current_state,
        controls_accepted: if current_state == ServiceState::Running { ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN } else { ServiceControlAccept::empty() },
        exit_code: ServiceExitCode::Win32(exit_code), checkpoint: 0,
        wait_hint: Duration::ZERO, process_id: None,
    });
    if report(ServiceState::Running, 0).is_err() { return; }
    let result = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()
        .map_err(|_| "Runtime indisponível.".into())
        .and_then(|runtime| runtime.block_on(run(CONFIG.get().unwrap().clone(), stop_rx)));
    let _ = report(ServiceState::Stopped, if result.is_ok() { 0 } else { 1 });
}

/// O handle mantém o descritor válido até CreateNamedPipe terminar de copiá-lo.
fn pipe(client_sid: &str, service_sid: &str, first: bool) -> Result<NamedPipeServer, String> {
    if !crate::valid_sid_text(client_sid) || !crate::valid_sid_text(service_sid) { return Err("SID inválido.".into()); }
    // Criar novas instâncias é reservado a SYSTEM/LocalService. O cliente recebe
    // leitura/escrita sem FILE_CREATE_PIPE_INSTANCE (bit 0x4 de GENERIC_WRITE).
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{service_sid})(A;;0x0012019b;;;{client_sid})");
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: string UTF-16 terminada em NUL; saída é liberada com LocalFree.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(wide.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
        return Err("Falha ao construir ACL do pipe.".into());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor, bInheritHandle: 0,
    };
    // SAFETY: atributos e descritor permanecem vivos durante a chamada.
    let result = unsafe { ServerOptions::new().first_pipe_instance(first)
        .reject_remote_clients(true).max_instances(32).in_buffer_size(8192).out_buffer_size(8192)
        .create_with_security_attributes_raw(PIPE_NAME, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast()) };
    unsafe { LocalFree(descriptor as _); }
    result.map_err(|_| "Não foi possível reservar o pipe local.".into())
}

async fn serve(mut listener: NamedPipeServer, config: PrivateConfig, backend: Arc<Backend>, state: Shared) -> Result<(), String> {
    let identity = Arc::new(Identity::from_secret(&config.agent_signing_secret)?);
    let slots = Arc::new(Semaphore::new(16));
    loop {
        listener.connect().await.map_err(|_| "Falha ao aceitar conexão local.")?;
        // Reservar a próxima instância antes de soltar a anterior elimina a
        // janela sem servidor em que outro processo poderia ocupar o nome.
        let next = pipe(&config.client_sid, &config.service_sid, false)?;
        let mut connected = std::mem::replace(&mut listener, next);
        let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
        let (identity, backend, state, config) = (identity.clone(), backend.clone(), state.clone(), config.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let operation = async {
                let bytes = read_frame(&mut connected, MAX_REQUEST).await.map_err(|_| ())?;
                let request: Request = serde_json::from_slice(&bytes).map_err(|_| ())?;
                request.validate().map_err(|_| ())?;
                let reply = match request.action {
                    Action::SerializeLogon { unlock } => {
                        if !caller_is_system(&connected) || !state.lock().await.authorized() || !state.lock().await.workstation {
                            Reply::Failure { message: "Entrada não autorizada.".into() }
                        } else {
                            match crate::workstation::serialize(&config, unlock) {
                                Ok(credentials) => Reply::Serialized { credentials: Box::new(credentials) },
                                Err(message) => Reply::Failure { message },
                            }
                        }
                    }
                    Action::Watchdog => {
                        if caller_is_system(&connected) { state.lock().await.watchdog_status() }
                        else { Reply::Failure { message: "Supervisor não autorizado.".into() } }
                    }
                    action => handle(action, &backend, &state).await,
                };
                let signed = identity.sign(&request.nonce, reply).map_err(|_| ())?;
                let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&signed).map_err(|_| ())?);
                write_frame(&mut connected, &bytes, MAX_FRAME).await.map_err(|_| ())
            };
            let _ = tokio::time::timeout(Duration::from_secs(4), operation).await;
        });
    }
}

/// Identidade obtida do token de impersonação do kernel, nunca de um campo RPC.
/// RevertToSelf ocorre antes de qualquer await ou operação em outro contexto.
fn caller_is_system(pipe: &NamedPipeServer) -> bool {
    use windows_sys::Win32::{Foundation::CloseHandle,
        Security::{RevertToSelf, TOKEN_QUERY},
        System::{Pipes::ImpersonateNamedPipeClient, Threading::{GetCurrentThread, OpenThreadToken}}};
    if unsafe { ImpersonateNamedPipeClient(pipe.as_raw_handle()) } == 0 { return false; }
    let mut token = std::ptr::null_mut();
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } != 0;
    let valid = opened && crate::workstation::is_system_token(token);
    if opened { unsafe { CloseHandle(token); } }
    let reverted = unsafe { RevertToSelf() } != 0;
    if !reverted { std::process::abort(); }
    valid
}

pub async fn run(config: PrivateConfig, mut stop: watch::Receiver<bool>) -> Result<(), String> {
    config.validate()?;
    let listener = pipe(&config.client_sid, &config.service_sid, true)?;
    let backend = Arc::new(Backend::new(config.clone())?);
    let state = Arc::new(Mutex::new(AgentState::default()));
    let workstation = config.workstation.is_some();
    if workstation && !crate::workstation::is_system() { return Err("Modo de entrada Windows exige SYSTEM.".into()); }
    state.lock().await.workstation = workstation;
    let private_path = std::env::args().nth(2).map(std::path::PathBuf::from);
    if workstation {
        let path = private_path.as_ref().ok_or("Configuração privada ausente.")?;
        let marker = path.with_file_name("maintenance.stop");
        if marker.exists() { std::fs::remove_file(marker).map_err(|_| "Não foi possível sair da manutenção.")?; }
    }
    let instance = state.lock().await.instance.clone();
    let spawn = || -> Result<Option<std::process::Child>, String> {
        if !workstation { return Ok(None); }
        let path = private_path.as_ref().ok_or("Configuração privada ausente.")?;
        std::process::Command::new(std::env::current_exe().map_err(|_| "Agente não localizado.")?)
            .arg("--watchdog").arg(path).arg(&instance).spawn().map(Some)
            .map_err(|_| "Não foi possível iniciar o supervisor independente.".into())
    };
    let mut watchdog = spawn()?;
    let monitor = tokio::spawn(poll(backend.clone(), state.clone()));
    let client_sid = config.client_sid.clone();
    let mut serving = tokio::spawn(serve(listener, config, backend, state.clone()));
    let mut monitor = monitor;
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            result = &mut serving => return result.map_err(|_| "Canal local encerrou inesperadamente.".to_string())?,
            _ = &mut monitor => return Err("Monitor do servidor encerrou inesperadamente.".into()),
            _ = interval.tick() => {
                if workstation {
                    if !state.lock().await.authorized() { let _ = crate::workstation::end_client_session(&client_sid); }
                    if let Some(child) = watchdog.as_mut() {
                        if child.try_wait().map_err(|_| "Falha ao verificar supervisor.")?.is_some() { watchdog = spawn()?; }
                    }
                }
            }
            _ = stop.changed() => {
                if workstation {
                    let _ = crate::workstation::end_client_session(&client_sid);
                    if let Some(path) = private_path.as_ref() {
                        std::fs::write(path.with_file_name("maintenance.stop"), b"maintenance").map_err(|_| "Falha ao registrar manutenção.")?;
                    }
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    if let Some(child) = watchdog.as_mut() { let _ = child.kill(); let _ = child.wait(); }
                }
                return Ok(());
            }
        }
    }
}
