use comunicacao_lan::{lease::Lease, protocol::{Plan, SessionView}};
use lan_transport::{Action, Identity, Reply, Request, SignedReply, MAX_FRAME};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub mod workstation;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateConfig {
    pub station_token: String,
    pub server_public_key: String,
    pub agent_signing_secret: String,
    pub client_sid: String,
    pub service_sid: String,
    #[serde(default)] pub workstation: Option<WorkstationConfig>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkstationConfig {
    pub local_user: String,
    /// DPAPI protegido por máquina, dentro de diretório privado por ACL.
    pub protected_password: String,
}
impl PrivateConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.station_token.len() < 32 || !self.station_token.is_ascii()
            || self.station_token.chars().any(char::is_whitespace) {
            return Err("Credencial da estação inválida.".into());
        }
        lan_transport::validate_public_key(&self.server_public_key)?;
        Identity::from_secret(&self.agent_signing_secret)?;
        if !valid_sid_text(&self.client_sid) { return Err("SID de cliente inválido.".into()); }
        if !self.service_sid.starts_with("S-1-5-80-") || !valid_sid_text(&self.service_sid) { return Err("SID de serviço inválido.".into()); }
        if let Some(workstation) = &self.workstation {
            if workstation.local_user.is_empty() || workstation.local_user.len() > 20
                || !workstation.local_user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                || workstation.protected_password.is_empty() || workstation.protected_password.len() > 8192 {
                return Err("Configuração de entrada Windows inválida.".into());
            }
        }
        Ok(())
    }
}

pub fn valid_sid_text(sid: &str) -> bool {
    let parts: Vec<_> = sid.split('-').collect();
    sid.len() <= 184 && parts.len() >= 4 && parts[0] == "S" && parts[1] == "1"
        && parts[2..].iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && part.parse::<u64>().is_ok())
}

pub struct AgentState {
    started: Instant,
    lease: Lease,
    session: SessionView,
    catalog: Vec<Plan>,
    last_online: Option<Instant>,
    message: Option<String>,
    pub workstation: bool,
    pub instance: String,
}
impl Default for AgentState {
    fn default() -> Self {
        Self {
            started: Instant::now(), lease: Lease::default(), catalog: Vec::new(),
            session: SessionView { machine_id: "PC-01".into(), remaining_seconds: 0, revision: 0, pending_order: None },
            last_online: None, message: Some("Aguardando autorização assinada do servidor.".into()),
            workstation: false, instance: uuid::Uuid::new_v4().to_string(),
        }
    }
}
impl AgentState {
    fn online(&self) -> bool { self.last_online.is_some_and(|instant| instant.elapsed() < Duration::from_secs(5)) }
    pub fn status(&self) -> Reply {
        let mut session = self.session.clone();
        session.remaining_seconds = self.lease.remaining(self.started.elapsed()).as_secs();
        Reply::Status { session, online: self.online(), message: self.message.clone(), workstation: self.workstation }
    }
    pub fn authorized(&self) -> bool { !self.lease.remaining(self.started.elapsed()).is_zero() }
    pub fn watchdog_status(&self) -> Reply {
        let mut session = self.session.clone();
        session.remaining_seconds = self.lease.remaining(self.started.elapsed()).as_secs();
        Reply::Watchdog { session, instance: self.instance.clone() }
    }
    fn apply(&mut self, session: SessionView, latency: Duration) -> Result<(), String> {
        if session.machine_id != "PC-01" { return Err("Autorização para outra máquina rejeitada.".into()); }
        // Não permitir que uma resposta antiga substitua metadados atuais.
        if session.revision < self.session.revision { return Err("Revisão antiga rejeitada.".into()); }
        self.lease.apply(&session, self.started.elapsed(), latency);
        self.session = session;
        self.last_online = Some(Instant::now());
        self.message = None;
        Ok(())
    }
    fn disconnected(&mut self) {
        self.last_online = None;
        self.message = Some("Sem autorização nova do servidor. O prazo já recebido continua contando.".into());
    }
}

