//! Shared "Vault" look — the ThoughtSlayer CRT palette and chrome, ported to
//! egui 0.31. ThoughtSlayer (`rust/src/ui/theme.rs` + `ui/crt.rs`) is the
//! source of truth for every colour here; VaultForge and VaultLauncher render
//! from this one module so the tools read as a single family. Nothing in this
//! file knows about VaultForge itself — keep it that way so it can be copied.

use eframe::egui::{self, Color32};

// ── General palette ────────────────────────────────────────────────

pub const COLOR_TEXT: Color32 = Color32::from_rgb(200, 200, 200);
pub const COLOR_TEXT_DIM: Color32 = Color32::from_rgb(90, 90, 90);
/// Secondary text with the phosphor warmth (inactive tab labels).
pub const COLOR_TEXT_WARM: Color32 = Color32::from_rgb(150, 145, 125);
pub const COLOR_BG: Color32 = Color32::from_rgb(0, 0, 0);
pub const COLOR_BG_ALT: Color32 = Color32::from_rgb(10, 10, 10);
pub const COLOR_HEADER: Color32 = Color32::from_rgb(16, 16, 16);
pub const COLOR_BORDER: Color32 = Color32::from_rgb(45, 45, 45);
/// The title bar's bottom rule and other warm dividers.
pub const COLOR_BORDER_WARM: Color32 = Color32::from_rgb(50, 42, 18);
pub const COLOR_SELECTION: Color32 = Color32::from_rgb(50, 45, 25);
/// Faint separator between grid rows / sections.
pub const COLOR_RULE: Color32 = Color32::from_rgb(28, 26, 18);
/// Selected-tab / group-header band.
pub const COLOR_GROUP_BG: Color32 = Color32::from_rgb(24, 21, 11);
pub const COLOR_HOVER_BG: Color32 = Color32::from_rgb(28, 25, 15);

// ── CRT phosphor + hazard ──────────────────────────────────────────

pub const P_PHOSPHOR: Color32 = Color32::from_rgb(220, 190, 90);
pub const P_SCAN_ALPHA: u8 = 28;
pub const HAZARD_YELLOW: Color32 = Color32::from_rgb(195, 165, 45);
pub const HAZARD_DARK: Color32 = Color32::from_rgb(30, 25, 12);
pub const COLOR_GOLD: Color32 = Color32::from_rgb(200, 170, 60);
#[allow(dead_code)] // palette constant, kept for parity with ThoughtSlayer
pub const COLOR_GOLD_DIM: Color32 = Color32::from_rgb(100, 85, 35);
pub const COLOR_GOLD_BRIGHT: Color32 = Color32::from_rgb(240, 210, 100);

// ── Meaning ────────────────────────────────────────────────────────

/// Good outcome — ThoughtSlayer's evidence-positive green, earned not default.
pub const EVIDENCE_POS: Color32 = Color32::from_rgb(120, 196, 128);
pub const EVIDENCE_NEG: Color32 = Color32::from_rgb(232, 112, 90);
pub const COLOR_FOE: Color32 = Color32::from_rgb(255, 180, 180);
pub const COLOR_FOE_BG: Color32 = Color32::from_rgb(38, 10, 10);
pub const COLOR_LISTEN: Color32 = Color32::from_rgb(110, 170, 190);
pub const COLOR_ROUTINE: Color32 = Color32::from_rgb(150, 144, 118);
pub const SEV_MEDIUM: Color32 = Color32::from_rgb(255, 210, 90);
pub const SEV_MEDIUM_BG: Color32 = Color32::from_rgb(52, 42, 16);
pub const SEV_HIGH: Color32 = Color32::from_rgb(255, 140, 60);
pub const SEV_CRITICAL: Color32 = Color32::from_rgb(255, 60, 60);

// ── Type scale ─────────────────────────────────────────────────────
// Nothing user-facing goes below FS_XS: 7–8px monospace is unreadable at 100%.

pub const FS_XS: f32 = 9.0;
pub const FS_SM: f32 = 10.0;
pub const FS_MD: f32 = 11.5;
pub const FS_LG: f32 = 13.0;

