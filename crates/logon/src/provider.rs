#![allow(non_snake_case)]
use comunicacao_lan::protocol::{demo_catalog, OrderView, Plan};
use lan_transport::{Action, PublicConfig, Reply};
use std::{ffi::c_void, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}}, time::{Duration, Instant}};
use windows::{core::*, Win32::{Foundation::*, Graphics::Gdi::*, System::{Com::*, LibraryLoader::*}, UI::Shell::*}};
use zeroize::Zeroizing;

pub const PROVIDER_CLSID: GUID = GUID::from_u128(0xe463ae81_2ba8_4fd3_948a_4baf1809818e);
const IMAGE: u32 = 0;
const TITLE: u32 = 1;
const STATUS: u32 = 2;
const PLAN: u32 = 3;
const GENERATE: u32 = 4;
const SUBMIT: u32 = 5;
const FIELDS: u32 = 6;

#[derive(Clone)]
struct Cache {
    plans: Vec<Plan>, selected: usize, pending: Option<OrderView>,
    remaining: u64, revision: u64, updated: Instant, message: String,
}
impl Default for Cache {
    fn default() -> Self {
        Self { plans: demo_catalog(), selected: 0, pending: None, remaining: 0, revision: 0,
            updated: Instant::now(), message: "Conectando ao serviço da estação…".into() }
    }
}
impl Cache {
    fn active(&self) -> bool { self.remaining > self.updated.elapsed().as_secs() }
    fn text(&self) -> String {
        if self.active() { return "Pagamento de teste confirmado. Preparando a entrada no Windows…".into(); }
        if let Some(order) = &self.pending {
            format!("PIX DE TESTE · R$ {:.2}\nNenhum dinheiro é cobrado. Confirme a cobrança no painel de simulação.\n{}", order.amount_cents as f64 / 100.0, order.id)
        } else { self.message.clone() }
    }
}

struct Controller {
    public: String, sid: String,
    cache: Mutex<Cache>, command: Mutex<Option<String>>,
    selected: AtomicBool, auto_pending: AtomicBool, attempted: AtomicU64,
    scenario: AtomicU32, alive: Mutex<Option<Arc<AtomicBool>>>,
    // Somente streams COM marshaled atravessam threads; interfaces não são Send.
    credential_events: Mutex<Option<(usize, usize, u64)>>,
    generation: AtomicU64,
    offered: AtomicBool,
}
impl Controller {
    fn new(config: PublicConfig) -> Result<Arc<Self>> {
        lan_transport::validate_public_key(&config.agent_public_key).map_err(|_| Error::from(E_ACCESSDENIED))?;
        let sid = config.client_sid.filter(|sid| sid.starts_with("S-1-")).ok_or_else(|| Error::from(E_ACCESSDENIED))?;
        Ok(Arc::new(Self { public: config.agent_public_key, sid, cache: Mutex::new(Cache::default()),
            command: Mutex::new(None), selected: AtomicBool::new(false), auto_pending: AtomicBool::new(false),
            attempted: AtomicU64::new(u64::MAX), scenario: AtomicU32::new(0), alive: Mutex::new(None),
            credential_events: Mutex::new(None), generation: AtomicU64::new(0), offered: AtomicBool::new(true) }))
    }
    fn snapshot(&self) -> Cache { self.cache.lock().map(|cache| cache.clone()).unwrap_or_default() }
    fn stop(&self) {
        if let Ok(mut current) = self.alive.lock() { if let Some(alive) = current.take() { alive.store(false, Ordering::Release); } }
    }
    fn buy(&self) -> Result<()> {
        let cache = self.snapshot();
        if cache.pending.is_some() || cache.active() { return Err(E_ACCESSDENIED.into()); }
        let plan = cache.plans.get(cache.selected).ok_or_else(|| Error::from(E_INVALIDARG))?;
        *self.command.lock().map_err(|_| Error::from(E_FAIL))? = Some(plan.id.clone());
        Ok(())
    }
}

fn string(text: &str) -> Result<PWSTR> {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let memory = unsafe { CoTaskMemAlloc(wide.len() * 2) }.cast::<u16>();
    if memory.is_null() { return Err(E_OUTOFMEMORY.into()); }
    unsafe { std::ptr::copy_nonoverlapping(wide.as_ptr(), memory, wide.len()); }
    Ok(PWSTR(memory))
}

