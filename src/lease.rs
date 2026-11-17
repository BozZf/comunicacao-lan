//! Contagem local monotônica. Respostas repetidas nunca estendem uma autorização.
use crate::protocol::SessionView;
use std::time::Duration;

#[derive(Default)]
pub struct Lease {
    revision: Option<u64>,
    deadline: Duration,
    warned: [bool; 3],
}

impl Lease {
    pub fn apply(&mut self, session: &SessionView, now: Duration, round_trip: Duration) {
        if self.revision.is_some_and(|revision| session.revision < revision) {
            return;
        }
        // Subtrair todo o RTT é conservador: uma resposta lenta não compra tempo.
        let remaining = Duration::from_secs(session.remaining_seconds).saturating_sub(round_trip);
        let proposed = now.saturating_add(remaining);
        if self.revision != Some(session.revision) {
            self.deadline = proposed;
            self.revision = Some(session.revision);
            self.warned = [false; 3];
        } else {
            self.deadline = self.deadline.min(proposed);
        }
    }

    pub fn remaining(&self, now: Duration) -> Duration {
        self.deadline.saturating_sub(now)
    }

    pub fn warning(&mut self, now: Duration) -> Option<u64> {
        let remaining = self.remaining(now);
        if remaining.is_zero() {
            return None;
        }
        let seconds = remaining.as_secs().saturating_add(u64::from(remaining.subsec_nanos() > 0));
        let index = [600, 300, 60].iter().rposition(|limit| seconds <= *limit)?;
        let emit = !self.warned[index];
        self.warned[..=index].fill(true);
        emit.then_some(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view(seconds: u64, revision: u64) -> SessionView {
        SessionView { machine_id: "PC-01".into(), remaining_seconds: seconds,
            revision, pending_order: None }
    }
    #[test]
    fn repeated_or_old_response_cannot_extend_even_after_expiration() {
        let mut lease = Lease::default();
        lease.apply(&view(60, 1), Duration::ZERO, Duration::ZERO);
        lease.apply(&view(60, 1), Duration::from_secs(20), Duration::ZERO);
        assert_eq!(lease.remaining(Duration::from_secs(20)).as_secs(), 40);
        lease.apply(&view(600, 0), Duration::from_secs(30), Duration::ZERO);
        assert_eq!(lease.remaining(Duration::from_secs(60)), Duration::ZERO);
        lease.apply(&view(60, 1), Duration::from_secs(70), Duration::ZERO);
        assert_eq!(lease.remaining(Duration::from_secs(70)), Duration::ZERO);
    }
    #[test]
    fn renewal_extends_and_latency_is_discounted() {
        let mut lease = Lease::default();
        lease.apply(&view(10, 1), Duration::ZERO, Duration::from_secs(2));
        assert_eq!(lease.remaining(Duration::ZERO).as_secs(), 8);
        lease.apply(&view(100, 2), Duration::from_secs(5), Duration::from_secs(3));
        assert_eq!(lease.remaining(Duration::from_secs(5)).as_secs(), 97);
    }
    #[test]
    fn offline_countdown_expires_and_warning_is_not_repeated() {
        let mut lease = Lease::default();
        lease.apply(&view(90, 1), Duration::ZERO, Duration::ZERO);
        assert_eq!(lease.warning(Duration::ZERO), Some(90));
        assert_eq!(lease.warning(Duration::from_secs(1)), None);
        assert_eq!(lease.warning(Duration::from_secs(30)), Some(60));
        assert_eq!(lease.remaining(Duration::from_secs(90)), Duration::ZERO);
    }
}