pub const TITLE_BAR_HEIGHT: f32 = 64.0;

/// Configure egui for the CRT look: black panels, amber interaction states and
/// a monospace face for everything (terminal feel, as in ThoughtSlayer).
pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(COLOR_TEXT);
    visuals.panel_fill = COLOR_BG;
    visuals.window_fill = COLOR_BG_ALT;
    visuals.extreme_bg_color = COLOR_BG;
    visuals.faint_bg_color = COLOR_BG_ALT;
    visuals.code_bg_color = COLOR_HEADER;
    visuals.hyperlink_color = COLOR_GOLD_BRIGHT;

    let warm_border = egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 50, 35));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(12, 12, 12);
    visuals.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(12, 12, 12);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, COLOR_RULE);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT_DIM);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(12, 12, 12);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(18, 18, 18);
    visuals.widgets.inactive.bg_stroke = warm_border;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT);
    visuals.widgets.hovered.bg_fill = COLOR_HOVER_BG;
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(45, 40, 25);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(90, 80, 45));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(210, 190, 120));
    visuals.widgets.active.bg_fill = Color32::from_rgb(30, 27, 14);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(30, 27, 14);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, COLOR_GOLD);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(230, 210, 130));
    visuals.widgets.open = visuals.widgets.active;

    visuals.selection.bg_fill = COLOR_SELECTION;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(200, 175, 80));
    visuals.window_stroke = egui::Stroke::new(1.0_f32, COLOR_BORDER);
    visuals.text_cursor.stroke = egui::Stroke::new(2.0_f32, COLOR_GOLD_BRIGHT);

    // Squarer than stock egui: ThoughtSlayer's controls are 3px-rounded.
    let radius = egui::CornerRadius::same(3);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.corner_radius = radius;
    visuals.window_corner_radius = egui::CornerRadius::same(4);
    visuals.menu_corner_radius = radius;
    ctx.set_visuals(visuals);

    // Monospace for the proportional family too: the whole UI reads as a terminal.
    let mut fonts = egui::FontDefinitions::default();
    if let Some(mono) = fonts.families.get(&egui::FontFamily::Monospace).cloned() {
        fonts.families.insert(egui::FontFamily::Proportional, mono);
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    for (text_style, font) in style.text_styles.iter_mut() {
        font.size = match text_style {
            egui::TextStyle::Heading => 16.0,
            egui::TextStyle::Small => FS_XS,
            egui::TextStyle::Button => FS_MD,
            _ => FS_MD + 0.5,
        };
    }
    ctx.set_style(style);
}

/// Tint `color` down to a dark badge background (ThoughtSlayer's pills).
pub fn tint_bg(color: Color32) -> Color32 {
    let scale = |channel: u8| (f32::from(channel) * 0.16) as u8;
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}

