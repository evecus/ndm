//! 现代化视觉主题与自绘灵动组件。
//!
//! 配色采用深蓝灰玻璃质感 + 蓝紫渐变强调色；所有渐变/流光/呼吸效果均由
//! `egui::Painter` 逐帧绘制，动画通过传入的时间戳驱动，调用方负责帧率节流。

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shadow, Stroke, TextStyle,
    Vec2,
};

// ── 色板 ────────────────────────────────────────────────────────────────────
pub mod c {
    use super::Color32;
    pub const BG: Color32 = Color32::from_rgb(13, 19, 32);
    pub const SURFACE: Color32 = Color32::from_rgb(20, 28, 46);
    pub const SURFACE_2: Color32 = Color32::from_rgb(27, 38, 61);
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(34, 48, 76);
    pub const BORDER: Color32 = Color32::from_rgb(42, 58, 90);
    pub const TEXT: Color32 = Color32::from_rgb(230, 237, 247);
    pub const MUTED: Color32 = Color32::from_rgb(138, 153, 181);
    pub const FAINT: Color32 = Color32::from_rgb(104, 120, 150);
    pub const ACCENT: Color32 = Color32::from_rgb(91, 140, 255);
    pub const ACCENT_2: Color32 = Color32::from_rgb(139, 108, 246);
    pub const TRACK: Color32 = Color32::from_rgb(38, 51, 78);
    pub const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);
    pub const WARN: Color32 = Color32::from_rgb(245, 165, 36);
    pub const ERROR: Color32 = Color32::from_rgb(248, 113, 113);
    pub const INFO: Color32 = Color32::from_rgb(56, 189, 248);
}

/// 把全局样式替换为现代深色主题。仅在初始化时调用一次。
pub fn apply_theme(ctx: &egui::Context) {
    // 锁定深色主题，不跟随 Windows 系统浅色主题。否则系统主题会把 widget
    // 渲染成浅色背景，配合白色文字就会出现「白底白字看不清」。
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = c::SURFACE;
        v.panel_fill = c::BG;
        v.faint_bg_color = c::SURFACE_2;
        v.extreme_bg_color = c::SURFACE;
        v.code_bg_color = c::SURFACE_2;
        // 不使用 override_text_color：它会把所有 widget 文字强制成同一颜色，
        // 一旦按钮背景为浅色就会白底白字。改为按状态分别设置 fg_stroke。
        v.override_text_color = None;
        v.hyperlink_color = c::ACCENT;
        v.warn_fg_color = c::WARN;
        v.error_fg_color = c::ERROR;
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_shadow = Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(170),
        };
        v.popup_shadow = v.window_shadow;
        v.button_frame = true;
        v.striped = false;
        v.widgets.noninteractive.bg_fill = c::SURFACE_2;
        v.widgets.noninteractive.fg_stroke.color = c::TEXT;
        v.widgets.inactive.bg_fill = c::SURFACE_2;
        v.widgets.inactive.fg_stroke.color = c::TEXT;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, c::BORDER);
        v.widgets.hovered.bg_fill = c::SURFACE_HOVER;
        v.widgets.hovered.fg_stroke.color = Color32::WHITE;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, c::ACCENT);
        v.widgets.active.bg_fill = c::ACCENT;
        v.widgets.active.fg_stroke.color = Color32::WHITE;
        v.widgets.open.bg_fill = c::SURFACE_HOVER;
        v.widgets.open.fg_stroke.color = Color32::WHITE;
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(91, 140, 255, 60);
        v.selection.stroke = Stroke::new(1.0, c::ACCENT);

        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.scroll = egui::style::ScrollStyle::solid();

        set_font_size(style, TextStyle::Small, 11.0);
        set_font_size(style, TextStyle::Body, 13.0);
        set_font_size(style, TextStyle::Button, 13.0);
        set_font_size(style, TextStyle::Heading, 18.0);
    });
}

fn set_font_size(style: &mut egui::Style, which: TextStyle, size: f32) {
    if let Some(id) = style.text_styles.get_mut(&which) {
        id.size = size;
    }
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgb(
        mix(a.r(), b.r(), t),
        mix(a.g(), b.g(), t),
        mix(a.b(), b.b(), t),
    )
}

