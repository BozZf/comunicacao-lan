//! Contrato do adaptador provisório. Não produz BR Code nem movimenta dinheiro.
pub struct ChargeDisplay {
    pub payload: String,
    pub provider: String,
    pub notice: String,
}

pub trait PaymentGateway {
    fn display(&self, charge_id: &str) -> ChargeDisplay;
}

pub struct SimulatedPix;
impl PaymentGateway for SimulatedPix {
    fn display(&self, charge_id: &str) -> ChargeDisplay {
        ChargeDisplay {
            payload: format!("LAN-DEMO:{charge_id}"),
            provider: "simulated_pix".into(),
            notice: "Pix de teste. Não pague este QR: nenhum dinheiro é movimentado.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provisional_charge_cannot_be_mistaken_for_a_real_brcode() {
        let charge = SimulatedPix.display("cobranca-1");
        assert_eq!(charge.payload, "LAN-DEMO:cobranca-1");
        assert_eq!(charge.provider, "simulated_pix");
        assert!(!charge.payload.starts_with("000201"));
        assert!(charge.notice.contains("nenhum dinheiro"));
    }
}