fn marshal<T: Interface>(value: &T) -> Result<usize> {
    let unknown: IUnknown = value.cast()?;
    Ok(unsafe { CoMarshalInterThreadInterfaceInStream(&T::IID, &unknown)? }.into_raw() as usize)
}
unsafe fn unmarshal<T: Interface>(raw: usize) -> Result<T> {
    // CoGetInterfaceAndReleaseStream consome a referência do stream.
    let stream = std::mem::ManuallyDrop::new(IStream::from_raw(raw as *mut c_void));
    CoGetInterfaceAndReleaseStream::<_, T>(&*stream)
}
fn discard_stream(raw: usize) {
    unsafe {
        let stream = IStream::from_raw(raw as *mut c_void);
        let _ = CoReleaseMarshalData(&stream);
    }
}

fn bitmap(cache: &Cache) -> Result<HBITMAP> {
    let code = cache.pending.as_ref().and_then(|order| qrcode::QrCode::new(order.qr_payload.as_bytes()).ok());
    let size = 240usize;
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = size as i32;
    info.bmiHeader.biHeight = -(size as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB.0;
    let mut bits = std::ptr::null_mut();
    let image = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
    let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), size * size) };
    pixels.fill(0xffffffff);
    if let Some(code) = code {
        let cells = code.width() + 8;
        let scale = size / cells;
        let origin = (size - cells * scale) / 2;
        for y in 0..code.width() { for x in 0..code.width() {
            if code[(x, y)] == qrcode::Color::Dark {
                for py in 0..scale { for px in 0..scale {
                    pixels[(origin + (y + 4) * scale + py) * size + origin + (x + 4) * scale + px] = 0xff000000;
                } }
            }
        } }
    } else {
        // Identidade visual simples; nenhum QR é exibido sem cobrança criada.
        pixels.fill(0xff0c1320);
        for y in 50..190 { for x in 60..85 { pixels[y * size + x] = 0xff61ddbf; } }
        for y in 165..190 { for x in 60..180 { pixels[y * size + x] = 0xff61ddbf; } }
    }
    Ok(image)
}

