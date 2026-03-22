use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKey {
    Hub,
    GatewayA,
    GatewayB,
}

impl ProcessKey {
    pub fn title(self) -> &'static str {
        match self {
            Self::Hub => "Hub",
            Self::GatewayA => "Gateway A",
            Self::GatewayB => "Gateway B",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RunnerEvent {
    Output {
        key: ProcessKey,
        channel: &'static str,
        text: String,
    },
    Status {
        key: ProcessKey,
        text: String,
    },
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub key: ProcessKey,
    pub cli_path: PathBuf,
    pub workspace_dir: PathBuf,
    pub args: Vec<String>,
}

pub trait BackendSession: Send {
    fn send_line(&mut self, line: &str) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
}

pub trait Backend: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn spawn(
        &self,
        request: LaunchRequest,
        tx: Sender<RunnerEvent>,
    ) -> Result<Box<dyn BackendSession>, String>;
}

struct PaneState {
    key: ProcessKey,
    title: &'static str,
    input: String,
    output: String,
    status: String,
    session: Option<Box<dyn BackendSession>>,
}

impl PaneState {
    fn new(key: ProcessKey) -> Self {
        Self {
            key,
            title: key.title(),
            input: String::new(),
            output: String::new(),
            status: "idle".to_string(),
            session: None,
        }
    }

