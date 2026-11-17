//! Bloqueio por sessão do Windows. Nenhuma janela em tela cheia é usada como barreira.
use crate::{PrivateConfig, WorkstationConfig};
use comunicacao_lan::lease::Lease;
use lan_transport::{Action, LogonSerialization, Reply};
use std::{path::Path, time::{Duration, Instant}};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
        Authorization::ConvertSidToStringSidW,
        Authentication::Identity::{LsaConnectUntrusted, LsaDeregisterLogonProcess, LsaLookupAuthenticationPackage, LSA_STRING},
        Credentials::CredProtectW,
        Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN}},
    System::{RemoteDesktop::*, Threading::{GetCurrentProcess, OpenProcessToken}},
};
use zeroize::{Zeroize, Zeroizing};

fn token_sid(token: HANDLE) -> Option<String> {
    let mut length = 0;
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length); }
    if length == 0 || length > 65536 { return None; }
    // Alinhamento de TOKEN_USER preservado, em vez de reinterpretar um Vec<u8>.
    let mut memory = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token, TokenUser, memory.as_mut_ptr().cast(), length, &mut length) } == 0 { return None; }
    let user = unsafe { &*(memory.as_ptr().cast::<TOKEN_USER>()) };
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 { return None; }
    let mut size = 0;
    while size < 184 && unsafe { *text.add(size) } != 0 { size += 1; }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, size) }).ok();
    unsafe { LocalFree(text.cast()); }
    sid
}

pub fn is_system_token(token: HANDLE) -> bool { token_sid(token).as_deref() == Some("S-1-5-18") }
pub fn is_system() -> bool {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 { return false; }
    let system = is_system_token(token);
    unsafe { CloseHandle(token); }
    system
}

fn client_sessions(sid: &str) -> Result<Vec<u32>, String> {
    let (mut sessions, mut count) = (std::ptr::null_mut(), 0);
    if unsafe { WTSEnumerateSessionsW(std::ptr::null_mut(), 0, 1, &mut sessions, &mut count) } == 0 {
        return Err("Falha ao consultar sessões do Windows.".into());
    }
    let mut result = Vec::new();
    if count > 0 && !sessions.is_null() {
        for session in unsafe { std::slice::from_raw_parts(sessions, count as usize) } {
            if session.SessionId == 0 || session.State != WTSActive { continue; }
            let mut token = std::ptr::null_mut();
            if unsafe { WTSQueryUserToken(session.SessionId, &mut token) } == 0 { continue; }
            let matches = token_sid(token).as_deref() == Some(sid);
            unsafe { CloseHandle(token); }
            if matches { result.push(session.SessionId); }
        }
    }
    unsafe { WTSFreeMemory(sessions.cast()); }
    Ok(result)
}

/// Atua somente sobre o SID provisionado; jamais sobre a sessão de manutenção.
pub fn disconnect_client(sid: &str) -> Result<(), String> {
    for session in client_sessions(sid)? {
        if unsafe { WTSDisconnectSession(std::ptr::null_mut(), session, 0) } == 0 {
            return Err("Windows recusou o bloqueio da sessão de cliente.".into());
        }
    }
    Ok(())
}

fn warn_client(sid: &str, seconds: u64) {
    let Ok(sessions) = client_sessions(sid) else { return; };
    let title: Vec<u16> = "Comunicação LAN".encode_utf16().collect();
    let text: Vec<u16> = format!("Seu tempo acaba em {seconds} segundos. Renove para continuar. Ao terminar, o Windows bloqueará a sessão sem fechar os programas.").encode_utf16().collect();
    for session in sessions {
        let mut response = 0;
        unsafe { WTSSendMessageW(std::ptr::null_mut(), session, title.as_ptr(), (title.len() * 2) as u32,
            text.as_ptr(), (text.len() * 2) as u32, 0x30, 8, &mut response, 0); }
    }
}