fn poll_controller(controller: Arc<Controller>, events_raw: usize, context: usize, alive: Arc<AtomicBool>) {
    unsafe { if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() { return; } }
    let result = (|| -> Result<()> {
        let events: ICredentialProviderEvents = unsafe { unmarshal(events_raw)? };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| Error::from(E_FAIL))?;
        let mut fields: Option<(ICredentialProviderCredentialEvents, ICredentialProviderCredential, u64)> = None;
        let mut previous_image = String::new();
        while alive.load(Ordering::Acquire) {
            if let Ok(mut pending) = controller.credential_events.lock() {
                if let Some((raw_events, raw_credential, generation)) = pending.take() {
                    let callback = unsafe { unmarshal::<ICredentialProviderCredentialEvents>(raw_events) };
                    let credential = unsafe { unmarshal::<ICredentialProviderCredential>(raw_credential) };
                    fields = callback.ok().zip(credential.ok()).map(|(e, c)| (e, c, generation));
                    previous_image.clear();
                }
            }
            let buy = controller.command.lock().ok().and_then(|mut command| command.take());
            if let Some(plan_id) = buy {
                if let Err(error) = runtime.block_on(lan_transport::call(&controller.public, Action::Buy { plan_id })) {
                    if let Ok(mut cache) = controller.cache.lock() { cache.message = error; }
                }
            }
            let result = runtime.block_on(lan_transport::call(&controller.public, Action::Status));
            if let Ok(mut cache) = controller.cache.lock() {
                match result {
                    Ok(Reply::Status { session, message, workstation: true, .. }) => {
                        cache.remaining = session.remaining_seconds;
                        cache.revision = session.revision;
                        cache.pending = session.pending_order;
                        cache.updated = Instant::now();
                        cache.message = message.unwrap_or_else(|| "Escolha o tempo e gere um Pix de teste. Nenhum valor será cobrado.".into());
                    }
                    _ => { cache.remaining = 0; cache.message = "Agente de bloqueio indisponível. A entrada permanece bloqueada.".into(); }
                }
            }
            if !alive.load(Ordering::Acquire) { break; }
            let cache = controller.snapshot();
            if let Some((callback, credential, generation)) = &fields {
                if *generation == controller.generation.load(Ordering::Acquire) {
                    let text = HSTRING::from(cache.text());
                    unsafe {
                        let _ = callback.SetFieldString(credential, STATUS, &text);
                        let disabled = cache.pending.is_some() || cache.active();
                        let _ = callback.SetFieldInteractiveState(credential, PLAN, if disabled { CPFIS_DISABLED } else { CPFIS_NONE });
                        let _ = callback.SetFieldInteractiveState(credential, GENERATE, if disabled { CPFIS_DISABLED } else { CPFIS_NONE });
                        let _ = callback.SetFieldInteractiveState(credential, SUBMIT, if cache.active() { CPFIS_NONE } else { CPFIS_DISABLED });
                    }
                    let id = cache.pending.as_ref().map(|order| order.id.clone()).unwrap_or_else(|| "logo".into());
                    if previous_image != id {
                        if let Ok(image) = bitmap(&cache) {
                            unsafe { let _ = callback.SetFieldBitmap(credential, IMAGE, image); }
                            // LogonUI assume a propriedade do bitmap do campo.
                        }
                        previous_image = id;
                    }
                }
            }
            if !cache.active() { controller.auto_pending.store(false, Ordering::Release); }
            if cache.active() && controller.selected.load(Ordering::Acquire)
                && controller.attempted.load(Ordering::Acquire) != cache.revision
                && !controller.auto_pending.swap(true, Ordering::AcqRel) {
                unsafe { let _ = events.CredentialsChanged(context); }
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        Ok(())
    })();
    let _ = result;
    unsafe { CoUninitialize(); }
}

#[implement(ICredentialProvider, ICredentialProviderSetUserArray)]
struct Provider { controller: Arc<Controller>, credential: ICredentialProviderCredential }
impl Provider {
    fn create(config: PublicConfig) -> Result<ICredentialProvider> {
        let controller = Controller::new(config)?;
        let credential: ICredentialProviderCredential2 = Credential { controller: controller.clone() }.into();
        Ok(Self { controller, credential: credential.cast()? }.into())
    }
}
impl Drop for Provider { fn drop(&mut self) { self.controller.stop(); } }
impl ICredentialProviderSetUserArray_Impl for Provider_Impl {
    fn SetUserArray(&self, users: Ref<'_, ICredentialProviderUserArray>) -> Result<()> {
        let users = users.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let mut offered = false;
        unsafe {
            for index in 0..users.GetCount()? {
                let sid = users.GetAt(index)?.GetSid()?;
                offered |= sid.to_string().ok().as_deref() == Some(self.controller.sid.as_str());
                CoTaskMemFree(Some(sid.0.cast()));
            }
        }
        self.controller.offered.store(offered, Ordering::Release);
        Ok(())
    }
}
impl ICredentialProvider_Impl for Provider_Impl {
    fn SetUsageScenario(&self, scenario: CREDENTIAL_PROVIDER_USAGE_SCENARIO, _flags: u32) -> Result<()> {
        if scenario != CPUS_LOGON && scenario != CPUS_UNLOCK_WORKSTATION { return Err(E_NOTIMPL.into()); }
        self.controller.scenario.store(scenario.0 as u32, Ordering::Release);
        Ok(())
    }
    fn SetSerialization(&self, _serialization: *const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn Advise(&self, callback: Ref<'_, ICredentialProviderEvents>, context: usize) -> Result<()> {
        self.controller.stop();
        let raw = marshal(callback.as_ref().ok_or_else(|| Error::from(E_POINTER))?)?;
        let alive = Arc::new(AtomicBool::new(true));
        *self.controller.alive.lock().map_err(|_| Error::from(E_FAIL))? = Some(alive.clone());
        let controller = self.controller.clone();
        std::thread::spawn(move || poll_controller(controller, raw, context, alive));
        Ok(())
    }
    fn UnAdvise(&self) -> Result<()> { self.controller.stop(); Ok(()) }
    fn GetFieldDescriptorCount(&self) -> Result<u32> { Ok(FIELDS) }
    fn GetFieldDescriptorAt(&self, index: u32) -> Result<*mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR> {
        let (kind, label) = match index {
            IMAGE => (CPFT_TILE_IMAGE, "Pix de teste"), TITLE => (CPFT_LARGE_TEXT, "Comunicação LAN"),
            STATUS => (CPFT_SMALL_TEXT, "Estado"), PLAN => (CPFT_COMBOBOX, "Tempo de uso"),
            GENERATE => (CPFT_COMMAND_LINK, "Confirmar pacote e gerar Pix de teste"), SUBMIT => (CPFT_SUBMIT_BUTTON, "Entrar"),
            _ => return Err(E_INVALIDARG.into()),
        };
        let descriptor = unsafe { CoTaskMemAlloc(std::mem::size_of::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>()) }.cast::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>();
        if descriptor.is_null() { return Err(E_OUTOFMEMORY.into()); }
        let label = match string(label) { Ok(label) => label, Err(error) => { unsafe { CoTaskMemFree(Some(descriptor.cast())); } return Err(error); } };
        unsafe { descriptor.write(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR { dwFieldID: index, cpft: kind, pszLabel: label, guidFieldType: GUID::zeroed() }); }
        Ok(descriptor)
    }
    fn GetCredentialCount(&self, count: *mut u32, default: *mut u32, auto: *mut BOOL) -> Result<()> {
        if count.is_null() || default.is_null() || auto.is_null() { return Err(E_POINTER.into()); }
        let offered = self.controller.offered.load(Ordering::Acquire);
        let selected = offered && self.controller.selected.load(Ordering::Acquire);
        unsafe { *count = u32::from(offered); *default = if selected { 0 } else { u32::MAX }; *auto = (selected && self.controller.snapshot().active() && self.controller.auto_pending.load(Ordering::Acquire)).into(); }
        Ok(())
    }
    fn GetCredentialAt(&self, index: u32) -> Result<ICredentialProviderCredential> {
        if index != 0 || !self.controller.offered.load(Ordering::Acquire) { return Err(E_INVALIDARG.into()); }
        Ok(self.credential.clone())
    }
}

#[implement(ICredentialProviderCredential2)]
struct Credential { controller: Arc<Controller> }
impl ICredentialProviderCredential2_Impl for Credential_Impl {
    fn GetUserSid(&self) -> Result<PWSTR> { string(&self.controller.sid) }
}
impl ICredentialProviderCredential_Impl for Credential_Impl {
    fn Advise(&self, callback: Ref<'_, ICredentialProviderCredentialEvents>) -> Result<()> {
        let callback = callback.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let credential = <Self as ComObjectInterface<ICredentialProviderCredential2>>::as_interface_ref(self).to_owned().cast::<ICredentialProviderCredential>()?;
        let events_raw = marshal(callback)?;
        let credential_raw = match marshal(&credential) { Ok(raw) => raw, Err(error) => { discard_stream(events_raw); return Err(error); } };
        let generation = self.controller.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut pending = self.controller.credential_events.lock().map_err(|_| Error::from(E_FAIL))?;
        if let Some((events, credential, _)) = pending.replace((events_raw, credential_raw, generation)) { discard_stream(events); discard_stream(credential); }
        Ok(())
    }
    fn UnAdvise(&self) -> Result<()> {
        self.controller.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut pending) = self.controller.credential_events.lock() {
            if let Some((events, credential, _)) = pending.take() { discard_stream(events); discard_stream(credential); }
        }
        Ok(())
    }
    fn SetSelected(&self) -> Result<BOOL> {
        self.controller.selected.store(true, Ordering::Release);
        let cache = self.controller.snapshot();
        let ready = cache.active() && self.controller.attempted.load(Ordering::Acquire) != cache.revision;
        self.controller.auto_pending.store(ready, Ordering::Release);
        Ok(ready.into())
    }
    fn SetDeselected(&self) -> Result<()> { self.controller.selected.store(false, Ordering::Release); self.controller.auto_pending.store(false, Ordering::Release); Ok(()) }
    fn GetFieldState(&self, field: u32, state: *mut CREDENTIAL_PROVIDER_FIELD_STATE, interactive: *mut CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE) -> Result<()> {
        if field >= FIELDS { return Err(E_INVALIDARG.into()); }
        if state.is_null() || interactive.is_null() { return Err(E_POINTER.into()); }
        let cache = self.controller.snapshot();
        unsafe {
            *state = if field == IMAGE || field == TITLE { CPFS_DISPLAY_IN_BOTH } else { CPFS_DISPLAY_IN_SELECTED_TILE };
            *interactive = if (field == SUBMIT && !cache.active()) || ((field == PLAN || field == GENERATE) && (cache.pending.is_some() || cache.active())) { CPFIS_DISABLED } else { CPFIS_NONE };
        }
        Ok(())
    }
    fn GetStringValue(&self, field: u32) -> Result<PWSTR> {
        match field {
            TITLE => string("Comunicação LAN"), STATUS => string(&self.controller.snapshot().text()),
            GENERATE => string("Confirmar pacote e gerar Pix de teste"), SUBMIT => string("Entrar"),
            _ => Err(E_INVALIDARG.into()),
        }
    }
    fn GetBitmapValue(&self, field: u32) -> Result<HBITMAP> { if field != IMAGE { return Err(E_INVALIDARG.into()); } bitmap(&self.controller.snapshot()) }
    fn GetCheckboxValue(&self, _field: u32, _checked: *mut BOOL, _label: *mut PWSTR) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn GetSubmitButtonValue(&self, field: u32) -> Result<u32> { if field == SUBMIT { Ok(PLAN) } else { Err(E_INVALIDARG.into()) } }
    fn GetComboBoxValueCount(&self, field: u32, items: *mut u32, selected: *mut u32) -> Result<()> {
        if field != PLAN { return Err(E_INVALIDARG.into()); }
        if items.is_null() || selected.is_null() { return Err(E_POINTER.into()); }
        let cache = self.controller.snapshot();
        unsafe { *items = cache.plans.len() as u32; *selected = cache.selected as u32; }
        Ok(())
    }
    fn GetComboBoxValueAt(&self, field: u32, item: u32) -> Result<PWSTR> {
        if field != PLAN { return Err(E_INVALIDARG.into()); }
        let cache = self.controller.snapshot();
        let plan = cache.plans.get(item as usize).ok_or_else(|| Error::from(E_INVALIDARG))?;
        string(&format!("{} · R$ {:.2}", plan.label, plan.amount_cents as f64 / 100.0))
    }
    fn SetStringValue(&self, _field: u32, _value: &PCWSTR) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn SetCheckboxValue(&self, _field: u32, _checked: BOOL) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn SetComboBoxSelectedValue(&self, field: u32, selected: u32) -> Result<()> {
        if field != PLAN { return Err(E_INVALIDARG.into()); }
        let mut cache = self.controller.cache.lock().map_err(|_| Error::from(E_FAIL))?;
        if cache.pending.is_some() || cache.active() || selected as usize >= cache.plans.len() { return Err(E_INVALIDARG.into()); }
        cache.selected = selected as usize;
        Ok(())
    }
    fn CommandLinkClicked(&self, field: u32) -> Result<()> { if field != GENERATE { return Err(E_INVALIDARG.into()); } self.controller.buy() }
    fn GetSerialization(&self, response: *mut CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE, serialization: *mut CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION, text: *mut PWSTR, icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON) -> Result<()> {
        if response.is_null() || serialization.is_null() || text.is_null() || icon.is_null() { return Err(E_POINTER.into()); }
        unsafe { *response = CPGSR_NO_CREDENTIAL_NOT_FINISHED; *serialization = CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION::default(); *text = PWSTR::null(); *icon = CPSI_NONE; }
        let cache = self.controller.snapshot();
        self.controller.auto_pending.store(false, Ordering::Release);
        if !cache.active() { unsafe { *text = string("Aguardando confirmação do Pix de teste.")?; } return Ok(()); }
        self.controller.attempted.store(cache.revision, Ordering::Release);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| Error::from(E_FAIL))?;
        let unlock = self.controller.scenario.load(Ordering::Acquire) == CPUS_UNLOCK_WORKSTATION.0 as u32;
        match runtime.block_on(lan_transport::call(&self.controller.public, Action::SerializeLogon { unlock })) {
            Ok(Reply::Serialized { credentials }) => {
                let bytes = Zeroizing::new(hex::decode(&credentials.buffer_hex).map_err(|_| Error::from(E_ACCESSDENIED))?);
                if bytes.is_empty() || bytes.len() > 16384 { return Err(E_ACCESSDENIED.into()); }
                let memory = unsafe { CoTaskMemAlloc(bytes.len()) }.cast::<u8>();
                if memory.is_null() { return Err(E_OUTOFMEMORY.into()); }
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), memory, bytes.len());
                    *serialization = CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION { ulAuthenticationPackage: credentials.package, clsidCredentialProvider: PROVIDER_CLSID, cbSerialization: bytes.len() as u32, rgbSerialization: memory };
                    *response = CPGSR_RETURN_CREDENTIAL_FINISHED;
                }
            }
            _ => unsafe { *text = string("Entrada recusada. Verifique o serviço e o tempo disponível.")?; *icon = CPSI_ERROR; },
        }
        Ok(())
    }
    fn ReportResult(&self, status: NTSTATUS, _substatus: NTSTATUS, text: *mut PWSTR, icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON) -> Result<()> {
        if !text.is_null() { unsafe { *text = if status.0 < 0 { string("O Windows recusou a entrada. Procure o atendente; nenhum crédito adicional foi consumido.")? } else { PWSTR::null() }; } }
        if !icon.is_null() { unsafe { *icon = if status.0 < 0 { CPSI_ERROR } else { CPSI_NONE }; } }
        Ok(())
    }
}

