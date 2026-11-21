use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use std::path::PathBuf;
use std::time::Duration;
use wasapi::Direction;

use crate::audio;
use crate::engine::{AudioEngine, EngineConfig};

pub fn run_gui() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 680.0])
            .with_min_inner_size([380.0, 420.0])
            .with_title("GhostMic — Stereo Echo Canceller"),
        ..Default::default()
    };

    eframe::run_native(
        "Seiren AEC",
        options,
        Box::new(|cc| {
            setup_fonts_and_style(&cc.egui_ctx);
            Ok(Box::new(SeirenAecApp::new()))
        }),
    )
}

fn setup_fonts_and_style(ctx: &egui::Context) {
    // Load Windows Segoe UI for full, crisp Vietnamese diacritics support
    let font_path = PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf");
    if let Ok(font_bytes) = std::fs::read(&font_path) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "segoe_ui".to_owned(),
            egui::FontData::from_owned(font_bytes).into(),
        );
        if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            prop.insert(0, "segoe_ui".to_owned());
        }
        ctx.set_fonts(fonts);
    }

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(15, 18, 25);
    visuals.window_fill = Color32::from_rgb(15, 18, 25);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 29, 40);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 38, 52);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 51, 69);
    visuals.widgets.active.bg_fill = Color32::from_rgb(52, 64, 86);
    visuals.selection.bg_fill = Color32::from_rgb(16, 185, 129);
    ctx.set_visuals(visuals);
}

struct SeirenAecApp {
    engine: AudioEngine,
    render_devices: Vec<String>,
    capture_devices: Vec<String>,
    selected_speaker: String,
    selected_mic: String,
    selected_output: String,
    has_vb_cable: bool,

    enable_rnnoise: bool,
    enable_webrtc_ns: bool,
    enable_agc: bool,
    delay_ms: i32,

    // Smoothed VU meter display values [0.0..1.0]
    smooth_spk: f32,
    smooth_raw: f32,
    smooth_clean: f32,
}

impl SeirenAecApp {
    fn new() -> Self {
        let mut app = Self {
            engine: AudioEngine::new(),
            render_devices: Vec::new(),
            capture_devices: Vec::new(),
            selected_speaker: String::new(),
            selected_mic: String::new(),
            selected_output: String::new(),
            has_vb_cable: false,
            enable_rnnoise: false,
            enable_webrtc_ns: false,
            enable_agc: false,
            delay_ms: 40,
            smooth_spk: 0.0,
            smooth_raw: 0.0,
            smooth_clean: 0.0,
        };
        app.refresh_devices();
        app
    }

    fn refresh_devices(&mut self) {
        if let Ok((renders, default_render)) = audio::list_active_devices(Direction::Render) {
            self.render_devices = renders;
            if self.selected_speaker.is_empty()
                || !self.render_devices.contains(&self.selected_speaker)
            {
                self.selected_speaker = default_render
                    .filter(|d| !d.to_lowercase().contains("cable input"))
                    .or_else(|| self.render_devices.first().cloned())
                    .unwrap_or_default();
            }

            let vb = self
                .render_devices
                .iter()
                .find(|d| d.to_lowercase().contains("cable input"))
                .cloned();
            self.has_vb_cable = vb.is_some();
            if let Some(cable) = vb {
                self.selected_output = cable;
            } else if !self.render_devices.contains(&self.selected_output) {
                self.selected_output.clear();
            }
        }

        if let Ok((captures, default_capture)) = audio::list_active_devices(Direction::Capture) {
            self.capture_devices = captures;
            if self.selected_mic.is_empty() || !self.capture_devices.contains(&self.selected_mic) {
                self.selected_mic = default_capture
                    .or_else(|| self.capture_devices.first().cloned())
                    .unwrap_or_default();
            }
        }
    }

    fn start_live(&mut self) {
        if self.selected_output.is_empty() {
            self.engine.meters.set_error(
                "Chưa có cổng Mic ảo (CABLE Input). Hãy cài VB-Audio Virtual Cable hoặc dùng nút Ghi thử 10s.",
            );
            return;
        }
        let cfg = EngineConfig {
            speaker: Some(self.selected_speaker.clone()),
            mic: Some(self.selected_mic.clone()),
            output: Some(self.selected_output.clone()),
            enable_webrtc_ns: self.enable_webrtc_ns,
            enable_agc: self.enable_agc,
            enable_rnnoise: self.enable_rnnoise,
            delay_ms: self.delay_ms,
            test_wav: None,
            test_seconds: 0,
        };
        if let Err(e) = self.engine.start(cfg) {
            self.engine.meters.set_error(format!("{e:#}"));
        }
    }

