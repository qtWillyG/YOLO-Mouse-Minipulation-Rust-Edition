//! YoloMouse (Rust) - move the cursor onto a YOLOv10-detected object.
//!   GUI: egui / eframe     Vision: ort (ONNX Runtime) + image
//!   Output: Windows SendInput OR RP2040/RP2350 USB-HID firmware
#![windows_subsystem = "windows"] // no console window

mod backends;
mod capture;
mod config;
mod detector;
mod worker;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use backends::SerialMouse;
use config::{Activation, Backend, Settings, Shared, TargetMode};

const KEYS: &[(&str, i32)] = &[
    ("Right Mouse", 0x02),
    ("Mouse 4 (XBUTTON1)", 0x05),
    ("Mouse 5 (XBUTTON2)", 0x06),
    ("Left Mouse", 0x01),
    ("Left Shift", 0xA0),
    ("Left Ctrl", 0xA2),
    ("Left Alt", 0xA4),
    ("Caps Lock", 0x14),
    ("Space", 0x20),
    ("F1", 0x70),
    ("F2", 0x71),
    ("F3", 0x72),
];

fn key_name(vk: i32) -> &'static str {
    KEYS.iter().find(|k| k.1 == vk).map(|k| k.0).unwrap_or("Right Mouse")
}

struct App {
    shared: Arc<Shared>,
    s: Settings,
    mover_enabled: bool,
    preview_enabled: bool,
    model_path: String,
    ports: Vec<String>,
    selected_port: String,
    tex: Option<egui::TextureHandle>,
}