/// 用逐像素竖线绘制带圆角的线性渐变矩形（egui 无原生渐变）。
pub fn gradient_rect(painter: &egui::Painter, rect: Rect, radius: f32, left: Color32, right: Color32) {
    let w = rect.width().max(1.0);
    let r = radius.min(w * 0.5).min(rect.height() * 0.5);
    let x0 = rect.left();
    let x1 = rect.right();
    let mut x = x0;
    while x < x1 {
        let t = ((x - x0) / w).clamp(0.0, 1.0);
        let col = lerp_color(left, right, t);
        let inset = if x < x0 + r {
            let dx = x0 + r - x;
            r - (r * r - dx * dx).max(0.0).sqrt()
        } else if x > x1 - r {
            let dx = x - (x1 - r);
            r - (r * r - dx * dx).max(0.0).sqrt()
        } else {
            0.0
        };
        painter.line_segment(
            [Pos2::new(x, rect.top() + inset), Pos2::new(x, rect.bottom() - inset)],
            Stroke::new(1.0, col),
        );
        x += 1.0;
    }
}

/// 渐变主操作按钮（如「新建任务」）。
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> bool {
    let font = FontId::proportional(13.0);
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE));
    let size = Vec2::new(galley.size().x + 26.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let down = resp.is_pointer_button_down_on();
    let (l, r) = if down {
        (c::ACCENT_2, c::ACCENT)
    } else if hovered {
        (Color32::from_rgb(112, 158, 255), Color32::from_rgb(155, 128, 252))
    } else {
        (c::ACCENT, c::ACCENT_2)
    };
    gradient_rect(ui.painter(), rect, 9.0, l, r);
    if hovered && !down {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0, Color32::from_white_alpha(70)),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        Color32::WHITE,
    );
    resp.clicked()
}

/// 状态胶囊：左侧小圆点 + 文字，背景为状态色的低透明度版本。
pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let font = FontId::proportional(11.0);
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE));
    let size = Vec2::new(galley.size().x + 30.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 34);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), bg);
    ui.painter().circle_filled(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        3.0,
        color,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

/// 圆角小图标块（分类/品牌标识）。
pub fn icon_tile(ui: &mut egui::Ui, glyph: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46);
    ui.painter()
        .rect_filled(rect, CornerRadius::same((size * 0.28) as u8), bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(size * 0.52),
        color,
    );
}

/// 流光进度条：渐变填充 + 随时间移动的高光，带百分比文字。
pub fn fancy_progress(ui: &mut egui::Ui, progress: f32, color: Color32, time: f64) {
    let height = 9.0;
    let percent_w = 44.0;
    let gap = 8.0;
    let bar_w = (ui.available_width() - percent_w - gap).max(20.0);
    let top = ui.cursor().min;
    let track = Rect::from_min_size(top, Vec2::new(bar_w, height));
    let resp_rect = Rect::from_min_size(top, Vec2::new(bar_w + gap + percent_w, height));
    ui.advance_cursor_after_rect(resp_rect);

    ui.painter()
        .rect_filled(track, CornerRadius::same(4), c::TRACK);
    let p = progress.clamp(0.0, 1.0);
    if p > 0.001 {
        let mut fill = track;
        fill.set_right(track.left() + track.width() * p);
        let light = lerp_color(color, Color32::WHITE, 0.18);
        gradient_rect(ui.painter(), fill, 4.0, color, light);

        // 移动高光：仅绘制在填充区域内部
        let clipped = ui.painter().with_clip_rect(fill);
        let shimmer_w = 46.0;
        let span = fill.width() + shimmer_w * 2.0;
        let phase = ((time * 0.55) % 1.0) as f32;
        let cx = fill.left() - shimmer_w + phase * span;
        for i in 0..12 {
            let t = i as f32 / 11.0;
            let alpha = (t.sin() * 60.0) as u8;
            let x = cx + t * shimmer_w;
            clipped.line_segment(
                [Pos2::new(x, fill.top()), Pos2::new(x, fill.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(alpha)),
            );
        }
    }
    let label = format!("{:.0}%", p * 100.0);
    ui.painter().text(
        Pos2::new(track.right() + gap, top.y + height * 0.5),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.0),
        c::MUTED,
    );
}