/// A flat tab with a gold underline on the active tab and an optional count
/// badge. One look for every tab/filter row.
pub fn tab_button(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    badge: Option<(String, Color32)>,
) -> egui::Response {
    let font = egui::FontId::monospace(FS_SM + 0.5);
    let badge_font = egui::FontId::monospace(FS_XS);
    let text_col = if selected { COLOR_GOLD_BRIGHT } else { COLOR_TEXT_WARM };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text_col);
    let badge_galley = badge
        .as_ref()
        .map(|(text, color)| ui.painter().layout_no_wrap(text.clone(), badge_font.clone(), *color));
    let pad_x = 10.0;
    let badge_w = badge_galley.as_ref().map_or(0.0, |galley| galley.size().x + 10.0);
    let size = egui::vec2(galley.size().x + badge_w + pad_x * 2.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        let top_rounded = egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 };
        if selected {
            painter.rect_filled(rect, top_rounded, COLOR_GROUP_BG);
        } else if hovered {
            painter.rect_filled(rect, top_rounded, Color32::from_rgb(16, 15, 10));
        }
        let text_col = if hovered && !selected { COLOR_TEXT } else { text_col };
        let text_y = rect.center().y - galley.size().y / 2.0 - 1.0;
        painter.galley_with_override_text_color(egui::pos2(rect.left() + pad_x, text_y), galley.clone(), text_col);
        if let (Some(badge_galley), Some((_, badge_color))) = (badge_galley, badge.as_ref()) {
            let badge_x = rect.left() + pad_x + galley.size().x + 6.0;
            let badge_rect = egui::Rect::from_min_size(
                egui::pos2(badge_x, rect.center().y - badge_galley.size().y / 2.0 - 2.0),
                egui::vec2(badge_galley.size().x + 6.0, badge_galley.size().y + 2.0),
            );
            painter.rect_filled(badge_rect, 6.0, tint_bg(*badge_color));
            painter.galley(egui::pos2(badge_rect.left() + 3.0, badge_rect.top() + 1.0), badge_galley, *badge_color);
        }
        let underline = if selected {
            Some((COLOR_GOLD_BRIGHT, 2.0_f32))
        } else if hovered {
            Some((Color32::from_rgb(90, 80, 45), 1.0_f32))
        } else {
            None
        };
        if let Some((color, width)) = underline {
            painter.hline(
                rect.left() + 2.0..=rect.right() - 2.0,
                rect.bottom() - 1.0,
                egui::Stroke::new(width, color),
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Dim hint text for `TextEdit::hint_text` — the default hint picks up
/// `override_text_color` and reads like real input.
pub fn hint(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .monospace()
        .size(FS_SM + 0.5)
        .color(Color32::from_rgb(70, 68, 58))
}

/// Small gold section caption ("CATALOG", "TERMINAL").
pub fn section_label(title: &str) -> egui::RichText {
    egui::RichText::new(title)
        .monospace()
        .size(FS_SM + 0.5)
        .strong()
        .color(COLOR_GOLD)
}

// ── CRT overlays ───────────────────────────────────────────────────

/// Scanline stripes + an edge vignette over `rect`, each as a single mesh.
pub fn paint_scanlines(painter: &egui::Painter, rect: egui::Rect) {
    let scan_color = Color32::from_rgba_premultiplied(0, 0, 0, P_SCAN_ALPHA);
    let mut mesh = egui::Mesh::default();
    let mut y = rect.top();
    while y < rect.bottom() {
        mesh.add_colored_rect(
            egui::Rect::from_min_size(egui::pos2(rect.left(), y + 2.0), egui::vec2(rect.width(), 2.0)),
            scan_color,
        );
        y += 4.0;
    }
    painter.add(egui::Shape::mesh(mesh));

    // Ring of quads, dark outside and clear inside: a smooth falloff on all
    // four edges interpolated by the GPU.
    let inset = (rect.width().min(rect.height()) * 0.09).clamp(24.0, 90.0);
    let inner = rect.shrink(inset);
    let dark = Color32::from_rgba_premultiplied(0, 0, 0, 70);
    let mut vignette = egui::Mesh::default();
    for point in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()] {
        vignette.colored_vertex(point, dark);
    }
    for point in [inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom()] {
        vignette.colored_vertex(point, Color32::TRANSPARENT);
    }
    for k in 0..4_u32 {
        let next = (k + 1) % 4;
        vignette.add_triangle(k, next, 4 + next);
        vignette.add_triangle(k, 4 + next, 4 + k);
    }
    painter.add(egui::Shape::mesh(vignette));
}

/// Slim hazard-tape frame around the whole window, one mesh for every stripe.
pub fn paint_hazard_border(painter: &egui::Painter, rect: egui::Rect) {
    let border_w = 3.0_f32;
    let cycle = 10.0_f32;
    let stripe_w = cycle / 2.0;
    let yellow = Color32::from_rgba_unmultiplied(175, 148, 40, 140);
    let dark = Color32::from_rgba_unmultiplied(30, 25, 12, 140);

    let top = egui::Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), border_w));
    let bottom = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - border_w),
        egui::vec2(rect.width(), border_w),
    );
    let left = egui::Rect::from_min_size(rect.left_top(), egui::vec2(border_w, rect.height()));
    let right = egui::Rect::from_min_size(
        egui::pos2(rect.right() - border_w, rect.top()),
        egui::vec2(border_w, rect.height()),
    );

    let mut mesh = egui::Mesh::default();
    for edge in [top, bottom, left, right] {
        mesh.add_colored_rect(edge, dark);
    }
    let mut quad = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, d: egui::Pos2| {
        let index = mesh.vertices.len() as u32;
        for point in [a, b, c, d] {
            mesh.colored_vertex(point, yellow);
        }
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index, index + 2, index + 3);
    };
    for edge in [top, bottom] {
        let h = edge.height();
        let mut x = edge.left() - h - cycle;
        while x < edge.right() + h + cycle {
            quad(
                egui::pos2(x, edge.bottom()),
                egui::pos2(x + h, edge.top()),
                egui::pos2(x + h + stripe_w, edge.top()),
                egui::pos2(x + stripe_w, edge.bottom()),
            );
            x += cycle;
        }
    }
    for edge in [left, right] {
        let w = edge.width();
        let mut y = edge.top() - w - cycle;
        while y < edge.bottom() + w + cycle {
            quad(
                egui::pos2(edge.left(), y + w),
                egui::pos2(edge.right(), y),
                egui::pos2(edge.right(), y + stripe_w),
                egui::pos2(edge.left(), y + w + stripe_w),
            );
            y += cycle;
        }
    }
    painter.add(egui::Shape::mesh(mesh));

    painter.rect_stroke(
        rect.shrink(border_w),
        0.0,
        egui::Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(50, 42, 18, 100)),
        egui::StrokeKind::Inside,
    );
}

