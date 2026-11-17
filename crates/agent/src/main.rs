use lan_agent::PrivateConfig;
use lan_transport::{Identity, PublicConfig};

fn main() {
    if let Err(error) = run() { eprintln!("{error}"); std::process::exit(1); }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--provision") => {
            if args.len() != 4 { return Err("Use --provision <private.json> <agent-public.json>.".into()); }
            let identity = Identity::generate();
            let config = PrivateConfig {
                station_token: std::env::var("LAN_STATION_TOKEN").map_err(|_| "Defina LAN_STATION_TOKEN.")?,
                server_public_key: std::env::var("LAN_SERVER_PUBLIC_KEY").map_err(|_| "Defina LAN_SERVER_PUBLIC_KEY.")?,
                agent_signing_secret: identity.secret_hex(),
                client_sid: std::env::var("LAN_ALLOWED_CLIENT_SID").map_err(|_| "Defina LAN_ALLOWED_CLIENT_SID.")?,
                service_sid: std::env::var("LAN_SERVICE_SID").map_err(|_| "Defina LAN_SERVICE_SID.")?,
                workstation: workstation_config()?,
            };
            config.validate()?;
            // O instalador cria diretórios com ACL antes desta operação.
            write_new(&args[2], &serde_json::to_vec(&config).map_err(|_| "Configuração inválida.")?)?;
            write_new(&args[3], &serde_json::to_vec(&PublicConfig { agent_public_key: identity.public_hex(), client_sid: config.workstation.as_ref().map(|_| config.client_sid.clone()) }).map_err(|_| "Configuração inválida.")?)?;
            Ok(())
        }
        #[cfg(windows)]
        Some("--service") => {
            let path = args.get(2).ok_or("Caminho da configuração privada ausente.")?;
            let bytes = std::fs::read(path).map_err(|_| "Não foi possível ler a configuração privada.")?;
            if bytes.len() > 8192 { return Err("Configuração privada excede o limite.".into()); }
            let config: PrivateConfig = serde_json::from_slice(&bytes).map_err(|_| "Configuração privada inválida.")?;
            config.validate()?;
            lan_agent::windows::dispatch(config)
        }
        #[cfg(windows)]
        Some("--watchdog") => {
            let path = args.get(2).ok_or("Configuração privada ausente.")?;
            let instance = args.get(3).ok_or("Instância de serviço ausente.")?.clone();
            let config: PrivateConfig = serde_json::from_slice(&std::fs::read(path).map_err(|_| "Configuração indisponível.")?).map_err(|_| "Configuração inválida.")?;
            config.validate()?;
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Runtime indisponível.")?;
            runtime.block_on(lan_agent::workstation::watchdog(config, std::path::Path::new(path), instance))
        }
        #[cfg(windows)]
        Some("--probe") => {
            let public = lan_transport::installed_public_key()?;
            let action = match args.get(2).map(String::as_str) {
                Some("--buy") => lan_transport::Action::Buy { plan_id: args.get(3).ok_or("Pacote ausente.")?.clone() },
                Some("--check-logon-denied") => lan_transport::Action::SerializeLogon { unlock: false },
                _ => lan_transport::Action::Status,
            };
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Runtime indisponível.")?;
            let reply = runtime.block_on(lan_transport::call(&public, action))?;
            if args.get(2).map(String::as_str) == Some("--check-logon-denied") {
                return match reply { lan_transport::Reply::Failure { .. } => { println!("Logon serialization denied"); Ok(()) }, _ => Err("Serialização de entrada exposta indevidamente.".into()) };
            }
            println!("{}", serde_json::to_string(&reply).map_err(|_| "Resposta inválida.")?);
            Ok(())
        }
        _ => Err("Agente experimental: --provision, --service ou --probe (Windows).".into()),
    }
}

fn workstation_config() -> Result<Option<lan_agent::WorkstationConfig>, String> {
    if std::env::var("LAN_WINDOWS_GATE").as_deref() != Ok("1") { return Ok(None); }
    #[cfg(windows)]
    {
        let password = zeroize::Zeroizing::new(std::env::var("LAN_WINDOWS_CLIENT_PASSWORD").map_err(|_| "Senha de provisionamento ausente.")?);
        std::env::remove_var("LAN_WINDOWS_CLIENT_PASSWORD");
        Ok(Some(lan_agent::WorkstationConfig {
            local_user: std::env::var("LAN_WINDOWS_CLIENT_USER").map_err(|_| "Conta de cliente ausente.")?,
            protected_password: lan_agent::workstation::protect_password(&password)?,
        }))
    }
    #[cfg(not(windows))]
    { Err("Entrada Windows somente disponível no Windows.".into()) }
}

fn write_new(path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path)
        .map_err(|_| "Não foi possível criar configuração. Não sobrescreva uma instalação existente.")?;
    file.write_all(bytes).map_err(|_| "Falha ao gravar configuração.")?;
    file.sync_all().map_err(|_| "Falha ao persistir configuração.".into())
}
