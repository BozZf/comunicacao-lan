use comunicacao_lan::{Access, Event, LanRegistry, PaymentAuthority, TrustedNow};

fn main() {
    println!("SIMULAÇÃO: sem Pix real e sem bloquear ou liberar o Windows.");
    let authority = PaymentAuthority::new_server_authority();
    let mut lan = LanRegistry::new();
    lan.register_station("PC-01").unwrap();
    lan.create_order("PC-01", "cobranca-demo", 1000, 3600).unwrap();
    assert_eq!(
        lan.access("PC-01", TrustedNow::from_monotonic_seconds(0)),
        Ok(Access::Locked)
    );
    println!("Pedido criado: PC continua bloqueado no domínio.");
    let payment = authority.verify("cobranca-demo", "PC-01", 1000).unwrap();
    println!(
        "Pagamento simulado: {:?}",
        lan.confirm_payment(&payment, TrustedNow::from_monotonic_seconds(0))
            .unwrap()
    );
    for now in [3000, 3300, 3540, 3600] {
        for event in lan
            .tick("PC-01", TrustedNow::from_monotonic_seconds(now))
            .unwrap()
        {
            match event {
                Event::Warning { remaining_seconds } => {
                    println!("Aviso: {remaining_seconds} segundos restantes")
                }
                other => println!("Evento: {other:?}"),
            }
        }
    }
}
