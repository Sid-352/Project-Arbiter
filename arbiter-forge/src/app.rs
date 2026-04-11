use eframe::egui;
use crate::theme::Theme;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ViewMode {
    VigilFeed,
    Conservatory,
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
    pub kind: String,
    pub param1: String,
    pub param2: String,
}

pub struct TerminalApp {
    pub mode: ViewMode,
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
            log_lines: vec!["[System] Initialising Arbiter Forge...".to_string()],
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
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::theme::configure_arbiter_style(ctx);
        self.handle_window_resizing(ctx);
        self.poll_logs();
        
        self.draw_top_bar(ctx);
        self.draw_central_panel(ctx);

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl TerminalApp {
    fn poll_logs(&mut self) {
        use std::io::Read;
        // Adjust path to root since we run from workspace root
        if let Ok(mut f) = std::fs::File::open("doc/logs/arbiter.log") {
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
            let edge = 8.0;
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
        let theme = Theme::arbiter_dark();
        
        let frame = egui::Frame::none()
            .fill(theme.bg_surface)
            .inner_margin(egui::Margin::symmetric(12.0, 0.0))
            .stroke(egui::Stroke::new(1.0, theme.border));

        egui::TopBottomPanel::top("top_bar")
            .frame(frame)
            .exact_height(40.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Drag area
                    let drag_rect = ui.available_rect_before_wrap();
                    let drag_resp = ui.interact(drag_rect, ui.id().with("drag"), egui::Sense::click_and_drag());
                    if drag_resp.dragged_by(egui::PointerButton::Primary) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }

                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        
                        // Icon/Brand
                        ui.label(egui::RichText::new("◈").color(theme.accent).size(18.0).strong());
                        ui.label(egui::RichText::new("ARBITER").color(theme.text_primary).size(14.0).strong().letter_spacing(2.0));
                        
                        ui.add_space(24.0);

                        // Tab logic
                        let mut tab = |mode: ViewMode, label: &str| {
                            let is_selected = self.mode == mode;
                            let text_color = if is_selected { egui::Color32::WHITE } else { theme.text_muted };
                            
                            let (rect, resp) = ui.allocate_at_least(egui::vec2(100.0, 40.0), egui::Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 0.0, theme.bg_widget);
                            }
                            if is_selected {
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(egui::pos2(rect.min.x, rect.max.y - 2.0), egui::vec2(rect.width(), 2.0)),
                                    0.0,
                                    theme.accent
                                );
                            }
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.0), text_color);
                            