/// A strip of hazard tape (yellow on dark diagonals) — the edge of a banner
/// that can't wait.
pub fn paint_hazard_tape(painter: &egui::Painter, strip: egui::Rect) {
    let painter = painter.with_clip_rect(strip);
    painter.rect_filled(strip, 0.0, HAZARD_DARK);
    let h = strip.height();
    let mut mesh = egui::Mesh::default();
    let mut x = strip.left() - h;
    while x < strip.right() {
        let index = mesh.vertices.len() as u32;
        for point in [
            egui::pos2(x, strip.bottom()),
            egui::pos2(x + 6.0, strip.bottom()),
            egui::pos2(x + 6.0 + h, strip.top()),
            egui::pos2(x + h, strip.top()),
        ] {
            mesh.colored_vertex(point, HAZARD_YELLOW);
        }
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index, index + 2, index + 3);
        x += 12.0;
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Segmented phosphor progress bar: lit amber cells for the done fraction, a
/// brighter band sweeping through them while work is in flight, and the label
/// centred on top with a shadow so it reads over lit and unlit cells alike.
pub fn phosphor_bar(ui: &mut egui::Ui, fraction: f32, sweep_secs: f32, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0_f32, COLOR_BORDER_WARM), egui::StrokeKind::Inside);

    let cells = rect.shrink2(egui::vec2(3.0, 4.0));
    let cell_w = 6.0_f32;
    let gap = 2.0_f32;
    let lit_until = cells.left() + cells.width() * fraction.clamp(0.0, 1.0);
    let band_center = cells.left() + (sweep_secs * 160.0) % (cells.width() + 120.0) - 60.0;
    let mut mesh = egui::Mesh::default();
    let mut x = cells.left();
    while x + cell_w <= cells.right() {
        let cell = egui::Rect::from_min_size(egui::pos2(x, cells.top()), egui::vec2(cell_w, cells.height()));
        let color = if x + cell_w <= lit_until {
            let near_band = (x + cell_w / 2.0 - band_center).abs() < 30.0;
            if near_band { COLOR_GOLD_BRIGHT } else { COLOR_GOLD }
        } else {
            Color32::from_rgb(26, 23, 12)
        };
        mesh.add_colored_rect(cell, color);
        x += cell_w + gap;
    }
    painter.add(egui::Shape::mesh(mesh));

    let font = egui::FontId::monospace(FS_MD);
    painter.text(rect.center() + egui::vec2(1.0, 1.0), egui::Align2::CENTER_CENTER, text, font.clone(), Color32::BLACK);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, Color32::WHITE);
}

