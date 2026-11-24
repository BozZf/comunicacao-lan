//! Regras de domínio. Não verifica Pix, não persiste dados e não bloqueia Windows.
//! `confirm_payment` exige um comprovante [`VerifiedPayment`] emitido exclusivamente
//! por [`PaymentAuthority`] (detido pelo futuro adaptador autenticado do servidor) e
//! um instante [`TrustedNow`] proveniente de relógio confiável.

use std::collections::HashMap;

/// Comprimento máximo aceito (em bytes) para `machine_id` e `charge_id`.
pub const MAX_IDENTIFIER_LEN: usize = 128;

/// Limite de ordens pendentes (ainda não creditadas) simultâneas por estação.
pub const MAX_PENDING_ORDERS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidIdentifier,
    InvalidOrder,
    DuplicateOrder,
    TooManyOrders,
    OrderExpired,
    UnknownOrder,
    UnknownStation,
    DuplicateStation,
    PaymentMismatch,
    ClockWentBackwards,
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Locked,
    Active { remaining_seconds: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Unlocked,
    Renewed,
    Warning { remaining_seconds: u64 },
    Locked,
}

/// Instante em segundos vindo de um relógio monotônico/confiável (nunca do navegador).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TrustedNow(u64);

impl TrustedNow {
    pub const fn from_monotonic_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

/// Capacidade selada retida exclusivamente pela camada autenticada do servidor (adaptador Pix).
/// Camadas não privilegiadas (como a interface da estação) nunca devem receber esta instância.
#[derive(Debug)]
pub struct PaymentAuthority {
    _sealed: (),
}

impl PaymentAuthority {
    /// Instancia a autoridade de confirmação do servidor.
    /// Chame apenas na inicialização do adaptador Pix autenticado (ou em testes/simulação).
    pub const fn new_server_authority() -> Self {
        Self { _sealed: () }
    }

    /// Emite um comprovante [`VerifiedPayment`] somente após validar o pagamento junto ao provedor.
    pub fn verify(
        &self,
        charge_id: impl Into<String>,
        machine_id: impl Into<String>,
        amount_cents: u64,
    ) -> Result<VerifiedPayment, Error> {
        let charge_id = charge_id.into();
        let machine_id = machine_id.into();
        validate_identifier(&charge_id)?;
        validate_identifier(&machine_id)?;
        if amount_cents == 0 {
            return Err(Error::InvalidOrder);
        }
        Ok(VerifiedPayment {
            charge_id,
            machine_id,
            amount_cents,
        })
    }
}

/// Comprovante de pagamento validado pelo provedor.
/// Campos privados impedem construção arbitrária fora de [`PaymentAuthority::verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPayment {
    charge_id: String,
    machine_id: String,
    amount_cents: u64,
}

impl VerifiedPayment {
    pub fn charge_id(&self) -> &str {
        &self.charge_id
    }

    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub fn amount_cents(&self) -> u64 {
        self.amount_cents
    }
}

#[derive(Debug, Clone)]
struct Order {
    amount_cents: u64,
    duration_seconds: u64,
    credited: bool,
    expires_at: Option<u64>,
}

fn validate_identifier(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_LEN
        || value != value.trim()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Error::InvalidIdentifier);
    }
    Ok(())
}

/// Uma instância pertence a uma máquina. Tempos são segundos fornecidos por
/// um relógio confiável ([`TrustedNow`]), nunca pelo navegador. Armazenamento é apenas em memória.
pub struct Station {
    machine_id: String,
    orders: HashMap<String, Order>,
    deadline: Option<u64>,
    last_now: Option<u64>,
    warned: [bool; 3],
}

impl Station {
    pub fn new(machine_id: impl Into<String>) -> Result<Self, Error> {
        let machine_id = machine_id.into();
        validate_identifier(&machine_id)?;
        Ok(Self {
            machine_id,
            orders: HashMap::new(),
            deadline: None,
            last_now: None,
            warned: [false; 3],
        })
    }

    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    /// O servidor obtém preço e duração do próprio catálogo.
    /// `charge_id` deve ser o identificador único da cobrança no provedor.
    pub fn create_order(
        &mut self,
        charge_id: impl Into<String>,
        amount_cents: u64,
        duration_seconds: u64,
    ) -> Result<(), Error> {
        self.insert_order(charge_id.into(), amount_cents, duration_seconds, None)
    }

