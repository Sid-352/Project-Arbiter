use eframe::egui;

pub struct Theme {
    pub bg_void: egui::Color32,
    pub bg_surface: egui::Color32,
    pub bg_widget: egui::Color32,
    pub bg_panel: egui::Color32,
    
    // Core industrial palette
    pub accent_primary: egui::Color32, // Cyan (Information/Core)
    pub kernel_pulse: egui::Color32,   // Neon Green (OS Triggers)
    pub action_trace: egui::Color32,   // Warning Orange (Engine Acts)
    pub panic: egui::Color32,          // Deep Red (Failures)
    
    pub text_primary: egui::Color32,
    pub text_muted: egui::Color32,
    pub border: egui::Color32,
}

impl Theme {
    pub fn industrial_dark() -> Self {
        Self {
            bg_void: egui::Color32::from_rgb(9, 9, 11),
            bg_surface: egui::Color32::from_rgb(18, 18, 20),
            bg_widget: egui::Color32::from_rgb(24, 24, 28),
            bg_panel: egui::Color32::from_rgb(14, 14, 16),
            
            accent_primary: egui::Color32::from_rgb(0, 229, 255), // Cyber Cyan
            kernel_pulse: egui::Color32::from_rgb(57, 255, 20),   // Neon Green
            action_trace: egui::Color32::from_rgb(255, 98, 0),    // Warning Orange
            panic: egui::Color32::from_rgb(255, 59, 48),          // Bright Red
            
            text_primary: egui::Color32::from_rgb(230, 230, 235),
            text_muted: egui::Color32::from_rgb(110, 110, 120),
            border: egui::Color32::from_rgb(34, 34, 40),
        }
    }
}

pub fn configure_industrial_style(ctx: &egui::Context) {
    let theme = Theme::industrial_dark();
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = theme.bg_void;
    visuals.window_fill = egui::Color32::from_rgba_premultiplied(14, 14, 16, 245);
    visuals.extreme_bg_color = theme.bg_void;
    visuals.override_text_color = Some(theme.text_primary);
    visuals.window_stroke = egui::Stroke::new(1.0, theme.border);
    visuals.window_rounding = egui::Rounding::same(0.0); // Industrial sharp edges
    
    visuals.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 24.0,
        spread: 2.0,
        color: egui::Color32::from_black_alpha(150),
    };

    let rounding = egui::Rounding::same(2.0); // very slight rounding
    
    // Non-interactive
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.noninteractive.bg_fill = theme.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.text_muted);

    // Inactive
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.inactive.bg_fill = theme.bg_widget;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.border);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.text_primary);

    // Hovered
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.hovered.bg_fill = theme.bg_widget;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme.accent_primary);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.hovered.expansion = 0.0;

    // Active
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.active.bg_fill = theme.accent_primary.gamma_multiply(0.3);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, theme.accent_primary);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

    visuals.selection.bg_fill = theme.accent_primary.gamma_multiply(0.2);
    visuals.selection.stroke = egui::Stroke::new(1.0, theme.accent_primary);

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.interact_size.y = 20.0; 

    use egui::{FontFamily, FontId, TextStyle};
    // System-native generic font sizing, favoring monospace for industrial vibe
    style.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Monospace));
    style.text_styles.insert(TextStyle::Heading, FontId::new(16.0, FontFamily::Monospace));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));

    style.visuals = visuals;
    ctx.set_style(style);
}