// ── Borderless window chrome ───────────────────────────────────────

/// One label/value readout on the right of the title bar.
pub struct Readout<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub color: Color32,
}

/// Phosphor title bar: glowing title, sub-label, app icon, live readouts and
/// the window buttons. Also owns window dragging and double-click maximize.
pub struct TitleBar {
    glow_phase: f64,
    /// Last input (egui time): the glow breathes only while you're at the
    /// window and holds still otherwise — no frames spent on it when idle.
    last_active: f64,
    /// Title icon texture and the UV rect of its visible (non-transparent) part.
    icon: Option<(egui::TextureHandle, egui::Rect)>,
}

/// UV rectangle covering the non-transparent pixels of `image`, so an icon
/// drawn with padding baked into its canvas can be fitted by its real shape.
fn opaque_uv(image: &egui::ColorImage) -> egui::Rect {
    let [width, height] = image.size;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if image.pixels[y * width + x].a() > 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x + 1);
                max_y = max_y.max(y + 1);
            }
        }
    }
    if min_x >= max_x || min_y >= max_y {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    egui::Rect::from_min_max(
        egui::pos2(min_x as f32 / width as f32, min_y as f32 / height as f32),
        egui::pos2(max_x as f32 / width as f32, max_y as f32 / height as f32),
    )
}

/// How long after the last input the title glow keeps breathing.
const BREATHE_SECS: f64 = 8.0;