pub fn protect_password(password: &str) -> Result<String, String> {
    if password.len() < 32 || password.len() > 256 || !password.is_ascii() {
        return Err("Use uma senha aleatória de 32 a 256 caracteres para a conta dedicada.".into());
    }
    let mut bytes = Zeroizing::new(password.as_bytes().to_vec());
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_mut_ptr() };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    if unsafe { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null(),
        CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN, &mut output) } == 0 {
        return Err("Não foi possível proteger a credencial com DPAPI.".into());
    }
    let encrypted = hex::encode(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) });
    unsafe { LocalFree(output.pbData.cast()); }
    Ok(encrypted)
}

fn password(config: &WorkstationConfig) -> Result<Zeroizing<String>, String> {
    let mut bytes = hex::decode(&config.protected_password).map_err(|_| "Credencial protegida inválida.")?;
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_mut_ptr() };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    if unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) } == 0 {
        return Err("Não foi possível recuperar a credencial protegida.".into());
    }
    let clear = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let decoded = std::str::from_utf8(clear).map(|text| Zeroizing::new(text.to_owned())).map_err(|_| "Credencial inválida.".into());
    clear.zeroize();
    unsafe { LocalFree(output.pbData.cast()); }
    decoded
}

fn protected_windows_password(password: &str) -> Result<Zeroizing<Vec<u16>>, String> {
    let mut wide = Zeroizing::new(password.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
    let (mut length, mut kind) = (0, 0);
    unsafe { CredProtectW(1, wide.as_mut_ptr(), wide.len() as u32, std::ptr::null_mut(), &mut length, &mut kind); }
    if length == 0 || length > 16384 { return Err("Windows recusou a proteção da credencial de entrada.".into()); }
    let mut output = Zeroizing::new(vec![0u16; length as usize]);
    if unsafe { CredProtectW(1, wide.as_mut_ptr(), wide.len() as u32, output.as_mut_ptr(), &mut length, &mut kind) } == 0 {
        return Err("Windows recusou a proteção da credencial de entrada.".into());
    }
    output.truncate(length as usize);
    while output.last() == Some(&0) { output.pop(); }
    Ok(output)
}

#[repr(C)]
struct Unicode { length: u16, maximum: u16, buffer: usize }
#[repr(C)]
struct Interactive { message: i32, domain: Unicode, user: Unicode, password: Unicode }
#[repr(C)]
struct Unlock { logon: Interactive, luid: [u32; 2] }

fn pack(domain: &str, user: &str, protected: &[u16], unlock: bool) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut bytes = Zeroizing::new(vec![0; std::mem::size_of::<Unlock>()]);
    bytes[..4].copy_from_slice(&(if unlock { 7i32 } else { 2i32 }).to_le_bytes());
    let domain: Vec<u16> = domain.encode_utf16().collect();
    let user: Vec<u16> = user.encode_utf16().collect();
    for (offset, text) in [(std::mem::offset_of!(Interactive, domain), domain.as_slice()),
        (std::mem::offset_of!(Interactive, user), user.as_slice()),
        (std::mem::offset_of!(Interactive, password), protected)] {
        let length = text.len().checked_mul(2).filter(|n| *n <= u16::MAX as usize).ok_or("Credencial muito longa.")?;
        let pointer = bytes.len();
        bytes[offset..offset + 2].copy_from_slice(&(length as u16).to_le_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&(length as u16).to_le_bytes());
        let position = offset + std::mem::offset_of!(Unicode, buffer);
        bytes[position..position + std::mem::size_of::<usize>()].copy_from_slice(&pointer.to_le_bytes());
        for character in text { bytes.extend_from_slice(&character.to_le_bytes()); }
    }
    Ok(bytes)
}

