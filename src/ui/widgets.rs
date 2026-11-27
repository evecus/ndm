//! 自绘组件：图标块、状态胶囊、流光进度条、呼吸状态灯、统计卡片、按钮、开关行、空状态插画。
//!
//! egui 版这些都靠 `Painter` 每帧手绘并由调用方推进时间；GPUI 里用 div + 渐变背景
//! 组合，动画用 `with_animation` 声明，由框架按显示器刷新率驱动。

use std::time::Duration;

use gpui::{
    div, prelude::*, px, relative, Animation, AnimationExt, AnyElement, Div, ElementId,
    FontWeight, Rgba, SharedString, Stateful,
};

use crate::theme::{self, alpha, c};

/// 圆角小图标块（分类/品牌标识）。
pub fn icon_tile(glyph: &'static str, color: Rgba, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.28))
        .bg(alpha(color, 0.18))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(size * 0.52))
        .text_color(color)
        .child(glyph)
}

/// 状态胶囊：左侧小圆点 + 文字，背景为状态色的低透明度版本。
pub fn status_badge(text: &'static str, color: Rgba) -> Div {
    div()
        .flex_none()
        .h(px(20.0))
        .px(px(8.0))
        .rounded(px(10.0))
        .bg(alpha(color, 0.13))
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_size(px(11.0))
        .text_color(color)
        .child(div().size(px(6.0)).rounded_full().bg(color))
        .child(text)
}

/// 流光进度条：渐变填充 + 移动高光（仅 `animate` 时），右侧百分比。
pub fn progress_bar(id: u64, progress: f32, color: Rgba, animate: bool) -> AnyElement {
    let p = progress.clamp(0.0, 1.0);

    let mut fill = div()
        .relative()
        .h_full()
        .w(relative(p))
        .rounded(px(4.0))
        .overflow_hidden()
        .bg(theme::gradient(color, theme::lighten(color, 0.18)));

    if animate && p > 0.001 {
        fill = fill.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(46.0))
                .bg(theme::gradient(alpha(c::WHITE, 0.0), alpha(c::WHITE, 0.28)))
                .with_animation(
                    ("shimmer", id as usize),
                    Animation::new(Duration::from_millis(1800)).repeat(),
                    // 高光从填充区左侧外滑到右侧外
                    |el, t| el.left(relative(-0.25 + t * 1.5)),
                ),
        );
    }

    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .h(px(9.0))
                .rounded(px(4.0))
                .bg(c::TRACK)
                .when(p > 0.001, |d| d.child(fill)),
        )
        .child(
            div()
                .flex_none()
                .w(px(40.0))
                .text_size(px(11.0))
                .text_color(c::MUTED)
                .child(format!("{:.0}%", p * 100.0)),
        )
        .into_any_element()
}

/// 带呼吸动画的连接状态灯 + 文字。
pub fn pulse_status(id: &'static str, connected: bool, text: &'static str) -> AnyElement {
    let color = if connected { c::SUCCESS } else { c::FAINT };

    let dot = div()
        .relative()
        .flex_none()
        .size(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .child(div().size(px(8.0)).rounded_full().bg(color));

    let dot: AnyElement = if connected {
        dot.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size(px(14.0))
                .rounded_full()
                .border_1()
                .border_color(alpha(c::SUCCESS, 0.45)),
        )
        .with_animation(
            id,
            Animation::new(Duration::from_millis(1400))
                .repeat()
                .with_easing(gpui::pulsating_between(0.45, 1.0)),
            |el, t| el.opacity(t),
        )
        .into_any_element()
    } else {
        dot.into_any_element()
    };

    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(11.0))
        .text_color(if connected { c::SUCCESS } else { c::MUTED })
        .child(dot)
        .child(text)
        .into_any_element()
}

/// 状态栏统计小卡片。
pub fn stat_chip(label: &'static str, value: String, color: Rgba) -> Div {
    div()
        .flex_none()
        .w(px(128.0))
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .bg(c::SURFACE_2)
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(div().size(px(6.0)).rounded_full().bg(color))
        .child(
            div()
                .text_size(px(10.5))
                .text_color(c::FAINT)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .justify_end()
                .text_size(px(12.0))
                .text_color(c::TEXT)
                .child(value),
        )
}

