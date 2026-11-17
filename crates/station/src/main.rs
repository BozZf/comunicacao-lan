//! Protótipo interativo, sem privilégios e sem proteção do Windows.
//! O watchdog deste processo não substitui um serviço protegido do sistema.
use comunicacao_lan::{lease::Lease, protocol::{CreateOrder, OrderView, Plan, SessionView}};
use eframe::egui;
use qrcode::{Color, QrCode};
use std::{sync::mpsc::{self, Receiver, SyncSender}, time::{Duration, Instant}};

const SERVER: &str = "http://127.0.0.1:8787";

enum Command { Buy(String) }
enum Update {
    Catalog(Vec<Plan>),
    Session(SessionView, Instant, Duration, bool, Option<String>),
    Failure(String),
}

fn send(sender: &SyncSender<Update>, update: Update) {
    let _ = sender.try_send(update);
}

fn demo_worker(token: String, commands: Receiver<Command>, updates: SyncSender<Update>) {
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(1)).timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none()).no_proxy().build() {
        Ok(client) => client,
        Err(error) => { send(&updates, Update::Failure(format!("Falha ao iniciar comunicação: {error}"))); return; }
    };
    let mut catalog_loaded = false;
    loop {
        if !catalog_loaded {
            match client.get(format!("{SERVER}/api/catalog")).bearer_auth(&token).send()
                .and_then(|r| r.error_for_status()).and_then(|r| r.json::<Vec<Plan>>()) {
                Ok(catalog) => { send(&updates, Update::Catalog(catalog)); catalog_loaded = true; }
                Err(_) => send(&updates, Update::Failure("Não foi possível carregar os pacotes. Verifique servidor e credencial.".into())),
            }
        }
        let requested = Instant::now();
        match client.get(format!("{SERVER}/api/session")).bearer_auth(&token).send()
            .and_then(|r| r.error_for_status()).and_then(|r| r.json::<SessionView>()) {
            Ok(session) if session.machine_id == "PC-01" => {
                let received = Instant::now();
                send(&updates, Update::Session(session, received, received.duration_since(requested), true, None));
            }
            Ok(_) => send(&updates, Update::Failure("Resposta para outra máquina rejeitada.".into())),
            Err(_) => send(&updates, Update::Failure("Sem conexão. O tempo já autorizado continua contando localmente.".into())),
        }
        match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(Command::Buy(plan_id)) => {
                let result = client.post(format!("{SERVER}/api/orders")).bearer_auth(&token)
                    .json(&CreateOrder { plan_id }).send();
                match result {
                    Ok(response) if response.status().is_success() => {}
                    Ok(response) => {
                        let message = if response.status().as_u16() == 409 {
                            "Já existe uma cobrança pendente. Aguarde a confirmação ou a expiração."
                        } else { "Não foi possível criar a cobrança. Verifique o servidor." };
                        send(&updates, Update::Failure(message.into()));
                    }
                    Err(_) => send(&updates, Update::Failure("Falha na criação da cobrança. Ao reconectar, a cobrança existente será recuperada.".into())),
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

enum ConnectionMode {
    Demo(String),
    #[cfg(windows)]
    Agent(String),
}

fn worker(mode: ConnectionMode, commands: Receiver<Command>, updates: SyncSender<Update>) {
    match mode {
        ConnectionMode::Demo(token) => demo_worker(token, commands, updates),
        #[cfg(windows)]
        ConnectionMode::Agent(public) => agent_worker(public, commands, updates),
    }
}

#[cfg(windows)]
fn agent_worker(public: String, commands: Receiver<Command>, updates: SyncSender<Update>) {
    use lan_transport::{Action, Reply};
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(_) => { send(&updates, Update::Failure("Não foi possível iniciar o cliente do agente.".into())); return; }
    };
    let mut catalog_loaded = false;
    loop {
        if !catalog_loaded {
            if let Ok(Reply::Catalog { plans }) = runtime.block_on(lan_transport::call(&public, Action::Catalog)) {
                catalog_loaded = !plans.is_empty();
                send(&updates, Update::Catalog(plans));
            }
        }
        let requested = Instant::now();
        match runtime.block_on(lan_transport::call(&public, Action::Status)) {
            Ok(Reply::Status { session, online, message }) => {
                let received = Instant::now();
                send(&updates, Update::Session(session, received, received.duration_since(requested), online, message));
            }
            _ => send(&updates, Update::Failure("Agente indisponível ou assinatura inválida. Sem autorização nova.".into())),
        }
        match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::Buy(plan_id)) => {
                match runtime.block_on(lan_transport::call(&public, Action::Buy { plan_id })) {
                    Ok(Reply::OrderCreated) => {},
                    Ok(Reply::Failure { message }) => send(&updates, Update::Failure(message)),
                    _ => send(&updates, Update::Failure("Agente não confirmou a criação da cobrança.".into())),
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {},
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

struct StationApp {
    started: Instant,
    lease: Lease,
    updates: Receiver<Update>,
    commands: SyncSender<Command>,
    catalog: Vec<Plan>,
    pending: Option<OrderView>,
    qr: Option<QrCode>,
    connection_error: Option<String>,
    last_contact: Option<Instant>,
    warning: Option<String>,
    last_revision: u64,
    active_view: bool,
    waiting_for_order: bool,
}

impl StationApp {
    fn new(mode: ConnectionMode) -> Self {
        let (command_tx, command_rx) = mpsc::sync_channel(4);
        let (update_tx, update_rx) = mpsc::sync_channel(8);
        std::thread::spawn(move || worker(mode, command_rx, update_tx));
        Self {
            started: Instant::now(), lease: Lease::default(), updates: update_rx,
            commands: command_tx, catalog: Vec::new(), pending: None, qr: None,
            connection_error: None, last_contact: None, warning: None,
            last_revision: 0, active_view: false, waiting_for_order: false,
        }
    }

    fn receive_updates(&mut self) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::Catalog(catalog) => self.catalog = catalog,
                Update::Failure(error) => { self.connection_error = Some(error); self.waiting_for_order = false; }
                Update::Session(session, received, round_trip, online, message) => {
                    self.lease.apply(&session, self.started.elapsed(), round_trip.saturating_add(received.elapsed()));
                    if session.revision > self.last_revision {
                        self.warning = None;
                        self.last_revision = session.revision;
                    }
                    if self.pending.as_ref().map(|p| &p.id) != session.pending_order.as_ref().map(|p| &p.id) {
                        self.qr = session.pending_order.as_ref().and_then(|order| QrCode::new(order.qr_payload.as_bytes()).ok());
                    }
                    self.pending = session.pending_order;
                    self.connection_error = message;
                    self.last_contact = online.then_some(received);
                    self.waiting_for_order = false;
                }
            }
        }
    }

    fn purchase(&mut self, ui: &mut egui::Ui) {
        ui.heading("Escolha seu tempo");
        ui.label("Valores fictícios para testar o fluxo de compra.");
        ui.add_space(16.0);
        if let Some(order) = &self.pending {
            ui.strong("Aguardando pagamento simulado");
            ui.label(format!("R$ {:.2} · cobrança {}", order.amount_cents as f64 / 100.0, order.id));
            if let Some(qr) = &self.qr { draw_qr(ui, qr); }
            ui.label("QR DE DEMONSTRAÇÃO — NÃO É PIX");
            ui.label("O código identifica a cobrança. A confirmação é feita no painel do atendente durante este teste.");
            ui.label("Esta cobrança expira em 10 minutos após a criação.");
        } else {
            for plan in &self.catalog {
                let label = format!("{} · R$ {:.2}", plan.label, plan.amount_cents as f64 / 100.0);
                let connected = self.last_contact.is_some_and(|time| time.elapsed() < Duration::from_secs(5));
                if ui.add_enabled(connected && !self.waiting_for_order, egui::Button::new(label).min_size(egui::vec2(300.0, 48.0))).clicked() {
                    if self.commands.try_send(Command::Buy(plan.id.clone())).is_ok() {
                        self.waiting_for_order = true;
                    }
                }
                ui.add_space(8.0);
            }
            if self.catalog.is_empty() { ui.label("Conectando ao servidor local…"); }
            if self.waiting_for_order { ui.label("Criando cobrança…"); }
        }
    }
}

impl eframe::App for StationApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive_updates();
        let now = self.started.elapsed();
        let remaining = self.lease.remaining(now);
        let active = !remaining.is_zero();
        if active != self.active_view {
            self.active_view = active;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!active));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if active { egui::WindowLevel::Normal } else { egui::WindowLevel::AlwaysOnTop }));
            if active { ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(560.0, 650.0))); }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if let Some(seconds) = self.lease.warning(now) {
            self.warning = Some(format!("Seu tempo está acabando: {seconds} segundos restantes."));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(700.0);
                    ui.add_space(24.0);
                    ui.label(egui::RichText::new("COMUNICAÇÃO LAN · PC-01").size(24.0).color(egui::Color32::from_rgb(97, 221, 191)));
                    ui.colored_label(egui::Color32::YELLOW, "DEMONSTRAÇÃO — esta janela ainda não protege o Windows.");
                    ui.add_space(20.0);
                    if active {
                        let seconds = remaining.as_secs().saturating_add(u64::from(remaining.subsec_nanos() > 0));
                        ui.heading("Sessão autorizada");
                        ui.label(egui::RichText::new(format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)).size(40.0));
                        ui.label("A tela de compra retorna automaticamente quando o tempo acaba.");
                        if let Some(warning) = &self.warning { ui.colored_label(egui::Color32::YELLOW, warning); }
                        ui.add_space(16.0);
                        ui.separator();
                        ui.label("Renovar adiciona tempo à sessão atual.");
                    } else {
                        ui.heading("Sem sessão ativa");
                    }
                    self.purchase(ui);
                    ui.add_space(20.0);
                    if let Some(error) = &self.connection_error { ui.colored_label(egui::Color32::YELLOW, error); }
                    ui.separator();
                    ui.small("Fechar esta janela não encerra um agente instalado. A proteção de acesso do Windows ainda não foi implementada.");
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