impl TitleBar {
    pub fn new() -> Self {
        Self {
            glow_phase: 0.0,
            last_active: f64::NEG_INFINITY,
            icon: None,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        subtitle: &str,
        icon: impl FnOnce() -> egui::ColorImage,
        readouts: &[Readout<'_>],
    ) {
        let ctx = ui.ctx().clone();
        let (time, active) = ctx.input(|input| {
            (
                input.time,
                input.pointer.any_down()
                    || input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::PointerMoved(_)
                                | egui::Event::PointerButton { .. }
                                | egui::Event::MouseWheel { .. }
                                | egui::Event::Key { .. }
                                | egui::Event::Text(_)
                        )
                    }),
            )
        });
        if active {
            self.last_active = time;
        }
        let breathing = time - self.last_active < BREATHE_SECS;
        if breathing {
            self.glow_phase = (time * 1.6) % std::f64::consts::TAU;
        }

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TITLE_BAR_HEIGHT),
            egui::Sense::click_and_drag(),
        );

        let btn_w = 36.0_f32;
        let btn_h = 26.0_f32;
        let btn_gap = 2.0_f32;
        let btn_y = rect.top() + (TITLE_BAR_HEIGHT - btn_h) / 2.0;
        let button_zone_left = rect.right() - 3.0 * (btn_w + btn_gap) - 8.0;
        let in_button_zone = ctx
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|pos| pos.x >= button_zone_left && rect.contains(pos));
        if !in_button_zone {
            if response.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if response.double_clicked() {
                let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(12, 12, 12));
        painter.hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0_f32, P_PHOSPHOR));
        painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0_f32, COLOR_BORDER_WARM));

        // Title: warm halo, tighter inner glow, crisp core (ThoughtSlayer).
        let glow_t = (0.85 + 0.15 * self.glow_phase.sin() as f32).clamp(0.0, 1.0);
        let title_font = egui::FontId::monospace(22.0);
        let tx = rect.left() + 18.0;
        let ty = rect.center().y - 5.0;
        let halo = Color32::from_rgba_unmultiplied(195, 165, 45, (45.0 * glow_t) as u8);
        for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0)] {
            painter.text(egui::pos2(tx + dx, ty + dy), egui::Align2::LEFT_CENTER, title, title_font.clone(), halo);
        }
        let inner = Color32::from_rgba_unmultiplied(220, 195, 70, (100.0 * glow_t) as u8);
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            painter.text(egui::pos2(tx + dx, ty + dy), egui::Align2::LEFT_CENTER, title, title_font.clone(), inner);
        }
        let title_rect = painter.text(
            egui::pos2(tx, ty),
            egui::Align2::LEFT_CENTER,
            title,
            title_font.clone(),
            Color32::from_rgb(240, 215, 100),
        );
        let space_w = painter
            .layout_no_wrap(" ".to_string(), title_font, Color32::WHITE)
            .size()
            .x;
        let subtitle_rect = painter.text(
            egui::pos2(tx + 2.0, ty + 18.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            egui::FontId::monospace(FS_XS),
            Color32::from_rgb(120, 100, 45),
        );

        // The icon reads as part of the title: one space after it, sized to the
        // title line and kept above the sub-label (which is usually wider and
        // runs underneath). Drawn from its opaque bounds only, so transparent
        // padding in the source image neither shrinks it nor offsets it.
        let (texture, uv) = self.icon.get_or_insert_with(|| {
            let image = icon();
            let uv = opaque_uv(&image);
            (ctx.load_texture("title_bar_icon", image, egui::TextureOptions::LINEAR), uv)
        });
        let [tex_w, tex_h] = texture.size();
        let aspect = (uv.width() * tex_w as f32) / (uv.height() * tex_h as f32).max(1.0);
        let icon_h = (subtitle_rect.top() - 3.0 - title_rect.top()).clamp(12.0, 20.0);
        let icon_top = (title_rect.center().y - icon_h / 2.0).min(subtitle_rect.top() - 3.0 - icon_h);
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(title_rect.right() + space_w, icon_top),
            egui::vec2(icon_h * aspect, icon_h),
        );
        painter.image(texture.id(), icon_rect, *uv, Color32::WHITE);

        // Readouts, right-aligned in front of the window buttons.
        let mut right = button_zone_left - 14.0;
        let min_left = icon_rect.right().max(subtitle_rect.right()) + 24.0;
        for readout in readouts.iter().rev() {
            let label = painter.layout_no_wrap(
                readout.label.to_string(),
                egui::FontId::monospace(FS_XS),
                Color32::from_rgb(120, 100, 45),
            );
            let value = painter.layout_no_wrap(
                readout.value.to_string(),
                egui::FontId::monospace(FS_LG),
                readout.color,
            );
            let width = label.size().x.max(value.size().x);
            if right - width < min_left {
                break;
            }
            let left = right - width;
            painter.galley(egui::pos2(left, rect.center().y - 15.0), label, readout.color);
            painter.galley(egui::pos2(left, rect.center().y - 2.0), value, readout.color);
            painter.vline(
                right + 8.0,
                rect.center().y - 14.0..=rect.center().y + 14.0,
                egui::Stroke::new(1.0_f32, COLOR_RULE),
            );
            right = left - 18.0;
        }

        // Window buttons: minimize, maximize/restore, close.
        let btn_border = Color32::from_rgb(55, 50, 35);
        let button_rect = |slot: f32| {
            egui::Rect::from_min_size(
                egui::pos2(rect.right() - slot * (btn_w + btn_gap) - 6.0, btn_y),
                egui::vec2(btn_w, btn_h),
            )
        };

        let min_rect = button_rect(3.0);
        let min_resp = ui.allocate_rect(min_rect, egui::Sense::click());
        let hovered = min_resp.hovered();
        painter.rect_filled(min_rect, 3.0, if hovered { Color32::from_rgb(45, 40, 25) } else { Color32::from_rgb(18, 18, 18) });
        painter.rect_stroke(min_rect, 3.0, egui::Stroke::new(1.0_f32, btn_border), egui::StrokeKind::Inside);
        painter.hline(
            min_rect.center().x - 6.0..=min_rect.center().x + 6.0,
            min_rect.center().y,
            egui::Stroke::new(1.5_f32, if hovered { COLOR_TEXT } else { COLOR_TEXT_DIM }),
        );
        if min_resp.clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        min_resp.on_hover_text("Minimize");

        let max_rect = button_rect(2.0);
        let max_resp = ui.allocate_rect(max_rect, egui::Sense::click());
        let hovered = max_resp.hovered();
        painter.rect_filled(max_rect, 3.0, if hovered { Color32::from_rgb(45, 40, 25) } else { Color32::from_rgb(18, 18, 18) });
        painter.rect_stroke(max_rect, 3.0, egui::Stroke::new(1.0_f32, btn_border), egui::StrokeKind::Inside);
        painter.rect_stroke(
            max_rect.shrink(8.0),
            1.0,
            egui::Stroke::new(1.2_f32, if hovered { COLOR_TEXT } else { COLOR_TEXT_DIM }),
            egui::StrokeKind::Inside,
        );
        if max_resp.clicked() {
            let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        max_resp.on_hover_text("Maximize / Restore");

        let close_rect = button_rect(1.0);
        let close_resp = ui.allocate_rect(close_rect, egui::Sense::click());
        let hovered = close_resp.hovered();
        painter.rect_filled(close_rect, 3.0, if hovered { Color32::from_rgb(140, 35, 35) } else { Color32::from_rgb(18, 18, 18) });
        painter.rect_stroke(
            close_rect,
            3.0,
            egui::Stroke::new(1.0_f32, if hovered { Color32::from_rgb(180, 60, 60) } else { btn_border }),
            egui::StrokeKind::Inside,
        );
        let center = close_rect.center();
        let d = 5.0_f32;
        let cross = egui::Stroke::new(1.5_f32, if hovered { Color32::WHITE } else { COLOR_FOE });
        painter.line_segment([egui::pos2(center.x - d, center.y - d), egui::pos2(center.x + d, center.y + d)], cross);
        painter.line_segment([egui::pos2(center.x + d, center.y - d), egui::pos2(center.x - d, center.y + d)], cross);
        if close_resp.clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        close_resp.on_hover_text("Close");

        if breathing {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }
}

