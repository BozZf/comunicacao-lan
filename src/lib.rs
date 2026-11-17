//! Regras de domínio. Não verifica Pix, não persiste dados e não bloqueia Windows.
//! `confirm_payment` é uma fronteira interna: somente o servidor pode chamá-la
//! depois de consultar o provedor autenticado e conferir o pagamento.

use std::collections::HashMap;

pub mod lease;
pub mod protocol;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidOrder,
    DuplicateOrder,
    UnknownOrder,
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

#[derive(Debug, Clone)]
struct Order {
    amount_cents: u64,
    duration_seconds: u64,
    credited: bool,
}

/// Uma instância pertence a uma máquina. Tempos são segundos fornecidos por
/// um relógio confiável, nunca pelo navegador. Armazenamento é apenas em memória.
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
        if machine_id.trim().is_empty() {
            return Err(Error::InvalidOrder);
        }
        Ok(Self {
            machine_id,
            orders: HashMap::new(),
            deadline: None,
            last_now: None,
            warned: [false; 3],
        })
    }

    /// O servidor obtém preço e duração do próprio catálogo.
    /// `charge_id` deve ser o identificador único da cobrança no provedor.
    pub fn create_order(
        &mut self,
        charge_id: impl Into<String>,
        amount_cents: u64,
        duration_seconds: u64,
    ) -> Result<(), Error> {
        let charge_id = charge_id.into();
        if charge_id.trim().is_empty() || amount_cents == 0 || duration_seconds == 0 {
            return Err(Error::InvalidOrder);
        }
        if self.orders.contains_key(&charge_id) {
            return Err(Error::DuplicateOrder);
        }
        self.orders.insert(charge_id, Order {
            amount_cents,
            duration_seconds,
            credited: false,
        });
        Ok(())
    }

    /// Recebe somente um pagamento já validado pelo adaptador do provedor.
    /// Repetir uma confirmação retorna nenhum evento e não adiciona tempo.
    pub fn confirm_payment(
        &mut self,
        charge_id: &str,
        machine_id: &str,
        amount_cents: u64,
        now: u64,
    ) -> Result<Vec<Event>, Error> {
        let order = self.orders.get(charge_id).ok_or(Error::UnknownOrder)?;
        if machine_id != self.machine_id || amount_cents != order.amount_cents {
            return Err(Error::PaymentMismatch);
        }
        self.check_time(now)?;
        if order.credited {
            return Ok(Vec::new());
        }
        let active = self.deadline.is_some_and(|deadline| deadline > now);
        let base = self.deadline.unwrap_or(now).max(now);
        let deadline = base.checked_add(order.duration_seconds).ok_or(Error::Overflow)?;
        // Todas as validações ocorrem antes de creditar.
        self.orders.get_mut(charge_id).unwrap().credited = true;
        self.deadline = Some(deadline);
        self.last_now = Some(now);
        self.warned = [false; 3];
        Ok(vec![if active { Event::Renewed } else { Event::Unlocked }])
    }

    pub fn access(&mut self, now: u64) -> Result<Access, Error> {
        self.check_time(now)?;
        self.last_now = Some(now);
        Ok(match self.deadline {
            Some(deadline) if deadline > now => Access::Active {
                remaining_seconds: deadline - now,
            },
            _ => Access::Locked,
        })
    }

    /// Chamado periodicamente pelo futuro serviço. Emite só o aviso mais urgente
    /// quando um atraso no agendamento atravessa vários limites.
    pub fn tick(&mut self, now: u64) -> Result<Vec<Event>, Error> {
        let access = self.access(now)?;
        let remaining = match access {
            Access::Locked => {
                return Ok(if self.deadline.take().is_some() {
                    vec![Event::Locked]
                } else {
                    Vec::new()
                });
            }
            Access::Active { remaining_seconds } => remaining_seconds,
        };
        let thresholds = [600, 300, 60];
        let urgent = thresholds.iter().rposition(|threshold| remaining <= *threshold);
        if let Some(index) = urgent {
            let emit = !self.warned[index];
            for flag in &mut self.warned[..=index] {
                *flag = true;
            }
            if emit {
                return Ok(vec![Event::Warning { remaining_seconds: remaining }]);
            }
        }
        Ok(Vec::new())
    }

    fn check_time(&self, now: u64) -> Result<(), Error> {
        if self.last_now.is_some_and(|last| now < last) {
            return Err(Error::ClockWentBackwards);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Station {
        let mut s = Station::new("PC-01").unwrap();
        s.create_order("pix-1", 1000, 3600).unwrap();
        s
    }

    #[test]
    fn order_does_not_unlock_and_confirmation_is_idempotent() {
        let mut s = station();
        assert_eq!(s.access(0), Ok(Access::Locked));
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1000, 0), Ok(vec![Event::Unlocked]));
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1000, 10), Ok(vec![]));
        assert_eq!(s.access(10), Ok(Access::Active { remaining_seconds: 3590 }));
    }

    #[test]
    fn rejects_wrong_machine_amount_and_unknown_charge() {
        let mut s = station();
        assert_eq!(s.confirm_payment("pix-1", "PC-02", 1000, 0), Err(Error::PaymentMismatch));
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1, 0), Err(Error::PaymentMismatch));
        assert_eq!(s.confirm_payment("fake", "PC-01", 1000, 0), Err(Error::UnknownOrder));
        assert_eq!(s.access(0), Ok(Access::Locked));
    }

    #[test]
    fn warns_at_ten_five_and_one_minute_then_locks_once() {
        let mut s = station();
        s.confirm_payment("pix-1", "PC-01", 1000, 0).unwrap();
        assert_eq!(s.tick(2999), Ok(vec![]));
        for (now, remaining) in [(3000, 600), (3300, 300), (3540, 60)] {
            assert_eq!(s.tick(now), Ok(vec![Event::Warning { remaining_seconds: remaining }]));
            assert_eq!(s.tick(now), Ok(vec![]));
        }
        assert_eq!(s.tick(3600), Ok(vec![Event::Locked]));
        assert_eq!(s.tick(3601), Ok(vec![]));
    }

    #[test]
    fn delayed_tick_only_emits_most_urgent_warning() {
        let mut s = station();
        s.confirm_payment("pix-1", "PC-01", 1000, 0).unwrap();
        assert_eq!(s.tick(3550), Ok(vec![Event::Warning { remaining_seconds: 50 }]));
        assert_eq!(s.tick(3551), Ok(vec![]));
    }

    #[test]
    fn renewal_adds_to_existing_deadline_and_rearms_warnings() {
        let mut s = station();
        s.confirm_payment("pix-1", "PC-01", 1000, 0).unwrap();
        s.tick(3540).unwrap();
        s.create_order("pix-2", 1000, 3600).unwrap();
        assert_eq!(s.confirm_payment("pix-2", "PC-01", 1000, 3550), Ok(vec![Event::Renewed]));
        assert_eq!(s.access(3550), Ok(Access::Active { remaining_seconds: 3650 }));
        assert_eq!(s.tick(6600), Ok(vec![Event::Warning { remaining_seconds: 600 }]));
    }

    #[test]
    fn payment_after_expiration_starts_from_confirmation() {
        let mut s = station();
        s.confirm_payment("pix-1", "PC-01", 1000, 0).unwrap();
        s.create_order("pix-2", 1000, 3600).unwrap();
        assert_eq!(s.confirm_payment("pix-2", "PC-01", 1000, 4000), Ok(vec![Event::Unlocked]));
        assert_eq!(s.access(4000), Ok(Access::Active { remaining_seconds: 3600 }));
    }

    #[test]
    fn rejects_clock_rollback_and_overflow_without_crediting() {
        let mut s = station();
        s.access(100).unwrap();
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1000, 99), Err(Error::ClockWentBackwards));
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1000, u64::MAX), Err(Error::Overflow));
        assert_eq!(s.confirm_payment("pix-1", "PC-01", 1000, 100), Ok(vec![Event::Unlocked]));
    }

    #[test]
    fn rejects_invalid_and_duplicate_orders() {
        let mut s = station();
        assert_eq!(s.create_order("pix-1", 1000, 3600), Err(Error::DuplicateOrder));
        assert_eq!(s.create_order("zero", 0, 3600), Err(Error::InvalidOrder));
        assert_eq!(s.create_order("empty", 1000, 0), Err(Error::InvalidOrder));
    }
}
