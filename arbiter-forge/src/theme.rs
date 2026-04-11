use eframe::egui;

pub struct Theme {
    pub bg_void: egui::Color32,
    pub bg_surface: egui::Color32,
    pub bg_widget: egui::Color32,
    pub bg_panel: egui::Color32,
    
    pub accent: egui::Color32,        // Cyber Blue
    pub accent_soft: egui::Color32,   // Translucent Blue
    pub success: egui::Color32,       // Emerald
    pub warning: egui::Color32,       // Amber
    pub panic: egui::Color32,         // Crimson
    
    pub text_primary: egui::Color32,
    pub text_muted: egui::Color32,
    pub border: egui::Color32,
}

impl Theme {
    pub fn arbiter_dark() -> Self {
        Self {
            bg_void: egui::Color32::from_rgb(10, 10, 14),
            bg_surface: egui::Color32::from_rgb(18, 18, 24),
            bg_widget: egui::Color32::from_rgb(28, 28, 36),
            bg_panel: egui::Color32::from_rgb(14, 14, 18),
            
            accent: egui::Color32::from_rgb(0, 157, 255),
            accent_soft: egui::Color32::from_rgba_premultiplied(0, 157, 255, 30),
            success: egui::Color32::from_rgb(0, 230, 150),
            warning: egui::Color32::from_rgb(255, 190, 0),
            panic: egui::Color32::from_rgb(255, 50, 70),
            
            text_primary: egui::Color32::from_rgb(220, 220, 230),
            text_muted: egui::Color32::from_rgb(115, 115, 130),
            border: egui::Color32::from_rgb(38, 38, 48),
        }
    }
}

pub fn configure_arbiter_style(ctx: &egui::Context) {
    let theme = Theme::arbiter_dark();
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();

    // Core Visuals
    visuals.panel_fill = theme.bg_void;
    visuals.window_fill = egui::Color32::from_rgba_premultiplied(16, 16, 20, 250);
    visuals.extreme_bg_color = egui::Color32::from_rgb(6, 6, 8);
    visuals.override_text_color = Some(theme.text_primary);
    
    visuals.window_stroke = egui::Stroke::new(1.0, theme.border);
    visuals.window_rounding = egui::Rounding::same(8.0);
    
    visuals.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 12.0),
        blur: 40.0,
        spread: 0.0,
        color: egui::Color32::from_black_alpha(120),
    };

    let rounding = egui::Rounding::same(4.0);
    
    // Widgets
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.noninteractive.bg_fill = theme.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, theme.border);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.text_muted);

    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.inactive.bg_fill = theme.bg_widget;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.border);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.text_primary);

    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(40, 40, 50);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme.accent);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.hovered.expansion = 1.0;

    visuals.widgets.active.rounding = rounding;
    visuals.widgets.active.bg_fill = theme.accent;
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

    visuals.selection.bg_fill = theme.accent_soft;
    visuals.selection.stroke = egui::Stroke::new(1.0, theme.accent);

    // Spacing
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.window_margin = egui::Margin::same(16.0);
    style.spacing.interact_size.y = 18.0; 

    // Fonts
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));

    style.visuals = visuals;
    ctx.set_style(style);
}
