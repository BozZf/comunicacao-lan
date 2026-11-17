use comunicacao_lan::protocol::{demo_catalog, OrderView, Overview, SessionView};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    InvalidPlan,
    PendingOrder,
    NotFound,
    Expired,
    ClockRollback,
    Overflow,
}
impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self { Self::Database(error) }
}
type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn from_connection(connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS station (
               id INTEGER PRIMARY KEY CHECK(id=1), deadline INTEGER NOT NULL,
               revision INTEGER NOT NULL, last_seen INTEGER NOT NULL);
             INSERT OR IGNORE INTO station VALUES(1,0,0,0);
             CREATE TABLE IF NOT EXISTS orders (
               id TEXT PRIMARY KEY, plan_id TEXT NOT NULL, amount INTEGER NOT NULL,
               duration INTEGER NOT NULL, status TEXT NOT NULL CHECK(status IN ('pending','paid','expired')),
               expires_at INTEGER NOT NULL);
             CREATE UNIQUE INDEX IF NOT EXISTS one_pending ON orders(status) WHERE status='pending';
             CREATE TABLE IF NOT EXISTS audit (
               id INTEGER PRIMARY KEY, charge_id TEXT NOT NULL UNIQUE REFERENCES orders(id),
               confirmed_at INTEGER NOT NULL, deadline INTEGER NOT NULL);"
        )?;
        Ok(Self { connection })
    }

    fn advance(connection: &Connection, now: i64) -> Result<()> {
        let last: i64 = connection.query_row("SELECT last_seen FROM station WHERE id=1", [], |r| r.get(0))?;
        if now < last { return Err(StoreError::ClockRollback); }
        connection.execute("UPDATE station SET last_seen=?1 WHERE id=1", [now])?;
        connection.execute("UPDATE orders SET status='expired' WHERE status='pending' AND expires_at<=?1", [now])?;
        Ok(())
    }

    pub fn create_order(&mut self, plan_id: &str, now: i64) -> Result<OrderView> {
        let plan = demo_catalog().into_iter().find(|p| p.id == plan_id).ok_or(StoreError::InvalidPlan)?;
        let expires_at = now.checked_add(600).ok_or(StoreError::Overflow)?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::advance(&tx, now)?;
        let pending: Option<(String, String)> = tx.query_row(
            "SELECT id,plan_id FROM orders WHERE status='pending'", [], |r| Ok((r.get(0)?, r.get(1)?))
        ).optional()?;
        if let Some((id, existing_plan)) = pending {
            if existing_plan != plan_id { return Err(StoreError::PendingOrder); }
            let order = read_order(&tx, &id)?.ok_or(StoreError::NotFound)?;
            tx.commit()?;
            return Ok(order);
        }
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO orders VALUES(?1,?2,?3,?4,'pending',?5)",
            params![id, plan.id, plan.amount_cents, plan.duration_seconds, expires_at])?;
        let order = read_order(&tx, &id)?.ok_or(StoreError::NotFound)?;
        tx.commit()?;
        Ok(order)
    }

    /// Apenas adaptador de simulação autorizado. Em produção será substituído
    /// por confirmação do provedor, conferindo recebedor, moeda, valor e cobrança.
    pub fn confirm_demo(&mut self, id: &str, now: i64) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::advance(&tx, now)?;
        let order = read_order(&tx, id)?.ok_or(StoreError::NotFound)?;
        if order.status == "paid" {
            tx.commit()?;
            return Ok(());
        }
        if order.status != "pending" || now >= order.expires_at {
            return Err(StoreError::Expired);
        }
        let (previous, revision): (i64, i64) = tx.query_row(
            "SELECT deadline,revision FROM station WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?))
        )?;
        let deadline = previous.max(now).checked_add(order.duration_seconds).ok_or(StoreError::Overflow)?;
        let revision = revision.checked_add(1).ok_or(StoreError::Overflow)?;
        tx.execute("UPDATE orders SET status='paid' WHERE id=?1", [id])?;
        tx.execute("UPDATE station SET deadline=?1,revision=?2 WHERE id=1", params![deadline, revision])?;
        tx.execute("INSERT INTO audit(charge_id,confirmed_at,deadline) VALUES(?1,?2,?3)", params![id, now, deadline])?;
        tx.commit()?;
        Ok(())
    }

    pub fn session(&mut self, now: i64) -> Result<SessionView> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::advance(&tx, now)?;
        let (deadline, revision): (i64, i64) = tx.query_row(
            "SELECT deadline,revision FROM station WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?))
        )?;
        let pending_id: Option<String> = tx.query_row("SELECT id FROM orders WHERE status='pending'", [], |r| r.get(0)).optional()?;
        let pending_order = pending_id.map(|id| read_order(&tx, &id)).transpose()?.flatten();
        tx.commit()?;
        Ok(SessionView {
            machine_id: "PC-01".into(), remaining_seconds: deadline.saturating_sub(now).max(0) as u64,
            revision: revision as u64, pending_order,
        })
    }

    pub fn overview(&mut self, now: i64) -> Result<Overview> {
        let session = self.session(now)?;
        let mut statement = self.connection.prepare("SELECT id,amount,duration,status,expires_at FROM orders ORDER BY rowid DESC LIMIT 50")?;
        let orders = statement.query_map([], order_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Overview { session, orders })
    }
}