/// 带呼吸动画的连接状态灯 + 文字。
pub fn pulse_status(ui: &mut egui::Ui, connected: bool, text: &str, time: f64) {
    let color = if connected { c::SUCCESS } else { c::FAINT };
    let pulse = 0.55 + 0.45 * (time * 2.2).sin().max(0.0) as f32;
    let dot_color = if connected {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (255.0 * pulse) as u8)
    } else {
        color
    };
    let font = FontId::proportional(11.0);
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE));
    let size = Vec2::new(galley.size().x + 24.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 6.0, rect.center().y), 4.0, dot_color);
    if connected {
        ui.painter().circle_stroke(
            Pos2::new(rect.left() + 6.0, rect.center().y),
            7.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(52, 211, 153, (110.0 * pulse) as u8)),
        );
    }
    ui.painter().text(
        Pos2::new(rect.left() + 17.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        if connected { c::SUCCESS } else { c::MUTED },
    );
}

/// 空状态下的插画：脉冲圆环 + 下载箭头。
pub fn empty_illustration(ui: &mut egui::Ui, time: f64) {
    let size = 96.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let center = rect.center();
    let base = 30.0;
    let pulse = ((time * 0.9) % 1.0) as f32;
    for k in 0..2 {
        let t = (pulse + k as f32 * 0.5) % 1.0;
        let radius = base + t * 18.0;
        let alpha = ((1.0 - t) * 90.0) as u8;
        ui.painter().circle_stroke(
            center,
            radius,
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(91, 140, 255, alpha)),
        );
    }
    gradient_rect(
        ui.painter(),
        Rect::from_center_size(center, Vec2::splat(base * 1.6)),
        base * 0.8,
        Color32::from_rgba_unmultiplied(91, 140, 255, 55),
        Color32::from_rgba_unmultiplied(139, 108, 246, 55),
    );
    // 向下箭头
    ui.painter().line_segment(
        [
            Pos2::new(center.x, center.y - 13.0),
            Pos2::new(center.x, center.y + 9.0),
        ],
        Stroke::new(3.0, Color32::WHITE),
    );
    ui.painter().line_segment(
        [
            Pos2::new(center.x - 8.0, center.y + 2.0),
            Pos2::new(center.x, center.y + 10.0),
        ],
        Stroke::new(3.0, Color32::WHITE),
    );
    ui.painter().line_segment(
        [
            Pos2::new(center.x + 8.0, center.y + 2.0),
            Pos2::new(center.x, center.y + 10.0),
        ],
        Stroke::new(3.0, Color32::WHITE),
    );
    ui.painter().line_segment(
        [
            Pos2::new(center.x - 13.0, center.y + 15.0),
            Pos2::new(center.x + 13.0, center.y + 15.0),
        ],
        Stroke::new(3.0, Color32::WHITE),
    );
}

/// 侧栏导航项，返回是否被点击。
pub fn nav_item(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    count: usize,
    selected: bool,
) -> bool {
    let font = FontId::proportional(13.0);
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    });
    let width = ui.available_width();
    let size = Vec2::new(width, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    if selected {
        let bg = Color32::from_rgba_unmultiplied(91, 140, 255, 40);
        ui.painter()
            .rect_filled(rect, CornerRadius::same(9), bg);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
            CornerRadius::same(2),
            c::ACCENT,
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(9),
            Color32::from_rgba_unmultiplied(255, 255, 255, 12),
        );
    }

    let fg = if selected { Color32::WHITE } else { c::MUTED };
    ui.painter().text(
        Pos2::new(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(14.0),
        if selected { c::ACCENT } else { c::FAINT },
    );
    ui.painter().text(
        Pos2::new(rect.left() + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        fg,
    );
    let count_text = count.to_string();
    ui.painter().text(
        Pos2::new(rect.right() - 12.0, rect.center().y),
        Align2::RIGHT_CENTER,
        count_text,
        FontId::proportional(11.0),
        if selected { c::ACCENT } else { c::FAINT },
    );
    let _ = text_w;
    resp.clicked()
}

/// 状态栏统计小卡片。
pub fn stat_chip(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(118.0, 26.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), c::SURFACE_2);
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 11.0, rect.center().y), 3.0, color);
    ui.painter().text(
        Pos2::new(rect.left() + 21.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(10.5),
        c::FAINT,
    );
    ui.painter().text(
        Pos2::new(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(12.0),
        c::TEXT,
    );
}
