#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use windows::{core::*, Win32::{System::Com::*, Graphics::Gdi::*, UI::Shell::*}};
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let provider: ICredentialProvider = CoCreateInstance(&lan_logon::PROVIDER_CLSID, None, CLSCTX_INPROC_SERVER)?;
        provider.SetUsageScenario(CPUS_LOGON, 0)?;
        if provider.GetFieldDescriptorCount()? != 6 { return Err("Campos do provedor inválidos.".into()); }
        let credential = provider.GetCredentialAt(0)?;
        let credential2: ICredentialProviderCredential2 = credential.cast()?;
        let sid = credential2.GetUserSid()?;
        let sid_text = sid.to_string()?;
        CoTaskMemFree(Some(sid.0.cast()));
        let root = std::path::PathBuf::from(std::env::var("ProgramFiles")?).join("ComunicacaoLanAgent");
        let config: lan_transport::PublicConfig = serde_json::from_slice(&std::fs::read(root.join("agent-public.json"))?)?;
        if config.client_sid.as_deref() != Some(sid_text.as_str()) { return Err("SID do provedor não corresponde à instalação.".into()); }
        let image = credential.GetBitmapValue(0)?;
        if image.0.is_null() { return Err("Bitmap do provedor inválido.".into()); }
        let _ = DeleteObject(image.into());
        drop(credential2); drop(credential); drop(provider);
        CoUninitialize();
    }
    println!("Credential provider activation verified");
    Ok(())
}
#[cfg(not(windows))]
fn main() { println!("Teste de ativação disponível somente no Windows."); }