pub struct Backend {
    client: reqwest::Client,
    config: PrivateConfig,
}
impl Backend {
    pub fn new(config: PrivateConfig) -> Result<Self, String> {
        config.validate()?;
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(2)).redirect(reqwest::redirect::Policy::none())
            .no_proxy().build().map_err(|_| "Não foi possível iniciar o cliente HTTP.")?;
        Ok(Self { client, config })
    }
    pub async fn call(&self, action: Action) -> Result<Reply, String> {
        let request = Request::new(action);
        let mut response = self.client.post("http://127.0.0.1:8787/api/agent/rpc")
            .bearer_auth(&self.config.station_token).json(&request).send().await
            .map_err(|_| "Servidor indisponível.")?.error_for_status().map_err(|_| "Servidor recusou a operação.")?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "Resposta incompleta.")? {
            if bytes.len().saturating_add(chunk.len()) > MAX_FRAME { return Err("Resposta excede o limite.".into()); }
            bytes.extend_from_slice(&chunk);
        }
        let signed: SignedReply = serde_json::from_slice(&bytes).map_err(|_| "Resposta não assinada ou inválida.")?;
        lan_transport::verify(&self.config.server_public_key, &request.nonce, signed)
    }
}

pub type Shared = Arc<Mutex<AgentState>>;

pub async fn poll(backend: Arc<Backend>, state: Shared) {
    loop {
        let requested = Instant::now();
        match backend.call(Action::Status).await {
            Ok(Reply::Status { session, .. }) => {
                let mut state = state.lock().await;
                if state.apply(session, requested.elapsed()).is_err() { state.disconnected(); }
            }
            _ => state.lock().await.disconnected(),
        }
        let needs_catalog = state.lock().await.catalog.is_empty();
        if needs_catalog {
            if let Ok(Reply::Catalog { plans }) = backend.call(Action::Catalog).await {
                state.lock().await.catalog = plans;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn handle(action: Action, backend: &Backend, state: &Shared) -> Reply {
    match action {
        Action::Status => state.lock().await.status(),
        Action::Catalog => Reply::Catalog { plans: state.lock().await.catalog.clone() },
        Action::Buy { plan_id } => {
            if !state.lock().await.online() {
                return Reply::Failure { message: "Sem conexão recente para criar uma cobrança.".into() };
            }
            match backend.call(Action::Buy { plan_id }).await {
                Ok(Reply::OrderCreated) => Reply::OrderCreated,
                _ => Reply::Failure { message: "Não foi possível criar a cobrança. Aguarde a atualização do estado.".into() },
            }
        }
        Action::SerializeLogon { .. } | Action::Watchdog => Reply::Failure { message: "Operação reservada ao Windows.".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sid_cannot_inject_an_access_control_entry() {
        assert!(valid_sid_text("S-1-5-21-100-200-300-1001"));
        assert!(!valid_sid_text("S-1-5-21-1)(A;;GA;;;WD)"));
        assert!(!valid_sid_text("Everyone"));
    }
    #[test]
    fn daemon_starts_without_credit_and_rejects_other_station() {
        let mut state = AgentState::default();
        if let Reply::Status { session, online, .. } = state.status() {
            assert_eq!(session.remaining_seconds, 0);
            assert!(!online);
        } else { panic!("status esperado"); }
        let session = SessionView { machine_id: "PC-02".into(), remaining_seconds: 3600, revision: 1, pending_order: None };
        assert!(state.apply(session, Duration::ZERO).is_err());
    }
    #[test]
    fn service_keeps_deadline_after_ui_disconnect_and_rejects_old_revision() {
        let mut state = AgentState::default();
        let mut session = SessionView { machine_id: "PC-01".into(), remaining_seconds: 90, revision: 2, pending_order: None };
        state.apply(session.clone(), Duration::ZERO).unwrap();
        state.disconnected();
        session.revision = 1;
        session.remaining_seconds = 3600;
        assert!(state.apply(session, Duration::ZERO).is_err());
        if let Reply::Status { session, online, .. } = state.status() {
            assert!(session.remaining_seconds > 0 && session.remaining_seconds <= 90);
            assert!(!online);
        } else { panic!("status esperado"); }
    }
}
