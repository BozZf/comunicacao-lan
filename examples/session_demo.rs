use comunicacao_lan::{Access, Event, Station};

fn main() {
    println!("SIMULAÇÃO: sem Pix real e sem bloquear ou liberar o Windows.");
    let mut pc = Station::new("PC-01").unwrap();
    pc.create_order("cobranca-demo", 1000, 3600).unwrap();
    assert_eq!(pc.access(0), Ok(Access::Locked));
    println!("Pedido criado: PC continua bloqueado no domínio.");
    println!("Pagamento simulado: {:?}", pc.confirm_payment("cobranca-demo", "PC-01", 1000, 0).unwrap());
    for now in [3000, 3300, 3540, 3600] {
        for event in pc.tick(now).unwrap() {
            match event {
                Event::Warning { remaining_seconds } => println!("Aviso: {remaining_seconds} segundos restantes"),
                other => println!("Evento: {other:?}"),
            }
        }
    }
}