    /// Cria uma cobrança com prazo de expiração (`expires_at`) caso não seja paga.
    pub fn create_order_expiring(
        &mut self,
        charge_id: impl Into<String>,
        amount_cents: u64,
        duration_seconds: u64,
        expires_at: TrustedNow,
    ) -> Result<(), Error> {
        let exp = expires_at.as_secs();
        if let Some(last) = self.last_now {
            self.prune_expired_orders(TrustedNow::from_monotonic_seconds(last));
            if exp <= last {
                return Err(Error::OrderExpired);
            }
        }
        self.insert_order(charge_id.into(), amount_cents, duration_seconds, Some(exp))
    }

    fn insert_order(
        &mut self,
        charge_id: String,
        amount_cents: u64,
        duration_seconds: u64,
        expires_at: Option<u64>,
    ) -> Result<(), Error> {
        validate_identifier(&charge_id)?;
        if amount_cents == 0 || duration_seconds == 0 {
            return Err(Error::InvalidOrder);
        }
        if self.orders.contains_key(&charge_id) {
            return Err(Error::DuplicateOrder);
        }
        let pending_count = self.orders.values().filter(|order| !order.credited).count();
        if pending_count >= MAX_PENDING_ORDERS {
            return Err(Error::TooManyOrders);
        }
        self.orders.insert(
            charge_id,
            Order {
                amount_cents,
                duration_seconds,
                credited: false,
                expires_at,
            },
        );
        Ok(())
    }

    /// Remove cobranças pendentes (não pagas) cujo prazo de validade já expirou.
    pub fn prune_expired_orders(&mut self, now: TrustedNow) -> usize {
        let now_secs = now.as_secs();
        let before = self.orders.len();
        self.orders
            .retain(|_, order| order.credited || order.expires_at.is_none_or(|exp| now_secs < exp));
        before - self.orders.len()
    }

    /// Recebe somente um pagamento já validado por [`PaymentAuthority`].
    /// Repetir uma confirmação retorna nenhum evento e não adiciona tempo.
    pub fn confirm_payment(
        &mut self,
        payment: &VerifiedPayment,
        now: TrustedNow,
    ) -> Result<Vec<Event>, Error> {
        let now_secs = now.as_secs();
        let order = self
            .orders
            .get(payment.charge_id())
            .ok_or(Error::UnknownOrder)?;
        if payment.machine_id() != self.machine_id || payment.amount_cents() != order.amount_cents {
            return Err(Error::PaymentMismatch);
        }
        self.check_time(now_secs)?;
        if order.credited {
            self.last_now = Some(now_secs);
            return Ok(Vec::new());
        }
        if order.expires_at.is_some_and(|exp| now_secs >= exp) {
            return Err(Error::OrderExpired);
        }
        let expired_unnotified = self.deadline.is_some_and(|deadline| deadline <= now_secs);
        let active = self.deadline.is_some_and(|deadline| deadline > now_secs);
        let base = self.deadline.unwrap_or(now_secs).max(now_secs);
        let deadline = base
            .checked_add(order.duration_seconds)
            .ok_or(Error::Overflow)?;
        // Todas as validações ocorrem antes de creditar.
        self.orders.get_mut(payment.charge_id()).unwrap().credited = true;
        self.deadline = Some(deadline);
        self.last_now = Some(now_secs);
        self.warned = [false; 3];
        if active {
            Ok(vec![Event::Renewed])
        } else if expired_unnotified {
            Ok(vec![Event::Locked, Event::Unlocked])
        } else {
            Ok(vec![Event::Unlocked])
        }
    }

    pub fn access(&mut self, now: TrustedNow) -> Result<Access, Error> {
        let now_secs = now.as_secs();
        self.check_time(now_secs)?;
        self.last_now = Some(now_secs);
        Ok(match self.deadline {
            Some(deadline) if deadline > now_secs => Access::Active {
                remaining_seconds: deadline - now_secs,
            },
            _ => Access::Locked,
        })
    }

    /// Chamado periodicamente pelo futuro serviço. Emite só o aviso mais urgente
    /// quando um atraso no agendamento atravessa vários limites.
    pub fn tick(&mut self, now: TrustedNow) -> Result<Vec<Event>, Error> {
        let access = self.access(now)?;
        self.prune_expired_orders(now);
        let remaining = match access {
            Access::Locked => {
                return Ok(if self.deadline.take().is_some() {
                    self.warned = [false; 3];
                    vec![Event::Locked]
                } else {
                    Vec::new()
                });
            }
            Access::Active { remaining_seconds } => remaining_seconds,
        };
        let thresholds = [600, 300, 60];
        let urgent = thresholds
            .iter()
            .rposition(|threshold| remaining <= *threshold);
        if let Some(index) = urgent {
            let emit = !self.warned[index];
            for flag in &mut self.warned[..=index] {
                *flag = true;
            }
            if emit {
                return Ok(vec![Event::Warning {
                    remaining_seconds: remaining,
                }]);
            }
        }
        Ok(Vec::new())
    }