fn installed_config() -> Result<PublicConfig> {
    let mut module = HMODULE::default();
    unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        PCWSTR(DllGetClassObject as *const () as *const u16), &mut module)?; }
    let mut buffer = vec![0u16; 32768];
    let count = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    if count == 0 || count >= buffer.len() { return Err(E_FAIL.into()); }
    let path = std::path::PathBuf::from(String::from_utf16(&buffer[..count]).map_err(|_| Error::from(E_FAIL))?);
    let file = path.parent().ok_or_else(|| Error::from(E_FAIL))?.join("agent-public.json");
    let bytes = std::fs::read(file).map_err(|_| Error::from(E_ACCESSDENIED))?;
    if bytes.len() > 2048 { return Err(E_ACCESSDENIED.into()); }
    serde_json::from_slice(&bytes).map_err(|_| Error::from(E_ACCESSDENIED))
}

#[implement(IClassFactory)]
struct Factory;
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, iid: *const GUID, output: *mut *mut c_void) -> Result<()> {
        if outer.as_ref().is_some() { return Err(CLASS_E_NOAGGREGATION.into()); }
        if iid.is_null() || output.is_null() { return Err(E_POINTER.into()); }
        unsafe { *output = std::ptr::null_mut(); }
        let provider = Provider::create(installed_config()?)?;
        unsafe { provider.query(iid, output).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> { Ok(()) }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(class: *const GUID, iid: *const GUID, output: *mut *mut c_void) -> HRESULT {
    if class.is_null() || iid.is_null() || output.is_null() { return E_POINTER; }
    *output = std::ptr::null_mut();
    if *class != PROVIDER_CLSID { return CLASS_E_CLASSNOTAVAILABLE; }
    let factory: IClassFactory = Factory.into();
    factory.query(iid, output)
}
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Callbacks podem estar marshaled para o worker. Manter a DLL até LogonUI sair.
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider() -> ICredentialProvider {
        Provider::create(PublicConfig { agent_public_key: lan_transport::Identity::generate().public_hex(), client_sid: Some("S-1-5-21-1-2-3-1001".into()) }).unwrap()
    }
    #[test]
    fn credential_tile_fields_combo_and_bitmap_are_available() {
        let provider = provider();
        unsafe {
            provider.SetUsageScenario(CPUS_LOGON, 0).unwrap();
            assert!(provider.SetUsageScenario(CPUS_CREDUI, 0).is_err());
            assert_eq!(provider.GetFieldDescriptorCount().unwrap(), FIELDS);
            for id in 0..FIELDS {
                let descriptor = provider.GetFieldDescriptorAt(id).unwrap();
                assert_eq!((*descriptor).dwFieldID, id);
                CoTaskMemFree(Some((*descriptor).pszLabel.0.cast()));
                CoTaskMemFree(Some(descriptor.cast()));
            }
            let credential = provider.GetCredentialAt(0).unwrap();
            assert!(provider.GetCredentialAt(1).is_err());
            let (mut items, mut selected) = (0, 0);
            credential.GetComboBoxValueCount(PLAN, &mut items, &mut selected).unwrap();
            assert_eq!(items, 3);
            let image = credential.GetBitmapValue(IMAGE).unwrap();
            assert!(!image.0.is_null());
            let _ = DeleteObject(image.into());
        }
    }
    #[test]
    fn unpaid_credential_does_not_serialize_or_autologon() {
        let provider = provider();
        unsafe {
            provider.SetUsageScenario(CPUS_LOGON, 0).unwrap();
            let credential = provider.GetCredentialAt(0).unwrap();
            assert!(!credential.SetSelected().unwrap().as_bool());
            let mut response = CPGSR_RETURN_CREDENTIAL_FINISHED;
            let mut serialization = CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION::default();
            let mut text = PWSTR::null();
            let mut icon = CPSI_NONE;
            credential.GetSerialization(&mut response, &mut serialization, &mut text, &mut icon).unwrap();
            assert_eq!(response, CPGSR_NO_CREDENTIAL_NOT_FINISHED);
            assert!(serialization.rgbSerialization.is_null());
            CoTaskMemFree(Some(text.0.cast()));
        }
    }
}