                            if resp.clicked() {
                                self.mode = mode;
                            }
                        };

                        tab(ViewMode::VigilFeed, "Vigil Feed");
                        tab(ViewMode::Conservatory, "Conservatory");
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let btn_sz = egui::vec2(32.0, 32.0);
                        
                        let close_resp = ui.add_sized(btn_sz, egui::Button::new(egui::RichText::new("✕").size(12.0)).frame(false));
                        if close_resp.clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        
                        let min_resp = ui.add_sized(btn_sz, egui::Button::new(egui::RichText::new("—").size(12.0)).frame(false));
                        if min_resp.clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
                    });
                });
            });
    }

    fn draw_central_panel(&mut self, ctx: &egui::Context) {
        let theme = Theme::arbiter_dark();
        let frame = egui::Frame::none().fill(theme.bg_void).inner_margin(0.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            match self.mode {
                ViewMode::VigilFeed => {
                    egui::Frame::none().fill(theme.bg_panel).inner_margin(16.0).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("LIVE TELEMETRY").color(theme.text_muted).strong().size(11.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Clear Log").clicked() {
                                    self.log_lines.clear();
                                }
                            });
                        });
                        ui.separator();
                        ui.add_space(4.0);
                        
                        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for line in &self.log_lines {
                                let color = if line.contains("WARN") {
                                    theme.warning
                                } else if line.contains("ERROR") {
                                    theme.panic
                                } else if line.contains("Arbiter") || line.contains("Atlas") {
                                    theme.accent
                                } else if line.contains("Inscribe") || line.contains("shell") {
                                    theme.success
                                } else {
                                    theme.text_muted
                                };

                                ui.label(egui::RichText::new(line).color(color).family(egui::FontFamily::Monospace).size(12.0));
                            }
                        });
                    });
                }
                ViewMode::Conservatory => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        
                        // Sidebar
                        egui::SidePanel::left("conservatory_side")
                            .frame(egui::Frame::none().fill(theme.bg_panel).inner_margin(12.0).stroke(egui::Stroke::new(1.0, theme.border)))
                            .default_width(220.0)
                            .show_inside(ui, |ui| {
                                ui.label(egui::RichText::new("ORDINANCES").color(theme.text_muted).strong().size(11.0));
                                ui.add_space(8.0);
                                if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("+ NEW SEQUENCE")).clicked() {
                                    // New sequence logic
                                }
                                ui.add_space(12.0);
                                
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let mut ordinance_btn = |name: &str, active: bool| {
                                        let fill = if active { theme.accent_soft } else { egui::Color32::TRANSPARENT };
                                        let stroke = if active { egui::Stroke::new(1.0, theme.accent) } else { egui::Stroke::NONE };
                                        
                                        let resp = ui.add(egui::Button::new(name).fill(fill).stroke(stroke).min_size(egui::vec2(ui.available_width(), 32.0)));
                                        if resp.clicked() { self.active_ordinance_name = name.to_string(); }
                                    };
                                    
                                    ordinance_btn("Archive ZIPs", self.active_ordinance_name == "Archive ZIPs");
                                    ordinance_btn("Clean Desktop", self.active_ordinance_name == "Clean Desktop");
                                });
                            });

                        // Main Editor
                        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(24.0)).show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.heading(egui::RichText::new(format!("◈ {}", self.active_ordinance_name)).color(theme.text_primary));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new("💾 SAVE CHANGES").fill(theme.accent)).clicked() {
                                        // Save
                                    }
                                });
                            });
                            ui.add_space(16.0);

                            // Trigger Section
                            egui::Frame::none().fill(theme.bg_widget).rounding(6.0).inner_margin(16.0).show(ui, |ui| {
                                ui.label(egui::RichText::new("SUMMONS TRIGGER").color(theme.accent).strong().size(11.0));
                                ui.add_space(8.0);
                                
                                egui::Grid::new("trigger_grid").num_columns(2).spacing([24.0, 12.0]).show(ui, |ui| {
                                    ui.label("Trigger Type");
                                    egui::ComboBox::from_id_source("trigger_type")
                                        .selected_text(match self.trigger_type {
                                            EditorTrigger::Hotkey => "Global Hotkey",
                                            EditorTrigger::FileWatch => "File Event",
                                            EditorTrigger::ProcessWatch => "Process Monitor",
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::Hotkey, "Global Hotkey");
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::FileWatch, "File Event");
                                            ui.selectable_value(&mut self.trigger_type, EditorTrigger::ProcessWatch, "Process Monitor");
                                        });
                                    ui.end_row();

                                    ui.label("Trigger Value");
                                    ui.add(egui::TextEdit::singleline(&mut self.trigger_value).hint_text("e.g. *.zip or Ctrl+Shift+Y"));
                                    ui.end_row();
                                });
                            });

                            ui.add_space(20.0);

                            // Actions Section
                            ui.label(egui::RichText::new("ACTION SEQUENCE").color(theme.accent).strong().size(11.0));
                            ui.add_space(8.0);
                            
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut to_remove = None;
                                for (idx, action) in self.actions.iter_mut().enumerate() {
                                    egui::Frame::none()
                                        .fill(theme.bg_surface)
                                        .rounding(4.0)
                                        .stroke(egui::Stroke::new(1.0, theme.border))
                                        .inner_margin(12.0)
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new(format!("{:02}", idx + 1)).color(theme.text_muted).family(egui::FontFamily::Monospace));
                                                ui.add_space(8.0);
                                                
                                                egui::ComboBox::from_id_source(format!("act_kind_{}", idx))
                                                    .selected_text(&action.kind)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(&mut action.kind, "Move".into(), "Move File");
                                                        ui.selectable_value(&mut action.kind, "Shell".into(), "Execute Shell");
                                                        ui.selectable_value(&mut action.kind, "Delay".into(), "Wait Delay");
                                                    });

                                                ui.add(egui::TextEdit::singleline(&mut action.param1).desired_width(120.0).hint_text("Source"));
                                                if action.kind != "Delay" {
                                                    ui.add(egui::TextEdit::singleline(&mut action.param2).desired_width(120.0).hint_text("Dest / Args"));
                                                }

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.button("🗑").clicked() { to_remove = Some(idx); }
                                                });
                                            });
                                        });
                                    ui.add_space(8.0);
                                }
                                if let Some(i) = to_remove { self.actions.remove(i); }
                                
                                if ui.button("+ Add Action").clicked() {
                                    self.actions.push(EditorAction { kind: "Move".into(), param1: "".into(), param2: "".into() });
                                }
                            });
                        });
                    });
                }
            }
        });
    }
}