pub fn serialize(config: &PrivateConfig, unlock: bool) -> Result<LogonSerialization, String> {
    if !is_system() { return Err("Serialização disponível somente no contexto do Windows.".into()); }
    let gate = config.workstation.as_ref().ok_or("Entrada do Windows não configurada.")?;
    let secret = password(gate)?;
    let protected = protected_windows_password(&secret)?;
    let domain = std::env::var("COMPUTERNAME").map_err(|_| "Nome do computador indisponível.")?;
    let packed = pack(&domain, &gate.local_user, &protected, unlock)?;
    let mut handle = std::ptr::null_mut();
    if unsafe { LsaConnectUntrusted(&mut handle) } < 0 { return Err("Não foi possível consultar a autenticação do Windows.".into()); }
    let mut name_bytes = b"Negotiate".to_vec();
    let name = LSA_STRING { Length: name_bytes.len() as u16, MaximumLength: name_bytes.len() as u16, Buffer: name_bytes.as_mut_ptr() };
    let mut package = 0;
    let result = unsafe { LsaLookupAuthenticationPackage(handle, &name, &mut package) };
    unsafe { LsaDeregisterLogonProcess(handle); }
    if result < 0 { return Err("Pacote de autenticação indisponível.".into()); }
    Ok(LogonSerialization { buffer_hex: hex::encode(packed.as_slice()), package })
}

/// Processo separado: mantém a contagem mesmo se o processo do serviço falhar.
pub async fn watchdog(config: PrivateConfig, private_path: &Path, instance: String) -> Result<(), String> {
    if config.workstation.is_none() || !is_system() { return Err("Supervisor exige modo de entrada Windows e SYSTEM.".into()); }
    let public = lan_transport::Identity::from_secret(&config.agent_signing_secret)?.public_hex();
    let started = Instant::now();
    let mut lease = Lease::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            let before = Instant::now();
            if let Ok(reply) = lan_transport::call(&public, Action::Watchdog).await {
                if tx.send((reply, before.elapsed(), Instant::now())).await.is_err() { break; }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    });
    let marker = private_path.with_file_name("maintenance.stop");
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            update = rx.recv() => {
                if let Some((Reply::Watchdog { session, instance: current }, latency, received)) = update {
                    if current != instance { return Ok(()); }
                    if session.machine_id == "PC-01" {
                        lease.apply(&session, started.elapsed(), latency.saturating_add(received.elapsed()));
                    }
                }
            }
            _ = interval.tick() => {
                if marker.exists() { disconnect_client(&config.client_sid)?; return Ok(()); }
                let now = started.elapsed();
                if lease.remaining(now).is_zero() {
                    // Falha de API não encerra o supervisor: repetir no próximo tick.
                    let _ = disconnect_client(&config.client_sid);
                } else if let Some(seconds) = lease.warning(now) { warn_client(&config.client_sid, seconds); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serialized_strings_are_offsets_and_header_padding_is_zero() {
        let packed = pack("MACHINE", "LanCliente", &[65, 66], true).unwrap();
        assert_eq!(i32::from_le_bytes(packed[..4].try_into().unwrap()), 7);
        let pointer = std::mem::offset_of!(Interactive, user) + std::mem::offset_of!(Unicode, buffer);
        let offset = usize::from_le_bytes(packed[pointer..pointer + std::mem::size_of::<usize>()].try_into().unwrap());
        assert!(offset >= std::mem::size_of::<Unlock>() && offset < packed.len());
        assert!(packed[std::mem::offset_of!(Unlock, luid)..std::mem::size_of::<Unlock>()].iter().all(|byte| *byte == 0));
    }
    #[test]
    fn dpapi_roundtrip_and_tampered_blob() {
        let clear = "a-long-test-password-32-characters-or-more";
        let encrypted = protect_password(clear).unwrap();
        let mut config = WorkstationConfig { local_user: "LanCliente".into(), protected_password: encrypted };
        assert_eq!(password(&config).unwrap().as_str(), clear);
        config.protected_password = "00".repeat(64);
        assert!(password(&config).is_err());
    }
}