fn order_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrderView> {
    let id: String = row.get(0)?;
    use crate::payments::PaymentGateway;
    let display = crate::payments::SimulatedPix.display(&id);
    Ok(OrderView {
        qr_payload: display.payload, payment_provider: display.provider, payment_notice: display.notice,
        id, amount_cents: row.get(1)?,
        duration_seconds: row.get(2)?, status: row.get(3)?, expires_at: row.get(4)?,
    })
}
fn read_order(connection: &Connection, id: &str) -> Result<Option<OrderView>> {
    Ok(connection.query_row("SELECT id,amount,duration,status,expires_at FROM orders WHERE id=?1", [id], order_row).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_and_duplicate_confirmation_preserve_credit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lan.db");
        let id;
        {
            let mut store = Store::open(&path).unwrap();
            id = store.create_order("demo", 100).unwrap().id;
            assert_eq!(store.session(100).unwrap().remaining_seconds, 0);
            store.confirm_demo(&id, 100).unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        store.confirm_demo(&id, 110).unwrap();
        let session = store.session(110).unwrap();
        assert_eq!((session.remaining_seconds, session.revision), (80, 1));
        assert_eq!(store.session(190).unwrap().remaining_seconds, 0);
    }
    #[test]
    fn renewal_is_added_to_existing_deadline() {
        let mut store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let a = store.create_order("demo", 100).unwrap();
        store.confirm_demo(&a.id, 100).unwrap();
        let b = store.create_order("demo", 110).unwrap();
        store.confirm_demo(&b.id, 110).unwrap();
        let session = store.session(110).unwrap();
        assert_eq!((session.remaining_seconds, session.revision), (170, 2));
    }
    #[test]
    fn rejects_expired_orders_invalid_plan_and_clock_rollback() {
        let mut store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        assert!(matches!(store.create_order("fake", 100), Err(StoreError::InvalidPlan)));
        let order = store.create_order("demo", 100).unwrap();
        assert!(matches!(store.confirm_demo(&order.id, 700), Err(StoreError::Expired)));
        assert_eq!(store.session(700).unwrap().remaining_seconds, 0);
        assert!(matches!(store.session(699), Err(StoreError::ClockRollback)));
    }
    #[test]
    fn duplicate_order_request_reuses_charge_and_rejects_conflicting_plan() {
        let mut store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let order = store.create_order("demo", 100).unwrap();
        assert_eq!(store.create_order("demo", 101).unwrap().id, order.id);
        assert!(matches!(store.create_order("1h", 102), Err(StoreError::PendingOrder)));
    }
    #[test]
    fn failed_audit_rolls_back_payment_and_session_together() {
        let mut store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let order = store.create_order("demo", 100).unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(store.confirm_demo(&order.id, 100).is_err());
        let session = store.session(100).unwrap();
        assert_eq!(session.remaining_seconds, 0);
        assert_eq!(session.pending_order.unwrap().status, "pending");
    }
}