    /// Revoga imediatamente uma sessão ativa ou expirada (ex.: manutenção ou estorno).
    pub fn revoke_session(&mut self, now: TrustedNow) -> Result<Vec<Event>, Error> {
        let now_secs = now.as_secs();
        self.check_time(now_secs)?;
        self.last_now = Some(now_secs);
        self.warned = [false; 3];
        Ok(if self.deadline.take().is_some() {
            vec![Event::Locked]
        } else {
            Vec::new()
        })
    }

    fn check_time(&self, now: u64) -> Result<(), Error> {
        if self.last_now.is_some_and(|last| now < last) {
            return Err(Error::ClockWentBackwards);
        }
        Ok(())
    }
}

/// Registro em memória para múltiplas estações que impõe unicidade global de `charge_id`
/// entre todas as máquinas. Em produção, deve ser respaldado por banco transacional
/// com restrição `UNIQUE(charge_id)` para preservar idempotência após reinícios.
#[derive(Default)]
pub struct LanRegistry {
    stations: HashMap<String, Station>,
    charge_owners: HashMap<String, String>,
}

impl LanRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_station(&mut self, machine_id: impl Into<String>) -> Result<(), Error> {
        let machine_id = machine_id.into();
        validate_identifier(&machine_id)?;
        if self.stations.contains_key(&machine_id) {
            return Err(Error::DuplicateStation);
        }
        let station = Station::new(machine_id.clone())?;
        self.stations.insert(machine_id, station);
        Ok(())
    }

    pub fn create_order(
        &mut self,
        machine_id: &str,
        charge_id: impl Into<String>,
        amount_cents: u64,
        duration_seconds: u64,
    ) -> Result<(), Error> {
        validate_identifier(machine_id)?;
        let charge_id = charge_id.into();
        validate_identifier(&charge_id)?;
        if self.charge_owners.contains_key(&charge_id) {
            return Err(Error::DuplicateOrder);
        }
        let station = self
            .stations
            .get_mut(machine_id)
            .ok_or(Error::UnknownStation)?;
        station.create_order(charge_id.clone(), amount_cents, duration_seconds)?;
        self.charge_owners
            .insert(charge_id, machine_id.to_owned());
        Ok(())
    }

    pub fn confirm_payment(
        &mut self,
        payment: &VerifiedPayment,
        now: TrustedNow,
    ) -> Result<Vec<Event>, Error> {
        let owner = self
            .charge_owners
            .get(payment.charge_id())
            .ok_or(Error::UnknownOrder)?;
        if owner != payment.machine_id() {
            return Err(Error::PaymentMismatch);
        }
        let station = self
            .stations
            .get_mut(owner)
            .ok_or(Error::UnknownStation)?;
        station.confirm_payment(payment, now)
    }

    pub fn access(&mut self, machine_id: &str, now: TrustedNow) -> Result<Access, Error> {
        self.stations
            .get_mut(machine_id)
            .ok_or(Error::UnknownStation)?
            .access(now)
    }

    pub fn tick(&mut self, machine_id: &str, now: TrustedNow) -> Result<Vec<Event>, Error> {
        self.stations
            .get_mut(machine_id)
            .ok_or(Error::UnknownStation)?
            .tick(now)
    }

    pub fn revoke_session(
        &mut self,
        machine_id: &str,
        now: TrustedNow,
    ) -> Result<Vec<Event>, Error> {
        self.stations
            .get_mut(machine_id)
            .ok_or(Error::UnknownStation)?
            .revoke_session(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now(secs: u64) -> TrustedNow {
        TrustedNow::from_monotonic_seconds(secs)
    }

    fn authority() -> PaymentAuthority {
        PaymentAuthority::new_server_authority()
    }

    fn pay(charge_id: &str, machine_id: &str, amount_cents: u64) -> VerifiedPayment {
        authority()
            .verify(charge_id, machine_id, amount_cents)
            .unwrap()
    }

    fn station() -> Station {
        let mut s = Station::new("PC-01").unwrap();
        s.create_order("pix-1", 1000, 3600).unwrap();
        s
    }

    #[test]
    fn order_does_not_unlock_and_confirmation_is_idempotent() {
        let mut s = station();
        let p = pay("pix-1", "PC-01", 1000);
        assert_eq!(s.access(now(0)), Ok(Access::Locked));
        assert_eq!(s.confirm_payment(&p, now(0)), Ok(vec![Event::Unlocked]));
        assert_eq!(s.confirm_payment(&p, now(10)), Ok(vec![]));
        assert_eq!(
            s.access(now(10)),
            Ok(Access::Active {
                remaining_seconds: 3590
            })
        );
        // Confirmação idempotente avança last_now para manter monotonicidade estrita.
        assert_eq!(s.access(now(5)), Err(Error::ClockWentBackwards));
    }

    #[test]
    fn rejects_wrong_machine_amount_and_unknown_charge() {
        let mut s = station();
        assert_eq!(
            s.confirm_payment(&pay("pix-1", "PC-02", 1000), now(0)),
            Err(Error::PaymentMismatch)
        );
        assert_eq!(
            s.confirm_payment(&pay("pix-1", "PC-01", 1), now(0)),
            Err(Error::PaymentMismatch)
        );
        assert_eq!(
            s.confirm_payment(&pay("fake", "PC-01", 1000), now(0)),
            Err(Error::UnknownOrder)
        );
        assert_eq!(s.access(now(0)), Ok(Access::Locked));
    }

    #[test]
    fn warns_at_ten_five_and_one_minute_then_locks_once() {
        let mut s = station();
        s.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        assert_eq!(s.tick(now(2999)), Ok(vec![]));
        for (t, remaining) in [(3000, 600), (3300, 300), (3540, 60)] {
            assert_eq!(
                s.tick(now(t)),
                Ok(vec![Event::Warning {
                    remaining_seconds: remaining
                }])
            );
            assert_eq!(s.tick(now(t)), Ok(vec![]));
        }
        assert_eq!(s.tick(now(3600)), Ok(vec![Event::Locked]));
        assert_eq!(s.tick(now(3601)), Ok(vec![]));
    }

    #[test]
    fn delayed_tick_only_emits_most_urgent_warning() {
        let mut s = station();
        s.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        assert_eq!(
            s.tick(now(3550)),
            Ok(vec![Event::Warning {
                remaining_seconds: 50
            }])
        );
        assert_eq!(s.tick(now(3551)), Ok(vec![]));
    }

    #[test]
    fn renewal_adds_to_existing_deadline_and_rearms_warnings() {
        let mut s = station();
        s.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        s.tick(now(3540)).unwrap();
        s.create_order("pix-2", 1000, 3600).unwrap();
        assert_eq!(
            s.confirm_payment(&pay("pix-2", "PC-01", 1000), now(3550)),
            Ok(vec![Event::Renewed])
        );
        assert_eq!(
            s.access(now(3550)),
            Ok(Access::Active {
                remaining_seconds: 3650
            })
        );
        assert_eq!(
            s.tick(now(6600)),
            Ok(vec![Event::Warning {
                remaining_seconds: 600
            }])
        );
    }

    #[test]
    fn payment_after_expiration_emits_lock_if_tick_missed_and_starts_from_confirmation() {
        // Caso 1: tick não rodou antes da nova confirmação -> emite Locked antes de Unlocked.
        let mut s = station();
        s.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        s.create_order("pix-2", 1000, 3600).unwrap();
        assert_eq!(
            s.confirm_payment(&pay("pix-2", "PC-01", 1000), now(4000)),
            Ok(vec![Event::Locked, Event::Unlocked])
        );
        assert_eq!(
            s.access(now(4000)),
            Ok(Access::Active {
                remaining_seconds: 3600
            })
        );

        // Caso 2: tick já bloqueou a sessão anterior -> nova confirmação emite apenas Unlocked.
        let mut s2 = station();
        s2.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        assert_eq!(s2.tick(now(3600)), Ok(vec![Event::Locked]));
        s2.create_order("pix-2", 1000, 3600).unwrap();
        assert_eq!(
            s2.confirm_payment(&pay("pix-2", "PC-01", 1000), now(4000)),
            Ok(vec![Event::Unlocked])
        );
    }

    #[test]
    fn rejects_clock_rollback_and_overflow_without_crediting() {
        let mut s = station();
        let p = pay("pix-1", "PC-01", 1000);
        s.access(now(100)).unwrap();
        assert_eq!(s.confirm_payment(&p, now(99)), Err(Error::ClockWentBackwards));
        assert_eq!(
            s.confirm_payment(&p, now(u64::MAX)),
            Err(Error::Overflow)
        );
        assert_eq!(s.confirm_payment(&p, now(100)), Ok(vec![Event::Unlocked]));
    }

    #[test]
    fn rejects_invalid_and_duplicate_orders() {
        let mut s = station();
        assert_eq!(s.create_order("pix-1", 1000, 3600), Err(Error::DuplicateOrder));
        assert_eq!(s.create_order("zero", 0, 3600), Err(Error::InvalidOrder));
        assert_eq!(s.create_order("empty", 1000, 0), Err(Error::InvalidOrder));
    }

    #[test]
    fn rejects_untrimmed_control_and_oversized_identifiers() {
        let oversized = "a".repeat(MAX_IDENTIFIER_LEN + 1);
        for bad in ["", "   ", "PC-01 ", " PC-01", "PC 01", "PC-01\n", "PC-01\r", "PC\0", "\x1b[31mPC", &oversized] {
            assert_eq!(Station::new(bad).err(), Some(Error::InvalidIdentifier));
        }
        let mut s = station();
        assert_eq!(s.create_order("pix-1 ", 1000, 3600), Err(Error::InvalidIdentifier));
        assert_eq!(s.create_order("pix-2\nlog", 1000, 3600), Err(Error::InvalidIdentifier));
        assert_eq!(
            authority().verify("pix-1 ", "PC-01", 1000),
            Err(Error::InvalidIdentifier)
        );
    }

    #[test]
    fn enforces_pending_order_cap_and_order_expiration() {
        let mut s = Station::new("PC-01").unwrap();
        for i in 0..MAX_PENDING_ORDERS {
            s.create_order_expiring(format!("pix-{i}"), 1000, 3600, now(600))
                .unwrap();
        }
        assert_eq!(
            s.create_order("pix-overflow", 1000, 3600),
            Err(Error::TooManyOrders)
        );
        // Após expirar em t=600, pagamento é rejeitado e tick limpa pendentes.
        assert_eq!(
            s.confirm_payment(&pay("pix-0", "PC-01", 1000), now(600)),
            Err(Error::OrderExpired)
        );
        assert_eq!(s.tick(now(600)), Ok(vec![]));
        assert_eq!(s.create_order("pix-fresh", 1000, 3600), Ok(()));
    }

    #[test]
    fn registry_enforces_global_charge_id_uniqueness_across_stations() {
        let mut reg = LanRegistry::new();
        reg.register_station("PC-01").unwrap();
        reg.register_station("PC-02").unwrap();
        assert_eq!(reg.register_station("PC-01"), Err(Error::DuplicateStation));

        reg.create_order("PC-01", "pix-global-1", 1000, 3600).unwrap();
        // Mesmo charge_id em outra estação deve falhar com DuplicateOrder.
        assert_eq!(
            reg.create_order("PC-02", "pix-global-1", 1000, 3600),
            Err(Error::DuplicateOrder)
        );
        // Confirmação apontando para PC-02 deve falhar com PaymentMismatch.
        assert_eq!(
            reg.confirm_payment(&pay("pix-global-1", "PC-02", 1000), now(0)),
            Err(Error::PaymentMismatch)
        );
        // Confirmação válida para PC-01 libera apenas PC-01.
        assert_eq!(
            reg.confirm_payment(&pay("pix-global-1", "PC-01", 1000), now(0)),
            Ok(vec![Event::Unlocked])
        );
        assert_eq!(
            reg.access("PC-01", now(0)),
            Ok(Access::Active {
                remaining_seconds: 3600
            })
        );
        assert_eq!(reg.access("PC-02", now(0)), Ok(Access::Locked));
    }

    #[test]
    fn revoke_session_locks_active_station_immediately() {
        let mut reg = LanRegistry::new();
        reg.register_station("PC-01").unwrap();
        reg.create_order("PC-01", "pix-1", 1000, 3600).unwrap();
        reg.confirm_payment(&pay("pix-1", "PC-01", 1000), now(0))
            .unwrap();
        assert_eq!(reg.revoke_session("PC-01", now(120)), Ok(vec![Event::Locked]));
        assert_eq!(reg.access("PC-01", now(120)), Ok(Access::Locked));
        assert_eq!(reg.revoke_session("PC-01", now(121)), Ok(vec![]));
    }
}