/// 渐变主按钮（新建任务 / 添加任务 / 保存设置）。
pub fn primary_button(id: impl Into<ElementId>, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(30.0))
        .px(px(14.0))
        .rounded(px(9.0))
        .bg(theme::accent_gradient())
        .border_1()
        .border_color(alpha(c::WHITE, 0.0))
        .hover(|s| {
            s.bg(theme::gradient(c::ACCENT_LIGHT, c::ACCENT_2_LIGHT))
                .border_color(alpha(c::WHITE, 0.28))
        })
        .active(|s| s.bg(theme::gradient(c::ACCENT_2, c::ACCENT)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c::WHITE)
        .child(text)
}

/// 普通按钮；`enabled = false` 时变暗且不响应（调用方用 `.when(enabled, ..)` 挂点击）。
pub fn button(id: impl Into<ElementId>, text: impl Into<SharedString>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .bg(c::SURFACE_2)
        .border_1()
        .border_color(c::BORDER)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.5))
        .text_color(c::TEXT)
        .when(enabled, |b| {
            b.cursor_pointer().hover(|s| {
                s.bg(c::SURFACE_HOVER)
                    .border_color(c::ACCENT)
                    .text_color(c::WHITE)
            })
        })
        .when(!enabled, |b| b.opacity(0.4))
        .child(text.into())
}

/// 任务卡片里的小按钮。
pub fn small_button(id: impl Into<ElementId>, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(22.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .bg(c::SURFACE_2)
        .flex()
        .items_center()
        .text_size(px(11.5))
        .text_color(c::MUTED)
        .cursor_pointer()
        .hover(|s| s.bg(c::SURFACE_HOVER).text_color(c::WHITE))
        .child(text)
}

/// 勾选行（替代 egui checkbox）。调用方挂 `on_click` 切换值。
pub fn toggle_row(id: impl Into<ElementId>, label: &'static str, checked: bool) -> Stateful<Div> {
    let bx = div()
        .flex_none()
        .size(px(16.0))
        .rounded(px(4.0))
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.0))
        .text_color(c::WHITE)
        .when(checked, |b| b.bg(c::ACCENT).border_color(c::ACCENT).child("✓"))
        .when(!checked, |b| b.bg(c::SURFACE_2).border_color(c::BORDER));

    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .py(px(4.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .hover(|s| s.text_color(c::WHITE))
        .child(bx)
        .child(label)
}

/// 分组标题：左侧强调竖条 + 文字。
pub fn section_title(title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .mt(px(14.0))
        .mb(px(8.0))
        .child(div().w(px(3.0)).h(px(14.0)).rounded(px(1.5)).bg(c::ACCENT))
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c::ACCENT)
                .child(title),
        )
}

/// 表单行：左侧固定宽度标签 + 右侧控件。
pub fn form_row(label: &'static str, label_w: f32, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .mb(px(10.0))
        .child(
            div()
                .flex_none()
                .w(px(label_w))
                .text_size(px(12.5))
                .text_color(c::MUTED)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
}

/// 空状态插画：两圈错相扩散的脉冲圆环 + 渐变圆底 + 下载箭头。
pub fn empty_illustration() -> AnyElement {
    const SIZE: f32 = 112.0;
    const BASE: f32 = 60.0; // 圆环起始直径
    const GROW: f32 = 36.0;

    let ring = |id: &'static str, delay_half: bool| {
        div()
            .absolute()
            .rounded_full()
            .border_2()
            .border_color(alpha(c::ACCENT, 0.35))
            .with_animation(
                id,
                Animation::new(Duration::from_millis(1100)).repeat(),
                move |el, t| {
                    let t = if delay_half { (t + 0.5) % 1.0 } else { t };
                    let d = BASE + t * GROW;
                    let off = (SIZE - d) / 2.0;
                    el.size(px(d)).left(px(off)).top(px(off)).opacity(1.0 - t)
                },
            )
    };

    let core = div()
        .absolute()
        .left(px((SIZE - 48.0) / 2.0))
        .top(px((SIZE - 48.0) / 2.0))
        .size(px(48.0))
        .rounded_full()
        .bg(theme::gradient(alpha(c::ACCENT, 0.3), alpha(c::ACCENT_2, 0.3)))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(24.0))
        .font_weight(FontWeight::BOLD)
        .text_color(c::WHITE)
        .child("⇩");

    div()
        .relative()
        .size(px(SIZE))
        .child(ring("ring-a", false))
        .child(ring("ring-b", true))
        .child(core)
        .into_any_element()
}