/// Edge/corner resizing for the borderless window: a thin grab band on every
/// edge shows the resize cursor and hands the drag to the OS. Call it after
/// the panels so its cursor wins over theirs.
pub fn handle_window_resize(ctx: &egui::Context) {
    const GRAB: f32 = 5.0;
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    if maximized {
        return;
    }
    let Some(pos) = ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let rect = ctx.screen_rect();
    let west = pos.x - rect.left() <= GRAB;
    let east = rect.right() - pos.x <= GRAB;
    let north = pos.y - rect.top() <= GRAB;
    let south = rect.bottom() - pos.y <= GRAB;
    let (direction, cursor) = match (north, south, west, east) {
        (true, _, true, _) => (egui::ResizeDirection::NorthWest, egui::CursorIcon::ResizeNorthWest),
        (true, _, _, true) => (egui::ResizeDirection::NorthEast, egui::CursorIcon::ResizeNorthEast),
        (_, true, true, _) => (egui::ResizeDirection::SouthWest, egui::CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (egui::ResizeDirection::SouthEast, egui::CursorIcon::ResizeSouthEast),
        (true, _, _, _) => (egui::ResizeDirection::North, egui::CursorIcon::ResizeNorth),
        (_, true, _, _) => (egui::ResizeDirection::South, egui::CursorIcon::ResizeSouth),
        (_, _, true, _) => (egui::ResizeDirection::West, egui::CursorIcon::ResizeWest),
        (_, _, _, true) => (egui::ResizeDirection::East, egui::CursorIcon::ResizeEast),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|input| input.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::opaque_uv;
    use eframe::egui;

    #[test]
    fn opaque_uv_crops_transparent_padding() {
        // 4x4 canvas with a 2x1 opaque block at (1,2)-(2,2).
        let mut image = egui::ColorImage::new([4, 4], egui::Color32::TRANSPARENT);
        image.pixels[2 * 4 + 1] = egui::Color32::WHITE;
        image.pixels[2 * 4 + 2] = egui::Color32::WHITE;
        let uv = opaque_uv(&image);
        assert_eq!(uv.min, egui::pos2(0.25, 0.5));
        assert_eq!(uv.max, egui::pos2(0.75, 0.75));

        let empty = egui::ColorImage::new([4, 4], egui::Color32::TRANSPARENT);
        assert_eq!(opaque_uv(&empty), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)));
    }
}
