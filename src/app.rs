use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    config::{APP_TITLE, GAME_EXE},
    events::Event,
    install, platform, state,
    worker::{self, Operation},
};

pub struct LauncherApp {
    game_input: String,
    game_dir: Option<PathBuf>,
    installed: Option<String>,
    stage: String,
    logs: Vec<String>,
    progress: Option<(u64, Option<u64>)>,
    busy: bool,
    result_ok: Option<bool>,
    result_message: String,
    close_requested: bool,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let saved = state::load();
        let game_dir = platform::normalize_game_dir(Path::new(&saved.game_dir));
        let installed = game_dir.as_deref().and_then(install::installed_version);
        let game_input = game_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(saved.game_dir);
        let (tx, rx) = unbounded();
        Self {
            game_input,
            game_dir,
            installed,
            stage: "Готов к работе".into(),
            logs: Vec::new(),
            progress: None,
            busy: false,
            result_ok: None,
            result_message: String::new(),
            close_requested: false,
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn validate_path(&mut self) {
        self.game_dir = platform::normalize_game_dir(Path::new(self.game_input.trim()));
        self.installed = self
            .game_dir
            .as_deref()
            .and_then(install::installed_version);
    }

    fn begin(&mut self, operation: Operation) {
        let Some(game) = self.game_dir.clone() else {
            self.result_ok = Some(false);
            self.result_message =
                format!("Выберите папку, где находится {GAME_EXE}, или родительскую папку Game");
            return;
        };
        self.busy = true;
        self.result_ok = None;
        self.result_message.clear();
        self.progress = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        worker::spawn(operation, game, self.tx.clone(), self.cancel.clone());
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Log(line) => {
                    state::append_log(&line);
                    self.logs.push(format!("{}  {}", chrono_time(), line));
                    if self.logs.len() > 500 {
                        self.logs.drain(..100);
                    }
                }
                Event::Stage(stage) => {
                    state::append_log(&format!("Этап: {stage}"));
                    self.stage = stage;
                    self.progress = None;
                }
                Event::Progress { done, total } => self.progress = Some((done, total)),
                Event::Finished {
                    ok,
                    message,
                    close_after,
                } => {
                    state::append_log(&format!(
                        "{}: {message}",
                        if ok { "Готово" } else { "Ошибка" }
                    ));
                    self.busy = false;
                    self.result_ok = Some(ok);
                    self.result_message = message.clone();
                    self.stage = if ok {
                        "Готово".into()
                    } else {
                        "Ошибка".into()
                    };
                    self.validate_path();
                    if close_after {
                        self.close_requested = true;
                    }
                }
            }
            ctx.request_repaint();
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        if self.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(APP_TITLE)
                            .size(28.0)
                            .strong()
                            .color(Color32::from_rgb(224, 190, 112)),
                    );
                    ui.label(
                        RichText::new("Нативный установщик · обновление · восстановление · запуск")
                            .color(Color32::from_gray(165)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new("ELDEN RING NIGHTREIGN")
                            .size(15.0)
                            .strong()
                            .color(Color32::from_gray(120)),
                    );
                });
            });
            ui.add_space(16.0);

            egui::Frame::new()
                .fill(Color32::from_rgb(24, 24, 27))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(70, 61, 43)))
                .corner_radius(8.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("ПАПКА ИГРЫ")
                            .small()
                            .strong()
                            .color(Color32::from_rgb(190, 160, 100)),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let response = ui.add_enabled(
                            !self.busy,
                            egui::TextEdit::singleline(&mut self.game_input)
                                .desired_width(f32::INFINITY),
                        );
                        if response.changed() {
                            self.validate_path();
                        }
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new("Обзор…").min_size(Vec2::new(92.0, 30.0)),
                            )
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_title("Выберите папку ELDEN RING NIGHTREIGN или Game")
                                .pick_folder()
                        {
                            self.game_input = path.to_string_lossy().into_owned();
                            self.validate_path();
                        }
                    });
                    ui.add_space(7.0);
                    if let Some(path) = &self.game_dir {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("● Игра найдена")
                                    .color(Color32::from_rgb(103, 198, 136)),
                            );
                            ui.separator();
                            ui.label(format!(
                                "Мод: {}",
                                self.installed.as_deref().unwrap_or("не установлен")
                            ));
                            if ui.small_button("Открыть папку").clicked() {
                                platform::open_folder(path);
                            }
                        });
                    } else {
                        ui.label(
                            RichText::new(format!("Файл {GAME_EXE} не найден"))
                                .color(Color32::from_rgb(220, 115, 105)),
                        );
                    }
                });

            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                let ready = self.game_dir.is_some() && !self.busy;
                if ui
                    .add_enabled(ready, primary_button("Установить / обновить"))
                    .clicked()
                {
                    self.begin(Operation::Install);
                }
                if ui
                    .add_enabled(ready, primary_button("Проверить и запустить"))
                    .clicked()
                {
                    self.begin(Operation::Launch);
                }
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new("Проверить обновление").min_size(Vec2::new(165.0, 38.0)),
                    )
                    .clicked()
                {
                    self.begin(Operation::Check);
                }
                if ui
                    .add_enabled(
                        ready && self.installed.is_some(),
                        egui::Button::new("Удалить мод").min_size(Vec2::new(120.0, 38.0)),
                    )
                    .clicked()
                {
                    self.begin(Operation::Uninstall);
                }
                if self.busy
                    && ui
                        .add(egui::Button::new("Остановить").min_size(Vec2::new(110.0, 38.0)))
                        .clicked()
                {
                    self.cancel.store(true, Ordering::Relaxed);
                    self.stage = "Остановка после текущей безопасной операции…".into();
                }
            });

            ui.add_space(12.0);
            egui::Frame::new()
                .fill(Color32::from_rgb(20, 20, 23))
                .corner_radius(8.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(&self.stage).strong());
                    });
                    if let Some((done, total)) = self.progress {
                        let fraction = total
                            .map(|t| (done as f32 / t.max(1) as f32).clamp(0.0, 1.0))
                            .unwrap_or(0.0);
                        let text = match total {
                            Some(total) => format!(
                                "{:.1} / {:.1} МБ · {}%",
                                done as f64 / 1_048_576.0,
                                total as f64 / 1_048_576.0,
                                (fraction * 100.0) as u32
                            ),
                            None => format!("{:.1} МБ", done as f64 / 1_048_576.0),
                        };
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .animate(total.is_none())
                                .text(text),
                        );
                    } else {
                        ui.add(
                            egui::ProgressBar::new(if self.busy { 0.35 } else { 0.0 })
                                .animate(self.busy),
                        );
                    }
                    if let Some(ok) = self.result_ok {
                        ui.add_space(6.0);
                        ui.label(RichText::new(&self.result_message).color(if ok {
                            Color32::from_rgb(103, 198, 136)
                        } else {
                            Color32::from_rgb(230, 110, 100)
                        }));
                    }
                });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("ЖУРНАЛ")
                        .small()
                        .strong()
                        .color(Color32::from_gray(145)),
                );
                if ui.small_button("Очистить").clicked() {
                    self.logs.clear();
                }
            });
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(250.0)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(13, 13, 15))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            if self.logs.is_empty() {
                                ui.label(
                                    RichText::new(
                                        "Здесь появятся подробности установки и загрузки.",
                                    )
                                    .color(Color32::from_gray(95)),
                                );
                            }
                            for line in &self.logs {
                                ui.label(
                                    RichText::new(line)
                                        .monospace()
                                        .size(12.0)
                                        .color(Color32::from_rgb(175, 190, 175)),
                                );
                            }
                        });
                });
        });
    }
}

fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(text)
            .strong()
            .color(Color32::from_rgb(30, 24, 15)),
    )
    .fill(Color32::from_rgb(213, 178, 102))
    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(245, 210, 130)))
    .min_size(Vec2::new(180.0, 38.0))
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(15, 15, 17);
    style.visuals.window_fill = Color32::from_rgb(19, 19, 22);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 38, 42);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 50, 38);
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    ctx.set_style(style);

    let font_path = Path::new(r"C:\Windows\Fonts\segoeui.ttf");
    if let Ok(bytes) = std::fs::read(font_path) {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("segoe".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe".into());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "segoe".into());
        ctx.set_fonts(fonts);
    }
}

fn chrono_time() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}