impl App {
    fn new(shared: Arc<Shared>) -> Self {
        let s = shared.settings.lock().unwrap().clone();
        let ports = SerialMouse::list_ports();
        let selected_port = ports.first().cloned().unwrap_or_default();
        Self {
            shared,
            s,
            mover_enabled: false,
            preview_enabled: true,
            model_path: String::new(),
            ports,
            selected_port,
            tex: None,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // refresh preview texture (consume so we don't re-upload unchanged frames)
        if let Some(p) = self.shared.preview.lock().unwrap().take() {
            let color = egui::ColorImage::from_rgba_unmultiplied([p.w, p.h], &p.rgba);
            self.tex = Some(ctx.load_texture("preview", color, egui::TextureOptions::LINEAR));
        }

        let status = self.shared.status.lock().unwrap().clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // ---- model ----
                egui::CollapsingHeader::new("Model").default_open(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.model_path);
                        if ui.button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("ONNX model", &["onnx"])
                                .pick_file()
                            {
                                self.model_path = p.display().to_string();
                            }
                        }
                    });
                    ui.checkbox(&mut self.s.use_gpu, "Use GPU (build with --features gpu)");
                    ui.horizontal(|ui| {
                        if ui.button("Load model").clicked() && !self.model_path.is_empty() {
                            self.shared.commands.lock().unwrap().load_model =
                                Some(self.model_path.clone());
                        }
                        if status.model_loaded {
                            ui.colored_label(
                                egui::Color32::from_rgb(120, 230, 120),
                                format!("loaded ({})", status.provider),
                            );
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(230, 150, 90), "not loaded");
                        }
                    });
                });

                // ---- output backend ----
                egui::CollapsingHeader::new("Output backend").default_open(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.s.backend, Backend::Windows, "Windows (SendInput)");
                        ui.radio_value(&mut self.s.backend, Backend::Serial, "RP2040/RP2350 HID");
                    });
                    if self.s.backend == Backend::Serial {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_label("COM port")
                                .selected_text(self.selected_port.clone())
                                .show_ui(ui, |ui| {
                                    for p in &self.ports {
                                        ui.selectable_value(&mut self.selected_port, p.clone(), p);
                                    }
                                });
                            if ui.button("Refresh").clicked() {
                                self.ports = SerialMouse::list_ports();
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Connect").clicked() && !self.selected_port.is_empty() {
                                self.shared.commands.lock().unwrap().connect =
                                    Some(self.selected_port.clone());
                            }
                            if ui.button("Disconnect").clicked() {
                                self.shared.commands.lock().unwrap().disconnect = true;
                            }
                            let (c, t) = if status.serial_verified {
                                (egui::Color32::from_rgb(120, 230, 120), "verified")
                            } else if status.serial_connected {
                                (egui::Color32::from_rgb(230, 230, 90), "open (unverified)")
                            } else {
                                (egui::Color32::from_rgb(230, 150, 90), "disconnected")
                            };
                            ui.colored_label(c, t);
                        });
                    }
                });

                // ---- activation ----
                egui::CollapsingHeader::new("Activation").default_open(true).show(ui, |ui| {
                    ui.checkbox(&mut self.mover_enabled, "MOVER ENABLED (master switch)");
                    egui::ComboBox::from_label("Mode")
                        .selected_text(match self.s.activation {
                            Activation::Hold => "Hold key",
                            Activation::Toggle => "Toggle key",
                            Activation::Always => "Always on",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.s.activation, Activation::Hold, "Hold key");
                            ui.selectable_value(&mut self.s.activation, Activation::Toggle, "Toggle key");
                            ui.selectable_value(&mut self.s.activation, Activation::Always, "Always on");
                        });
                    egui::ComboBox::from_label("Trigger key")
                        .selected_text(key_name(self.s.activation_vk))
                        .show_ui(ui, |ui| {
                            for (name, vk) in KEYS {
                                ui.selectable_value(&mut self.s.activation_vk, *vk, *name);
                            }
                        });
                    ui.checkbox(&mut self.s.click_on_target, "Click left button when on target");
                });

                // ---- detection & capture ----
                egui::CollapsingHeader::new("Detection & capture").default_open(true).show(ui, |ui| {
                    ui.add(egui::Slider::new(&mut self.s.conf, 0.05..=0.95).text("Confidence"));
                    ui.checkbox(&mut self.s.full_screen, "Capture full screen");
                    ui.add_enabled(
                        !self.s.full_screen,
                        egui::Slider::new(&mut self.s.fov_size, 128..=1080).text("FOV box size (px)"),
                    );
                    egui::ComboBox::from_label("Target")
                        .selected_text(match self.s.target_mode {
                            TargetMode::Cursor => "Nearest to cursor",
                            TargetMode::Center => "Nearest to screen center",
                            TargetMode::Score => "Highest score",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.s.target_mode, TargetMode::Cursor, "Nearest to cursor");
                            ui.selectable_value(&mut self.s.target_mode, TargetMode::Center, "Nearest to screen center");
                            ui.selectable_value(&mut self.s.target_mode, TargetMode::Score, "Highest score");
                        });
                });

                // ---- smoothing & movement ----
                egui::CollapsingHeader::new("Smoothing & movement").default_open(true).show(ui, |ui| {
                    ui.add(egui::Slider::new(&mut self.s.smoothing, 0.0..=0.99).text("Smoothing"));
                    ui.label("0 = snap instantly,  higher = slower/smoother");
                    ui.add(egui::Slider::new(&mut self.s.max_speed, 1.0..=300.0).text("Max speed (px/tick)"));
                    ui.add(egui::Slider::new(&mut self.s.gain, 0.1..=3.0).text("Gain"));
                    ui.add(egui::Slider::new(&mut self.s.deadzone, 0.0..=30.0).text("Deadzone (px)"));
                    ui.add(egui::Slider::new(&mut self.s.target_ema, 0.0..=0.95).text("Target jitter filter"));
                    ui.add(egui::Slider::new(&mut self.s.tick_hz, 30..=500).text("Tick rate (Hz)"));
                });

                // ---- status & preview ----
                egui::CollapsingHeader::new("Status").default_open(true).show(ui, |ui| {
                    ui.label(format!(
                        "FPS: {:.0}   Detections: {}   Mover active: {}",
                        status.fps, status.det_count, if status.active { "YES" } else { "no" }
                    ));
                    ui.label(&status.message);
                    ui.checkbox(&mut self.preview_enabled, "Show preview");
                    if self.preview_enabled {
                        if let Some(tex) = &self.tex {
                            ui.image(egui::load::SizedTexture::new(tex.id(), tex.size_vec2()));
                        }
                    }
                });
            });
        });

        // push GUI state -> shared
        *self.shared.settings.lock().unwrap() = self.s.clone();
        self.shared.mover_enabled.store(self.mover_enabled, Ordering::Relaxed);
        self.shared.preview_enabled.store(self.preview_enabled, Ordering::Relaxed);

        ctx.request_repaint_after(Duration::from_millis(33)); // keep status/preview live
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shared.running.store(false, Ordering::Relaxed);
    }
}

fn main() -> eframe::Result<()> {
    let shared = Arc::new(Shared::new());

    let worker_shared = shared.clone();
    std::thread::spawn(move || worker::run(worker_shared));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([560.0, 940.0]),
        ..Default::default()
    };
    eframe::run_native(
        "YoloMouse (Rust) - YOLOv10 cursor mover",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(shared)))),
    )
}
