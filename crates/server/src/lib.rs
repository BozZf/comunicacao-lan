pub mod store;

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post}, Json, Router,
};
use comunicacao_lan::protocol::{demo_catalog, CreateOrder, OrderView, Overview, Plan, SessionView};
use std::{sync::{Arc, Mutex}, time::{Instant, SystemTime, UNIX_EPOCH}};
use store::{Store, StoreError};
use subtle::ConstantTimeEq;
use lan_transport::{Action, Identity, Reply, Request as RpcRequest, SignedReply};

pub const ADDRESS: &str = "127.0.0.1:8787";
const ORIGIN: &str = "http://127.0.0.1:8787";

#[derive(Clone)]
pub struct Config {
    admin_token: String,
    station_token: String,
    demo_payments: bool,
    agent_signer: Option<Identity>,
}
impl Config {
    pub fn new(admin_token: String, station_token: String, demo_payments: bool) -> Result<Self, &'static str> {
        if admin_token.len() < 32 || station_token.len() < 32
            || !admin_token.is_ascii() || !station_token.is_ascii()
            || admin_token.chars().any(char::is_whitespace)
            || station_token.chars().any(char::is_whitespace)
            || admin_token == station_token {
            return Err("Use dois tokens distintos, sem espaços, com pelo menos 32 caracteres ASCII.");
        }
        Ok(Self { admin_token, station_token, demo_payments, agent_signer: None })
    }
    pub fn with_agent_signer(mut self, secret: &str) -> Result<Self, String> {
        self.agent_signer = Some(Identity::from_secret(secret)?);
        Ok(self)
    }
}

struct Clock { started: Instant, epoch: i64 }
impl Clock {
    fn new() -> Self {
        Self { started: Instant::now(), epoch: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) as i64 }
    }
    fn now(&self) -> i64 { self.epoch.saturating_add(self.started.elapsed().as_secs().min(i64::MAX as u64) as i64) }
}
struct AppState { config: Config, store: Mutex<Store>, clock: Clock }
type Shared = Arc<AppState>;

pub fn app(store: Store, config: Config) -> Router {
    Router::new()
        .route("/", get(panel))
        .route("/panel.js", get(panel_js))
        .route("/style.css", get(style))
        .route("/api/catalog", get(catalog))
        .route("/api/session", get(session))
        .route("/api/orders", post(create_order))
        .route("/api/agent/rpc", post(agent_rpc))
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/orders/{id}/confirm-demo", post(confirm_demo))
        .layer(DefaultBodyLimit::max(1024))
        .layer(axum::middleware::map_response(secure_headers))
        .with_state(Arc::new(AppState { config, store: Mutex::new(store), clock: Clock::new() }))
}

async fn secure_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response { (self.0, self.1).into_response() }
}
impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::InvalidPlan => Self(StatusCode::BAD_REQUEST, "Pacote inválido."),
            StoreError::PendingOrder => Self(StatusCode::CONFLICT, "Já existe uma cobrança pendente. Aguarde o pagamento ou a expiração em 10 minutos."),
            StoreError::NotFound => Self(StatusCode::NOT_FOUND, "Cobrança não encontrada."),
            StoreError::Expired => Self(StatusCode::CONFLICT, "Cobrança expirada."),
            StoreError::ClockRollback => Self(StatusCode::SERVICE_UNAVAILABLE, "Relógio do servidor regrediu. Verifique a configuração antes de continuar."),
            StoreError::Database(error) => {
                eprintln!("Falha SQLite: {error}");
                Self(StatusCode::INTERNAL_SERVER_ERROR, "Falha ao persistir a operação.")
            }
            StoreError::Overflow => Self(StatusCode::BAD_REQUEST, "Limite de tempo excedido."),
        }
    }
}
type ApiResult<T> = Result<T, ApiError>;

fn authorize(headers: &HeaderMap, expected: &str) -> ApiResult<()> {
    if headers.get(header::HOST).and_then(|h| h.to_str().ok()) != Some(ADDRESS) {
        return Err(ApiError(StatusCode::FORBIDDEN, "Host inválido. Use 127.0.0.1:8787."));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if origin.to_str().ok() != Some(ORIGIN) {
            return Err(ApiError(StatusCode::FORBIDDEN, "Origem inválida."));
        }
    }
    let token = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("");
    if !bool::from(token.as_bytes().ct_eq(expected.as_bytes())) {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Credencial inválida."));
    }
    Ok(())
}

fn with_store<T>(state: &Shared, operation: impl FnOnce(&mut Store, i64) -> Result<T, StoreError>) -> ApiResult<T> {
    let mut store = state.store.lock().map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Armazenamento indisponível."))?;
    Ok(operation(&mut store, state.clock.now())?)
}