    fn append_output(&mut self, channel: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        for segment in normalize_output(text).lines() {
            self.output.push_str(&format!("[{channel}] {segment}\n"));
        }
        if !text.ends_with('\n') && !text.ends_with('\r') {
            self.output.push('\n');
        }
    }
}

pub struct RunnerApp<B: Backend> {
    backend: B,
    cli_path: String,
    server_url: String,
    gateway_host: String,
    workspace_dir: PathBuf,
    tx: Sender<RunnerEvent>,
    rx: Receiver<RunnerEvent>,
    panes: Vec<PaneState>,
}

impl<B: Backend> RunnerApp<B> {
    pub fn new(backend: B) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            backend,
            cli_path: default_cli_path().to_string_lossy().to_string(),
            server_url: "http://127.0.0.1:8080".to_string(),
            gateway_host: "127.0.0.1".to_string(),
            workspace_dir: openhanse_root(),
            tx,
            rx,
            panes: vec![
                PaneState::new(ProcessKey::Hub),
                PaneState::new(ProcessKey::GatewayA),
                PaneState::new(ProcessKey::GatewayB),
            ],
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                RunnerEvent::Output { key, channel, text } => {
                    if let Some(pane) = self.panes.iter_mut().find(|pane| pane.key == key) {
                        pane.append_output(channel, &text);
                    }
                }
                RunnerEvent::Status { key, text } => {
                    if let Some(pane) = self.panes.iter_mut().find(|pane| pane.key == key) {
                        pane.status = text;
                    }
                }
            }
        }
    }

    fn start_all(&mut self) {
        for key in [ProcessKey::Hub, ProcessKey::GatewayA, ProcessKey::GatewayB] {
            let _ = self.start_process(key);
        }
    }

    fn stop_all(&mut self) {
        for key in [ProcessKey::Hub, ProcessKey::GatewayA, ProcessKey::GatewayB] {
            self.stop_process(key);
        }
    }

    fn start_process(&mut self, key: ProcessKey) -> Result<(), String> {
        let index = self
            .panes
            .iter()
            .position(|pane| pane.key == key)
            .ok_or_else(|| "pane missing".to_string())?;

        if self.panes[index].session.is_some() {
            return Ok(());
        }

        let request = LaunchRequest {
            key,
            cli_path: PathBuf::from(self.cli_path.clone()),
            workspace_dir: self.workspace_dir.clone(),
            args: self.args_for(key),
        };

        let session = self.backend.spawn(request, self.tx.clone())?;
        self.panes[index].status = format!("running ({})", self.backend.name());
        self.panes[index].session = Some(session);
        Ok(())
    }

    fn stop_process(&mut self, key: ProcessKey) {
        if let Some(index) = self.panes.iter().position(|pane| pane.key == key) {
            if let Some(mut session) = self.panes[index].session.take() {
                let _ = session.stop();
            }
            self.panes[index].status = "stopped".to_string();
        }
    }

    fn send_input(&mut self, key: ProcessKey) {
        if let Some(index) = self.panes.iter().position(|pane| pane.key == key) {
            let line = self.panes[index].input.trim_end().to_string();
            if line.is_empty() {
                return;
            }
            if let Some(session) = self.panes[index].session.as_mut() {
                match session.send_line(&line) {
                    Ok(()) => {
                        self.panes[index].output.push_str(&format!("[stdin] {line}\n"));
                        self.panes[index].input.clear();
                    }
                    Err(error) => {
                        self.panes[index]
                            .output
                            .push_str(&format!("[error] failed to send input: {error}\n"));
                    }
                }
            } else {
                self.panes[index]
                    .output
                    .push_str("[info] process is not running\n");
            }
        }
    }

    fn args_for(&self, key: ProcessKey) -> Vec<String> {
        match key {
            ProcessKey::Hub => vec![
                "--id".to_string(),
                "hub".to_string(),
                "--peer-mode".to_string(),
                "hub".to_string(),
                "--server".to_string(),
                self.server_url.clone(),
            ],
            ProcessKey::GatewayA => vec![
                "--id".to_string(),
                "gateway-a".to_string(),
                "--target".to_string(),
                "gateway-b".to_string(),
                "--peer-mode".to_string(),
                "gateway".to_string(),
                "--server".to_string(),
                self.server_url.clone(),
                "--host".to_string(),
                self.gateway_host.clone(),
            ],
            ProcessKey::GatewayB => vec![
                "--id".to_string(),
                "gateway-b".to_string(),
                "--target".to_string(),
                "gateway-a".to_string(),
                "--peer-mode".to_string(),
                "gateway".to_string(),
                "--server".to_string(),
                self.server_url.clone(),
                "--host".to_string(),
                self.gateway_host.clone(),
            ],
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("Backend: {}", self.backend.name()));
            if ui.button("Start All").clicked() {
                self.start_all();
            }
            if ui.button("Stop All").clicked() {
                self.stop_all();
            }
        });
        ui.horizontal(|ui| {
            ui.label("CLI");
            ui.text_edit_singleline(&mut self.cli_path);
        });
        ui.horizontal(|ui| {
            ui.label("Server");
            ui.text_edit_singleline(&mut self.server_url);
            ui.label("Gateway Host");
            ui.text_edit_singleline(&mut self.gateway_host);
        });
        ui.label(format!(
            "Workspace: {}",
            self.workspace_dir.to_string_lossy()
        ));
    }

    fn draw_pane(&mut self, ui: &mut egui::Ui, key: ProcessKey) {
        let index = match self.panes.iter().position(|pane| pane.key == key) {
            Some(index) => index,
            None => return,
        };

        let mut start_clicked = false;
        let mut stop_clicked = false;
        let mut send_clicked = false;
        let mut clear_clicked = false;

        let pane = &mut self.panes[index];

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.heading(pane.title);
                ui.label(format!("Status: {}", pane.status));
                ui.horizontal(|ui| {
                    start_clicked = ui.button("Start").clicked();
                    stop_clicked = ui.button("Stop").clicked();
                    clear_clicked = ui.button("Clear").clicked();
                });
                ui.horizontal(|ui| {
                    let response = ui.text_edit_singleline(&mut pane.input);
                    let enter_pressed = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    send_clicked = ui.button("Send").clicked() || enter_pressed;
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut pane.output)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .desired_rows(18)
                                .interactive(false),
                        );
                    });
            });
        });

        if clear_clicked {
            self.panes[index].output.clear();
        }
        if start_clicked {
            let _ = self.start_process(key);
        }
        if stop_clicked {
            self.stop_process(key);
        }
        if send_clicked {
            self.send_input(key);
        }
    }
}

impl<B: Backend> eframe::App for RunnerApp<B> {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        ctx.request_repaint_after(Duration::from_millis(50));

        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            self.top_bar(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_height = ui.available_height();
            let top_height = total_height * 0.42;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), top_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.draw_pane(ui, ProcessKey::Hub);
                },
            );
            ui.separator();
            ui.columns(2, |columns| {
                self.draw_pane(&mut columns[0], ProcessKey::GatewayA);
                self.draw_pane(&mut columns[1], ProcessKey::GatewayB);
            });
        });
    }
}

pub fn run_native_app<B: Backend>(title: &str, backend: B) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 950.0]),
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |_cc| Ok(Box::new(RunnerApp::new(backend)))),
    )
}

pub fn openhanse_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("../../..")
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.join("../../.."))
}

pub fn default_cli_path() -> PathBuf {
    openhanse_root().join("Source/openhanse-cli/Artefact/openhanse-cli-macos-apple-silicon")
}

fn normalize_output(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}
