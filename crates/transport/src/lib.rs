//! RPC de escopo limitado. Não existe operação para conceder tempo localmente.
use comunicacao_lan::protocol::{Plan, SessionView};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const PIPE_NAME: &str = r"\\.\pipe\ComunicacaoLan.Agent.v1";
pub const MAX_FRAME: usize = 64 * 1024;
pub const MAX_REQUEST: usize = 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Status,
    Catalog,
    Buy { plan_id: String },
    /// Somente o LogonUI/SYSTEM pode solicitar serialização de entrada.
    SerializeLogon { unlock: bool },
    /// Canal do processo supervisor, também restrito a SYSTEM.
    Watchdog,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub nonce: String,
    pub action: Action,
}
impl Request {
    pub fn new(action: Action) -> Self {
        Self { nonce: uuid::Uuid::new_v4().to_string(), action }
    }
    pub fn validate(&self) -> Result<(), String> {
        let nonce = uuid::Uuid::parse_str(&self.nonce).map_err(|_| "Nonce inválido.")?;
        if nonce.get_version_num() != 4 { return Err("Nonce inválido.".into()); }
        if let Action::Buy { plan_id } = &self.action {
            if plan_id.is_empty() || plan_id.len() > 32 { return Err("Pacote inválido.".into()); }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Status { session: SessionView, online: bool, message: Option<String>, #[serde(default)] workstation: bool },
    Catalog { plans: Vec<Plan> },
    OrderCreated,
    Failure { message: String },
    Serialized { credentials: Box<LogonSerialization> },
    Watchdog { session: SessionView, instance: String },
}

#[derive(Clone, Deserialize, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct LogonSerialization { pub buffer_hex: String, pub package: u32 }
impl std::fmt::Debug for LogonSerialization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("LogonSerialization([REDACTED])") }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Payload { nonce: String, reply: Reply }

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedReply {
    pub payload: String,
    pub signature: String,
}
impl Drop for SignedReply { fn drop(&mut self) { self.payload.zeroize(); } }
impl std::fmt::Debug for SignedReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("SignedReply([REDACTED])") }
}

#[derive(Clone)]
pub struct Identity(SigningKey);
impl Identity {
    pub fn generate() -> Self { Self(SigningKey::generate(&mut rand::rngs::OsRng)) }
    pub fn from_secret(secret: &str) -> Result<Self, String> {
        let mut bytes = [0; 32];
        hex::decode_to_slice(secret, &mut bytes).map_err(|_| "Chave privada inválida.")?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }
    pub fn secret_hex(&self) -> String { hex::encode(self.0.to_bytes()) }
    pub fn public_hex(&self) -> String { hex::encode(self.0.verifying_key().to_bytes()) }
    pub fn sign(&self, nonce: &str, reply: Reply) -> Result<SignedReply, String> {
        let payload = serde_json::to_string(&Payload { nonce: nonce.into(), reply }).map_err(|_| "Falha ao codificar resposta.")?;
        let signature = hex::encode(self.0.sign(payload.as_bytes()).to_bytes());
        Ok(SignedReply { payload, signature })
    }
}

pub fn validate_public_key(public: &str) -> Result<VerifyingKey, String> {
    let mut bytes = [0; 32];
    hex::decode_to_slice(public, &mut bytes).map_err(|_| "Chave pública inválida.")?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "Chave pública inválida.".into())
}

pub fn verify(public: &str, nonce: &str, signed: SignedReply) -> Result<Reply, String> {
    if signed.payload.len() > MAX_FRAME || signed.signature.len() != 128 {
        return Err("Resposta fora do limite.".into());
    }
    let key = validate_public_key(public)?;
    let mut signature = [0; 64];
    hex::decode_to_slice(&signed.signature, &mut signature).map_err(|_| "Assinatura inválida.")?;
    key.verify_strict(signed.payload.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| "Assinatura inválida.")?;
    let payload: Payload = serde_json::from_str(&signed.payload).map_err(|_| "Resposta inválida.")?;
    if payload.nonce != nonce { return Err("Resposta de outra consulta ou repetida.".into()); }
    Ok(payload.reply)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicConfig {
    pub agent_public_key: String,
    #[serde(default)] pub client_sid: Option<String>,
}

pub fn installed_public_key() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|_| "Localização do aplicativo inválida.")?
        .parent().ok_or("Localização do aplicativo inválida.")?.join("agent-public.json");
    let bytes = std::fs::read(path).map_err(|_| "Configuração pública do agente não encontrada junto ao executável.")?;
    if bytes.len() > 1024 { return Err("Configuração pública inválida.".into()); }
    let config: PublicConfig = serde_json::from_slice(&bytes).map_err(|_| "Configuração pública inválida.")?;
    validate_public_key(&config.agent_public_key)?;
    Ok(config.agent_public_key)
}

