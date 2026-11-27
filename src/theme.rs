//! 视觉主题：深蓝灰玻璃质感 + 蓝紫渐变强调色。
//!
//! egui 版需要用逐像素竖线手绘渐变、每帧手动推进动画时间；GPUI 原生支持
//! `linear_gradient` 背景与 `with_animation`，这里只保留色板与少量辅助函数。

use gpui::{linear_color_stop, linear_gradient, App, Background, Rgba};
use gpui_component::{Theme, ThemeMode};

pub const fn rgb8(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// 同一颜色换透明度（0.0 ~ 1.0）。
pub const fn alpha(c: Rgba, a: f32) -> Rgba {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a,
    }
}

// ── 色板 ────────────────────────────────────────────────────────────────────
pub mod c {
    use super::{rgb8, Rgba};
    pub const BG: Rgba = rgb8(13, 19, 32);
    pub const SURFACE: Rgba = rgb8(20, 28, 46);
    pub const SURFACE_2: Rgba = rgb8(27, 38, 61);
    pub const SURFACE_HOVER: Rgba = rgb8(34, 48, 76);
    pub const BORDER: Rgba = rgb8(42, 58, 90);
    pub const TEXT: Rgba = rgb8(230, 237, 247);
    pub const MUTED: Rgba = rgb8(138, 153, 181);
    pub const FAINT: Rgba = rgb8(104, 120, 150);
    pub const ACCENT: Rgba = rgb8(91, 140, 255);
    pub const ACCENT_2: Rgba = rgb8(139, 108, 246);
    pub const ACCENT_LIGHT: Rgba = rgb8(112, 158, 255);
    pub const ACCENT_2_LIGHT: Rgba = rgb8(155, 128, 252);
    pub const TRACK: Rgba = rgb8(38, 51, 78);
    pub const SUCCESS: Rgba = rgb8(52, 211, 153);
    pub const WARN: Rgba = rgb8(245, 165, 36);
    pub const ERROR: Rgba = rgb8(248, 113, 113);
    pub const INFO: Rgba = rgb8(56, 189, 248);
    pub const WHITE: Rgba = rgb8(255, 255, 255);
}

/// 界面字体。GPUI 走系统文本栈（DirectWrite / CoreText / fontconfig），
/// 缺字时由系统自动回退，不再需要像 egui 版那样手动读取 .ttf/.ttc。
#[cfg(windows)]
pub const UI_FONT: &str = "Microsoft YaHei UI";
#[cfg(target_os = "macos")]
pub const UI_FONT: &str = "PingFang SC";
#[cfg(not(any(windows, target_os = "macos")))]
pub const UI_FONT: &str = "Noto Sans CJK SC";

/// 从左到右的线性渐变。
pub fn gradient(left: Rgba, right: Rgba) -> Background {
    linear_gradient(
        90.0,
        linear_color_stop(left, 0.0),
        linear_color_stop(right, 1.0),
    )
}

/// 主强调渐变（新建任务、保存设置等主按钮）。
pub fn accent_gradient() -> Background {
    gradient(c::ACCENT, c::ACCENT_2)
}

/// 把颜色往白色混合 t（用于进度条渐变的亮端）。
pub fn lighten(col: Rgba, t: f32) -> Rgba {
    Rgba {
        r: col.r + (1.0 - col.r) * t,
        g: col.g + (1.0 - col.g) * t,
        b: col.b + (1.0 - col.b) * t,
        a: col.a,
    }
}

/// 锁定深色主题，不跟随系统浅色主题（gpui-component 的输入框跟着走深色）。
pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
}
