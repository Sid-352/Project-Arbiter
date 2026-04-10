use eframe::egui;

use crate::theme::Theme;

#[derive(PartialEq, Eq)]
pub enum ViewMode {
    Conservatory,
    VigilFeed,
}

#[derive(Clone, Default, PartialEq)]
pub enum EditorTrigger {
    #[default]
    Hotkey,
    FileWatch,
    ProcessWatch,
}

#[derive(Clone)]
pub struct EditorAction {
    pub kind: String, // "Shell", "Move", "Delay"
    pub param1: String,
    pub param2: String,
}

pub struct TerminalApp {
    pub mode: ViewMode,
    pub heartbeat_toggle: bool,
    pub log_lines: Vec<String>,
    pub max_log_lines: usize,
    
    // Editor State
    pub active_ordinance_name: String,
    pub trigger_type: EditorTrigger,
    pub trigger_value: String,
    pub actions: Vec<EditorAction>,
}

impl Default for TerminalApp {
    fn default() -> Self {
        Self {
            mode: ViewMode::VigilFeed,
            heartbeat_toggle: false,
            log_lines: vec!["[Kernel] Awaiting transmission...".to_string()],
            max_log_lines: 500,
            
            active_ordinance_name: "Archive ZIPs".into(),
            trigger_type: EditorTrigger::FileWatch,
            trigger_value: "*.zip".into(),
            actions: vec![
                EditorAction { kind: "Move".into(), param1: "${env.file_path}".into(), param2: "C:\\Archives".into() }
            ],
        }
    }
}