    fn start_test_wav(&mut self) {
        let desktop = std::env::var("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Desktop").join("seiren_test.wav"))
            .unwrap_or_else(|_| PathBuf::from("seiren_test.wav"));

        let cfg = EngineConfig {
            speaker: Some(self.selected_speaker.clone()),
            mic: Some(self.selected_mic.clone()),
            output: None,
            enable_webrtc_ns: self.enable_webrtc_ns,
            enable_agc: self.enable_agc,
            enable_rnnoise: self.enable_rnnoise,
            delay_ms: self.delay_ms,
            test_wav: Some(desktop),
            test_seconds: 10,
        };
        if let Err(e) = self.engine.start(cfg) {
            self.engine.meters.set_error(format!("{e:#}"));
        }
    }
}

/// Convert linear RMS [0..1] to perceptual bar fraction [0..1] (-60 dB .. 0 dB)
fn rms_to_meter(rms: f32) -> f32 {
    if rms <= 1e-4 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 55.0) / 55.0).clamp(0.0, 1.0)
}

fn draw_vu_bar(ui: &mut egui::Ui, label: &str, value: f32, fill_color: Color32) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [115.0, 16.0],
            egui::Label::new(RichText::new(label).size(12.5).color(Color32::from_rgb(190, 200, 215))),
        );
        let desired_size = Vec2::new(ui.available_width(), 12.0);
        let (rect, _resp) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(30, 36, 48));
        if value > 0.005 {
            let mut fill_rect = rect;
            fill_rect.max.x = rect.min.x + rect.width() * value.clamp(0.0, 1.0);
            painter.rect_filled(fill_rect, CornerRadius::same(4), fill_color);
        }
    });
}

fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(Color32::from_rgb(22, 27, 38))
        .stroke(Stroke::new(1.0, Color32::from_rgb(40, 48, 66)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
}

impl eframe::App for SeirenAecApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let running = self.engine.is_running();

        // Repaint at ~25 FPS only while audio is active (0% idle CPU/GPU when stopped!)
        if running {
            let (spk, raw, clean) = self.engine.meters.levels();
            let alpha = 0.35;
            self.smooth_spk = self.smooth_spk * (1.0 - alpha) + rms_to_meter(spk) * alpha;
            self.smooth_raw = self.smooth_raw * (1.0 - alpha) + rms_to_meter(raw) * alpha;
            self.smooth_clean = self.smooth_clean * (1.0 - alpha) + rms_to_meter(clean) * alpha;
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        } else {
            self.smooth_spk = 0.0;
            self.smooth_raw = 0.0;
            self.smooth_clean = 0.0;
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(15, 18, 25))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                // Header
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("GHOSTMIC")
                                .size(22.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new("Triệt tiếng loa lọt vào Mic — Stereo WebRTC AEC3")
                                .size(12.0)
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (badge_text, badge_color) = if running {
                            ("● ĐANG CHẠY", Color32::from_rgb(16, 185, 129))
                        } else {
                            ("○ ĐÃ DỪNG", Color32::from_rgb(100, 116, 139))
                        };
                        egui::Frame::NONE
                            .fill(badge_color.gamma_multiply(0.18))
                            .stroke(Stroke::new(1.0, badge_color))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(egui::Margin::symmetric(10, 4))
                            .show(ui, |ui| {
                                ui.label(RichText::new(badge_text).size(11.5).strong().color(badge_color));
                            });
                    });
                });

                ui.add_space(12.0);

                // Card 1: Audio Devices
                card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Thiết bị âm thanh").size(14.0).strong().color(Color32::WHITE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(!running, egui::Button::new("Làm mới"))
                                .clicked()
                            {
                                self.refresh_devices();
                            }
                        });
                    });
                    ui.add_space(8.0);

                    ui.add_enabled_ui(!running, |ui| {
                        let combo_w = ui.available_width();

                        ui.label(RichText::new("Loa tham chiếu (Loopback Stereo)").size(12.0).color(Color32::from_rgb(148, 163, 184)));
                        egui::ComboBox::from_id_salt("speaker_combo")
                            .width(combo_w)
                            .selected_text(&self.selected_speaker)
                            .show_ui(ui, |ui| {
                                for dev in &self.render_devices {
                                    ui.selectable_value(&mut self.selected_speaker, dev.clone(), dev);
                                }
                            });

                        ui.add_space(6.0);
                        ui.label(RichText::new("Micro thu vào (Razer Seiren)").size(12.0).color(Color32::from_rgb(148, 163, 184)));
                        egui::ComboBox::from_id_salt("mic_combo")
                            .width(combo_w)
                            .selected_text(&self.selected_mic)
                            .show_ui(ui, |ui| {
                                for dev in &self.capture_devices {
                                    ui.selectable_value(&mut self.selected_mic, dev.clone(), dev);
                                }
                            });

                        ui.add_space(6.0);
                        ui.label(RichText::new("Mic ảo đầu ra (VB-Cable Input -> Discord)").size(12.0).color(Color32::from_rgb(148, 163, 184)));
                        let out_preview = if self.selected_output.is_empty() {
                            "(Chưa cài VB-Audio Virtual Cable)".to_string()
                        } else {
                            self.selected_output.clone()
                        };
                        egui::ComboBox::from_id_salt("output_combo")
                            .width(combo_w)
                            .selected_text(out_preview)
                            .show_ui(ui, |ui| {
                                for dev in &self.render_devices {
                                    ui.selectable_value(&mut self.selected_output, dev.clone(), dev);
                                }
                            });
                    });

                    if !self.has_vb_cable {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("[!] Chưa thấy 'CABLE Input'. Tải VB-Cable tại vb-audio.com/Cable để nối vào Discord.")
                                .size(11.5)
                                .color(Color32::from_rgb(250, 204, 21)),
                        );
                    }
                });

                ui.add_space(10.0);

                // Card 2: Live Audio Meters
                card_frame().show(ui, |ui| {
                    ui.label(RichText::new("Mức tín hiệu thời gian thực").size(14.0).strong().color(Color32::WHITE));
                    ui.add_space(8.0);
                    draw_vu_bar(ui, "Loa (Loopback)", self.smooth_spk, Color32::from_rgb(56, 189, 248));
                    ui.add_space(5.0);
                    draw_vu_bar(ui, "Mic gốc (Raw)", self.smooth_raw, Color32::from_rgb(251, 191, 36));
                    ui.add_space(5.0);
                    draw_vu_bar(ui, "Mic đã lọc (AEC)", self.smooth_clean, Color32::from_rgb(16, 185, 129));
                });

                ui.add_space(10.0);

                // Card 3: DSP Tuning
                card_frame().show(ui, |ui| {
                    ui.label(RichText::new("Tùy chỉnh bộ lọc (DSP)").size(14.0).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!running, |ui| {
                        ui.checkbox(&mut self.enable_rnnoise, "Bật RNNoise (Lọc thêm tiếng gõ phím cơ / quạt bằng AI)");
                        ui.checkbox(&mut self.enable_webrtc_ns, "Bật WebRTC Noise Suppression (khuyên tắt để giữ giọng tự nhiên)");
                        ui.checkbox(&mut self.enable_agc, "Bật AGC2 (Tự cân bằng âm lượng mic)");
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Độ trễ ban đầu (ms):").size(12.5));
                            ui.add(egui::Slider::new(&mut self.delay_ms, 0..=150).suffix(" ms"));
                        });
                    });
                });

                ui.add_space(14.0);

                // Action Buttons
                ui.horizontal(|ui| {
                    let btn_w = (ui.available_width() - 10.0) * 0.62;
                    if running {
                        let stop_btn = egui::Button::new(
                            RichText::new("DỪNG LỌC").size(14.5).strong().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(220, 38, 38))
                        .min_size(Vec2::new(btn_w, 40.0));
                        if ui.add(stop_btn).clicked() {
                            self.engine.stop();
                        }
                    } else {
                        let start_btn = egui::Button::new(
                            RichText::new("BẮT ĐẦU LỌC").size(14.5).strong().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(16, 185, 129))
                        .min_size(Vec2::new(btn_w, 40.0));
                        if ui.add(start_btn).clicked() {
                            self.start_live();
                        }
                    }

                    let test_btn = egui::Button::new(
                        RichText::new("Ghi thử 10s").size(13.5).color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(51, 65, 85))
                    .min_size(Vec2::new(ui.available_width(), 40.0));
                    if ui.add_enabled(!running, test_btn).clicked() {
                        self.start_test_wav();
                    }
                });

                ui.add_space(10.0);

                // Status & Error Footer
                if let Ok(err_guard) = self.engine.meters.last_error.lock() {
                    if let Some(ref err) = *err_guard {
                        ui.label(
                            RichText::new(format!("Lỗi: {err}"))
                                .size(12.0)
                                .color(Color32::from_rgb(248, 113, 113)),
                        );
                    }
                }
                if let Ok(status) = self.engine.meters.status_text.lock() {
                    ui.label(
                        RichText::new(format!("Trạng thái: {status}"))
                            .size(12.0)
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                }
                    });
            });
    }
}