fn draw_qr(ui: &mut egui::Ui, qr: &QrCode) {
    let modules = qr.width();
    let pixel = 4.0;
    let size = (modules + 8) as f32 * pixel;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
    for y in 0..modules {
        for x in 0..modules {
            if qr[(x, y)] == Color::Dark {
                let min = rect.min + egui::vec2((x + 4) as f32 * pixel, (y + 4) as f32 * pixel);
                ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::vec2(pixel, pixel)), 0.0, egui::Color32::BLACK);
            }
        }
    }
}

fn main() -> eframe::Result {
    let mode = if std::env::args().any(|arg| arg == "--demo") {
        match std::env::var("LAN_STATION_TOKEN") {
            Ok(token) if token.len() >= 32 => ConnectionMode::Demo(token),
            _ => { eprintln!("Defina LAN_STATION_TOKEN para a demonstração direta."); return Ok(()); }
        }
    } else {
        #[cfg(windows)]
        {
            match lan_transport::installed_public_key() {
                Ok(public) => ConnectionMode::Agent(public),
                Err(error) => { eprintln!("{error}"); return Ok(()); }
            }
        }
        #[cfg(not(windows))]
        { eprintln!("Modo de serviço disponível somente no Windows. Use --demo para demonstração direta."); return Ok(()); }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Comunicação LAN · demonstração")
            .with_fullscreen(true).with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native("Comunicação LAN", options, Box::new(move |creation| {
        creation.egui_ctx.set_visuals(egui::Visuals::dark());
        Ok(Box::new(StationApp::new(mode)))
    }))
}
