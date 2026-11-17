use lan_agent::PrivateConfig;
use lan_transport::{Identity, PublicConfig};

fn main() {
    if let Err(error) = run() { eprintln!("{error}"); std::process::exit(1); }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--provision") => {
            if args.len() != 4 { return Err("Use --provision <private.json> <agent-public.json>.".into()); }
            let identity = Identity::generate();
            let config = PrivateConfig {
                station_token: std::env::var("LAN_STATION_TOKEN").map_err(|_| "Defina LAN_STATION_TOKEN.")?,
                server_public_key: std::env::var("LAN_SERVER_PUBLIC_KEY").map_err(|_| "Defina LAN_SERVER_PUBLIC_KEY.")?,
                agent_signing_secret: identity.secret_hex(),
                client_sid: std::env::var("LAN_ALLOWED_CLIENT_SID").map_err(|_| "Defina LAN_ALLOWED_CLIENT_SID.")?,
                service_sid: std::env::var("LAN_SERVICE_SID").map_err(|_| "Defina LAN_SERVICE_SID.")?,
            };
            config.validate()?;
            // O instalador cria diretórios com ACL antes desta operação.
            write_new(&args[2], &serde_json::to_vec(&config).map_err(|_| "Configuração inválida.")?)?;
            write_new(&args[3], &serde_json::to_vec(&PublicConfig { agent_public_key: identity.public_hex() }).map_err(|_| "Configuração inválida.")?)?;
            Ok(())
        }
        #[cfg(windows)]
        Some("--service") => {
            let path = args.get(2).ok_or("Caminho da configuração privada ausente.")?;
            let bytes = std::fs::read(path).map_err(|_| "Não foi possível ler a configuração privada.")?;
            if bytes.len() > 8192 { return Err("Configuração privada excede o limite.".into()); }
            let config: PrivateConfig = serde_json::from_slice(&bytes).map_err(|_| "Configuração privada inválida.")?;
            config.validate()?;
            lan_agent::windows::dispatch(config)
        }
        #[cfg(windows)]
        Some("--probe") => {
            let public = lan_transport::installed_public_key()?;
            let action = match args.get(2).map(String::as_str) {
                Some("--buy") => lan_transport::Action::Buy { plan_id: args.get(3).ok_or("Pacote ausente.")?.clone() },
                _ => lan_transport::Action::Status,
            };
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Runtime indisponível.")?;
            let reply = runtime.block_on(lan_transport::call(&public, action))?;
            println!("{}", serde_json::to_string(&reply).map_err(|_| "Resposta inválida.")?);
            Ok(())
        }
        _ => Err("Agente experimental: --provision, --service ou --probe (Windows).".into()),
    }
}

fn write_new(path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path)
        .map_err(|_| "Não foi possível criar configuração. Não sobrescreva uma instalação existente.")?;
    file.write_all(bytes).map_err(|_| "Falha ao gravar configuração.")?;
    file.sync_all().map_err(|_| "Falha ao persistir configuração.".into())
}