impl eframe::App for TerminalApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array() // Allow frameless transparency if needed
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::theme::configure_industrial_style(ctx);
        self.handle_window_resizing(ctx);
        self.poll_logs();
        self.draw_top_bar(ctx);
        self.draw_central_panel(ctx);

        // UI refresh poll limit
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl TerminalApp {
    fn poll_logs(&mut self) {
        use std::io::Read;
        // In a true robust tail, we'd keep the file handle open and remember our offset.
        // For keeping the footprint tiny, we just read the last chunk and extract new lines if we haven't seen them.
        if let Ok(mut f) = std::fs::File::open("../doc/logs/arbiter.log") {
            let mut buffer = String::new();
            if f.read_to_string(&mut buffer).is_ok() {
                let lines: Vec<String> = buffer.lines().map(|s| s.to_string()).collect();
                let count = lines.len();
                if count > self.max_log_lines {
                    self.log_lines = lines.into_iter().skip(count - self.max_log_lines).collect();
                } else {
                    self.log_lines = lines;
                }
            }
        }
    }
    fn handle_window_resizing(&self, ctx: &egui::Context) {
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_maximized { return; }

        if let Some(p) = ctx.pointer_latest_pos() {
            let rect = ctx.screen_rect();
            let edge = 6.0;
            let left = p.x < rect.min.x + edge;
            let right = p.x > rect.max.x - edge;
            let top = p.y < rect.min.y + edge;
            let bottom = p.y > rect.max.y - edge;

            let dir = match (top, bottom, left, right) {
                (true, false, true, false) => Some(egui::ResizeDirection::NorthWest),
                (true, false, false, true) => Some(egui::ResizeDirection::NorthEast),
                (false, true, true, false) => Some(egui::ResizeDirection::SouthWest),
                (false, true, false, true) => Some(egui::ResizeDirection::SouthEast),
                (true, false, false, false) => Some(egui::ResizeDirection::North),
                (false, true, false, false) => Some(egui::ResizeDirection::South),
                (false, false, true, false) => Some(egui::ResizeDirection::West),
                (false, false, false, true) => Some(egui::ResizeDirection::East),
                _ => None,
            };

            if let Some(dir) = dir {
                ctx.set_cursor_icon(match dir {
                    egui::ResizeDirection::North | egui::ResizeDirection::South => egui::CursorIcon::ResizeVertical,
                    egui::ResizeDirection::East | egui::ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
                    egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
                    egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
                });
                if ctx.input(|i| i.pointer.primary_pressed()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                }
            }
        }
    }
    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        let theme = Theme::industrial_dark();
        let bg_topbar = theme.bg_surface;

        let frame = egui::Frame::none()
            .fill(bg_topbar)
            .inner_margin(egui::Margin::symmetric(16.0, 8.0))
            .stroke(egui::Stroke::new(1.0, theme.border));

        egui::TopBottomPanel::top("top_tab_bar")
            .frame(frame)
            .exact_height(32.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // StartDrag logic for frameless
                    let drag_rect = ui.available_rect_before_wrap();
                    let drag_resp = ui.interact(drag_rect, ui.id().with("drag"), egui::Sense::click_and_drag());
                    if drag_resp.dragged_by(egui::PointerButton::Primary) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }

                    // --- Left Side: Branding & Status ---
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("A R B I T E R")
                                .strong()
                                .size(14.0)
                                .color(theme.text_primary),
                        );
                        ui.add_space(20.0);

                        // Tabs
                        let mut tab = |mode: ViewMode, label: &str| {
                            let is_selected = self.mode == mode;
                            let text_color = if is_selected { theme.accent_primary } else { theme.text_muted };
                            let resp = ui.button(egui::RichText::new(label).color(text_color).strong());
                            if resp.clicked() {
                                self.mode = mode;
                            }
                        };

                        tab(ViewMode::VigilFeed, "Vigil Feed");
                        tab(ViewMode::Conservatory, "Conservatory");
                    });

                    // --- Right Side: Window Controls ---
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let win_btn = |ui: &mut egui::Ui, icon: &str, is_close: bool| -> bool {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 2.0, if is_close { theme.panic } else { theme.bg_widget });
                            }
                            let tc = if resp.hovered() { egui::Color32::WHITE } else { theme.text_muted };
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::monospace(14.0), tc);
                            resp.clicked()
                        };

                        if win_btn(ui, "X", true) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if win_btn(ui, "—", false) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });
    }

    fn draw_central_panel(&mut self, ctx: &egui::Context) {
        let theme = Theme::industrial_dark();
        let frame = egui::Frame::none().fill(theme.bg_void).inner_margin(16.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            match self.mode {
                ViewMode::VigilFeed => {
                    ui.heading(egui::RichText::new("The Vigil Feed / Audit Log").color(theme.accent_primary));
                    ui.add_space(8.0);
                    
                    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                        for line in &self.log_lines {
                            let (c, text) = if line.contains("WARN") || line.contains("ERROR") {
                                (theme.panic, line)
                            } else if line.contains("Inscribe") || line.contains("shell") {
                                (theme.action_trace, line)
                            } else if line.contains("Vigil") {
                                (theme.kernel_pulse, line)
                            } else {
                                (theme.text_muted, line)
                            };

                            ui.label(egui::RichText::new(text).color(c).family(egui::FontFamily::Monospace).size(12.0));
                        }
                    });
                }
                ViewMode::Conservatory => {
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("The Conservatory").color(theme.accent_primary));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(egui::RichText::new("+ New Ordinance").color(theme.kernel_pulse)).clicked() {
                                // TODO: Add ordinance
                            }
                        });
                    });
                    ui.add_space(8.0);

                    // Left Side: List | Right Side: Editor
                    egui::SidePanel::left("ordinance_list_panel")
                        .frame(egui::Frame::none().fill(theme.bg_panel).inner_margin(8.0))
                        .default_width(200.0)
                        .show_inside(ui, |ui| {
                            ui.label(egui::RichText::new("Active Ordinances").color(theme.text_muted).strong());
                            ui.separator();
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                // Mock item
                                let btn = ui.add_sized([ui.available_width(), 30.0], egui::Button::new("Archive ZIPs"));
                                if btn.hovered() {
                                    btn.on_hover_text("Trigger: FileCreated | Target: *.zip");
                                }
                            });
                        });

                    egui::CentralPanel::default()
                        .frame(egui::Frame::none().fill(theme.bg_widget).inner_margin(16.0))
                        .show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.heading(egui::RichText::new(format!("The Forge [{}]", self.active_ordinance_name)).color(theme.text_primary));
                                ui.add_space(20.0);
                                if ui.button(egui::RichText::new("💾 Save").color(theme.kernel_pulse)).clicked() {
                                    // TODO: serialize and save to Signet
                                }
                            });
                            ui.add_space(8.0);
                            
                            egui::Grid::new("trigger_grid")
                                .num_columns(2)
                                .spacing([16.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Summons Type:").color(theme.text_muted));
                                    egui::ComboBox::from_id_source("cb_trig")
                                        .selected_text(match self.trigger_type {
                                            EditorTrigger::Hotkey => "Hotkey Combo",
                                            EditorTrigger::FileWatch => "File Creation",
                                            EditorTrigger::ProcessWatch => "Process Started",
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::Hotkey, "Hotkey Combo");
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::FileWatch, "File Creation");
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::ProcessWatch, "Process Started");
                                        });
                                    ui.end_row();

                                    ui.label(egui::RichText::new("Listener Target:").color(theme.text_muted));
                                    ui.add(egui::TextEdit::singleline(&mut self.trigger_value).hint_text("e.g. *.zip or Ctrl+Shift+Y"));
                                    ui.end_row();
                                });
                            
                            ui.add_space(16.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Action Sequence").strong().color(theme.text_muted));
                                if ui.button("+ Add Action").clicked() {
                                    self.actions.push(EditorAction { kind: "Move".into(), param1: "".into(), param2: "".into() });
                                }
                            });
                            ui.separator();

                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut action_to_remove = None;
                                let mut action_to_move_up = None;

                                for (idx, action) in self.actions.iter_mut().enumerate() {
                                    egui::Frame::none().fill(theme.bg_widget.gamma_multiply(0.5)).inner_margin(8.0).show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(format!("{}.", idx + 1)).color(theme.text_muted).size(14.0));
                                            
                                            egui::ComboBox::from_id_source(format!("cb_act_{}", idx))
                                                .selected_text(&action.kind)
                                                .width(100.0)
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(&mut action.kind, "Move".to_string(), "Inscribe Move");
                                                    ui.selectable_value(&mut action.kind, "Shell".to_string(), "Shell Exec");
                                                    ui.selectable_value(&mut action.kind, "Delay".to_string(), "Time Delay");
                                                });
                                                
                                            ui.add(egui::TextEdit::singleline(&mut action.param1).desired_width(150.0).hint_text("Param 1"));
                                            
                                            if action.kind != "Delay" {
                                                ui.add(egui::TextEdit::singleline(&mut action.param2).desired_width(150.0).hint_text("Param 2"));
                                            }

                                            // Re-order and delete buttons
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button(egui::RichText::new("X").color(theme.panic)).clicked() {
                                                    action_to_remove = Some(idx);
                                                }
                                                if idx > 0 {
                                                    if ui.button("↑").clicked() {
                                                        action_to_move_up = Some(idx);
                                                    }
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(4.0);
                                }

                                if let Some(idx) = action_to_remove {
                                    self.actions.remove(idx);
                                }
                                if let Some(idx) = action_to_move_up {
                                    self.actions.swap(idx, idx - 1);
                                }
                            });
                        });
                }
            }
        });
    }
}
