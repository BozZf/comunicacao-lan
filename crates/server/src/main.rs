use lan_server::{app, store::Store, Config, ADDRESS};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let admin = std::env::var("LAN_ADMIN_TOKEN").map_err(|_| "Defina LAN_ADMIN_TOKEN.")?;
    let station = std::env::var("LAN_STATION_TOKEN").map_err(|_| "Defina LAN_STATION_TOKEN.")?;
    let config = Config::new(admin, station, std::env::var("LAN_DEMO_PAYMENTS").as_deref() == Ok("1"))?;
    let path = std::env::var("LAN_DB").unwrap_or_else(|_| "lan-demo.sqlite3".into());
    let store = Store::open(path).map_err(|error| format!("Não foi possível abrir o banco: {error:?}"))?;
    let listener = tokio::net::TcpListener::bind(ADDRESS).await?;
    println!("Protótipo local — NÃO processa Pix nem protege o Windows.");
    println!("Painel: http://{ADDRESS}/");
    axum::serve(listener, app(store, config)).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    }).await?;
    Ok(())
}