/// Framing com tamanho validado antes de alocar; conexão é encerrada após um RPC.
pub async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(stream: &mut W, bytes: &[u8], maximum: usize) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    if bytes.len() > maximum { return Err(std::io::Error::other("Frame excede o limite.")); }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(stream: &mut R, maximum: usize) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let length = stream.read_u32().await? as usize;
    if length > maximum { return Err(std::io::Error::other("Frame excede o limite.")); }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

#[cfg(windows)]
pub async fn call(public: &str, action: Action) -> Result<Reply, String> {
    let operation = async {
        let request = Request::new(action);
        let mut client = loop {
            match open_client() {
                Ok(client) => break client,
                Err(error) if error.raw_os_error() == Some(231) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
                Err(_) => return Err("Serviço indisponível ou acesso ao pipe negado.".into()),
            }
        };
        let bytes = serde_json::to_vec(&request).map_err(|_| "Consulta inválida.")?;
        write_frame(&mut client, &bytes, MAX_REQUEST).await.map_err(|_| "Falha ao consultar serviço.")?;
        let bytes = zeroize::Zeroizing::new(read_frame(&mut client, MAX_FRAME).await.map_err(|_| "Resposta inválida do serviço.")?);
        let reply = serde_json::from_slice(&bytes).map_err(|_| "Resposta inválida do serviço.")?;
        verify(public, &request.nonce, reply)
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), operation).await
        .map_err(|_| "O serviço não respondeu no prazo.".to_string())?
}

#[cfg(windows)]
fn open_client() -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING}};
    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
    // GENERIC_WRITE também concede FILE_CREATE_PIPE_INSTANCE. Solicitar somente
    // os direitos específicos concedidos ao cliente, sem esse bit (0x4).
    const CLIENT_ACCESS: u32 = 0x0012019b;
    const SECURITY_SQOS_PRESENT: u32 = 0x00100000;
    const SECURITY_IDENTIFICATION: u32 = 0x00010000;
    let handle = unsafe { CreateFileW(name.as_ptr(), CLIENT_ACCESS, 0, std::ptr::null(), OPEN_EXISTING,
        FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, std::ptr::null_mut()) };
    if handle == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error()); }
    // SAFETY: handle válido, exclusivo, criado com OVERLAPPED; ownership passa ao Tokio.
    unsafe { tokio::net::windows::named_pipe::NamedPipeClient::from_raw_handle(handle) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authentic_response_is_accepted() {
        let key = Identity::generate();
        let request = Request::new(Action::Status);
        request.validate().unwrap();
        assert!(matches!(verify(&key.public_hex(), &request.nonce, key.sign(&request.nonce, Reply::OrderCreated).unwrap()).unwrap(), Reply::OrderCreated));
    }
    #[test]
    fn tampered_response_and_wrong_key_are_rejected() {
        let key = Identity::generate();
        let request = Request::new(Action::Status);
        let mut signed = key.sign(&request.nonce, Reply::OrderCreated).unwrap();
        signed.payload.push(' ');
        assert!(verify(&key.public_hex(), &request.nonce, signed).is_err());
        let signed = key.sign(&request.nonce, Reply::OrderCreated).unwrap();
        assert!(verify(&Identity::generate().public_hex(), &request.nonce, signed).is_err());
    }
    #[test]
    fn previous_request_cannot_be_replayed() {
        let key = Identity::generate();
        let old = Request::new(Action::Status);
        let new = Request::new(Action::Status);
        assert!(verify(&key.public_hex(), &new.nonce, key.sign(&old.nonce, Reply::OrderCreated).unwrap()).is_err());
    }
    #[test]
    fn credit_operation_and_invalid_nonce_are_not_supported() {
        assert!(serde_json::from_str::<Request>(r#"{"nonce":"x","action":{"type":"grant","seconds":3600}}"#).is_err());
        assert!(Request { nonce: "x".into(), action: Action::Status }.validate().is_err());
    }
}
