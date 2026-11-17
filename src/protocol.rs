//! Contrato do protótipo local. Não representa um comprovante de Pix.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plan {
    pub id: String,
    pub label: String,
    pub amount_cents: i64,
    pub duration_seconds: i64,
}

pub fn demo_catalog() -> Vec<Plan> {
    [("demo", "Teste de 90 segundos", 100, 90),
     ("1h", "1 hora", 1000, 3600),
     ("2h", "2 horas", 1800, 7200)]
        .into_iter()
        .map(|(id, label, amount_cents, duration_seconds)| Plan {
            id: id.into(), label: label.into(), amount_cents, duration_seconds,
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOrder {
    pub plan_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrderView {
    pub id: String,
    pub amount_cents: i64,
    pub duration_seconds: i64,
    pub status: String,
    pub expires_at: i64,
    pub qr_payload: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionView {
    pub machine_id: String,
    pub remaining_seconds: u64,
    pub revision: u64,
    pub pending_order: Option<OrderView>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Overview {
    pub session: SessionView,
    pub orders: Vec<OrderView>,
}