async fn catalog(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Vec<Plan>>> {
    authorize(&headers, &state.config.station_token)?;
    Ok(Json(demo_catalog()))
}
async fn session(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Json<SessionView>> {
    authorize(&headers, &state.config.station_token)?;
    Ok(Json(with_store(&state, |store, now| store.session(now))?))
}
async fn create_order(State(state): State<Shared>, headers: HeaderMap, Json(input): Json<CreateOrder>) -> ApiResult<Json<OrderView>> {
    authorize(&headers, &state.config.station_token)?;
    Ok(Json(with_store(&state, |store, now| store.create_order(&input.plan_id, now))?))
}
async fn overview(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Overview>> {
    authorize(&headers, &state.config.admin_token)?;
    Ok(Json(with_store(&state, |store, now| store.overview(now))?))
}
async fn confirm_demo(State(state): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<StatusCode> {
    authorize(&headers, &state.config.admin_token)?;
    if !state.config.demo_payments {
        return Err(ApiError(StatusCode::FORBIDDEN, "Confirmação simulada desativada."));
    }
    with_store(&state, |store, now| store.confirm_demo(&id, now))?;
    Ok(StatusCode::NO_CONTENT)
}
async fn agent_rpc(State(state): State<Shared>, headers: HeaderMap, Json(request): Json<RpcRequest>) -> ApiResult<Json<SignedReply>> {
    authorize(&headers, &state.config.station_token)?;
    let signer = state.config.agent_signer.as_ref().ok_or(ApiError(StatusCode::SERVICE_UNAVAILABLE, "Assinatura do servidor não configurada."))?;
    request.validate().map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Consulta inválida."))?;
    let reply = match request.action {
        Action::Status => Reply::Status { session: with_store(&state, |store, now| store.session(now))?, online: true, message: None },
        Action::Catalog => Reply::Catalog { plans: demo_catalog() },
        Action::Buy { plan_id } => {
            with_store(&state, |store, now| store.create_order(&plan_id, now))?;
            Reply::OrderCreated
        }
    };
    Ok(Json(signer.sign(&request.nonce, reply).map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Falha ao assinar resposta."))?))
}
async fn panel() -> Html<&'static str> { Html(include_str!("../web/index.html")) }
async fn panel_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../web/panel.js"))
}
async fn style() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../web/style.css"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::{to_bytes, Body}, http::Request};
    use tower::ServiceExt;
    const ADMIN: &str = "admin-test-token-at-least-32-characters";
    const STATION: &str = "station-test-token-at-least-32-characters";
    fn router(demo: bool) -> Router {
        app(Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap(),
            Config::new(ADMIN.into(), STATION.into(), demo).unwrap())
    }
    fn request(method: &str, path: &str, token: &str, body: &str) -> Request<Body> {
        Request::builder().method(method).uri(path).header("host", ADDRESS)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json").body(Body::from(body.to_owned())).unwrap()
    }
    #[tokio::test]
    async fn credentials_are_separated_and_untrusted_origins_rejected() {
        let app = router(true);
        assert_eq!(app.clone().oneshot(request("GET", "/api/session", "", "")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(request("GET", "/api/admin/overview", STATION, "")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(app.clone().oneshot(request("GET", "/api/catalog", ADMIN, "")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let mut cross_origin = request("GET", "/api/session", STATION, "");
        cross_origin.headers_mut().insert(header::ORIGIN, HeaderValue::from_static("https://example.com"));
        assert_eq!(app.clone().oneshot(cross_origin).await.unwrap().status(), StatusCode::FORBIDDEN);
        let mut rebinding = request("GET", "/api/session", STATION, "");
        rebinding.headers_mut().insert(header::HOST, HeaderValue::from_static("example.com:8787"));
        assert_eq!(app.oneshot(rebinding).await.unwrap().status(), StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    async fn full_purchase_flow_and_duplicate_confirmation() {
        let app = router(true);
        let response = app.clone().oneshot(request("POST", "/api/orders", STATION, r#"{"plan_id":"demo"}"#)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let order: OrderView = serde_json::from_slice(&bytes).unwrap();
        let path = format!("/api/admin/orders/{}/confirm-demo", order.id);
        assert_eq!(app.clone().oneshot(request("POST", &path, STATION, "")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        for _ in 0..2 {
            assert_eq!(app.clone().oneshot(request("POST", &path, ADMIN, "")).await.unwrap().status(), StatusCode::NO_CONTENT);
        }
        let response = app.oneshot(request("GET", "/api/session", STATION, "")).await.unwrap();
        let session: SessionView = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(session.revision, 1);
        assert!(session.remaining_seconds <= 90 && session.remaining_seconds > 0);
        assert!(session.pending_order.is_none());
    }
    #[tokio::test]
    async fn simulation_requires_explicit_enable_and_price_cannot_be_submitted() {
        let app = router(false);
        let response = app.clone().oneshot(request("POST", "/api/orders", STATION, r#"{"plan_id":"demo","amount_cents":1}"#)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let response = app.oneshot(request("POST", "/api/admin/orders/fake/confirm-demo", ADMIN, "")).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    #[tokio::test]
    async fn agent_rpc_requires_signer_and_returns_a_nonce_bound_signature() {
        let input = RpcRequest::new(Action::Status);
        let body = serde_json::to_string(&input).unwrap();
        let response = router(true).oneshot(request("POST", "/api/agent/rpc", STATION, &body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let identity = Identity::generate();
        let config = Config::new(ADMIN.into(), STATION.into(), true).unwrap()
            .with_agent_signer(&identity.secret_hex()).unwrap();
        let app = app(Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap(), config);
        let response = app.oneshot(request("POST", "/api/agent/rpc", STATION, &body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let signed: SignedReply = serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        let reply = lan_transport::verify(&identity.public_hex(), &input.nonce, signed).unwrap();
        assert!(matches!(reply, Reply::Status { session: SessionView { remaining_seconds: 0, .. }, .. }));
    }
}
