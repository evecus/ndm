//! 顶部工具栏、左侧分类栏、底部状态栏、右下角通知。

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight};

use crate::app::{Filter, NdmApp};
use crate::model::{TaskState, TaskView};
use crate::platform;
use crate::theme::{alpha, c};
use crate::ui::widgets::{button, icon_tile, primary_button, pulse_status, stat_chip};
use crate::util;

impl NdmApp {
    pub(crate) fn render_toolbar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let sel = self.selected_task();
        let enabled = sel.is_some();

        let brand = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(icon_tile("N", c::ACCENT, 26.0))
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(c::WHITE)
                    .child("NDM"),
            )
            .child(div().text_size(px(11.0)).text_color(c::ACCENT).child("RS"));

        let divider = || div().flex_none().w(px(1.0)).h(px(20.0)).bg(c::BORDER);

        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(9.0))
            .bg(c::SURFACE)
            .border_b_1()
            .border_color(c::BORDER)
            .child(brand)
            .child(divider())
            .child(
                primary_button("tb-new", "＋  新建任务")
                    .on_click(cx.listener(|this, _, window, cx| this.open_new_task(window, cx))),
            )
            .child(button("tb-resume-all", "▶  全部开始", true).on_click(cx.listener(
                |this, _, _, cx| {
                    this.state.resume_all();
                    cx.notify();
                },
            )))
            .child(button("tb-pause-all", "⏸  全部暂停", true).on_click(cx.listener(
                |this, _, _, cx| {
                    this.state.pause_all();
                    cx.notify();
                },
            )))
            .child(divider())
            .child(
                button("tb-start", "▶ 开始", enabled).when_some(sel, |b, id| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.state.resume(id);
                        cx.notify();
                    }))
                }),
            )
            .child(
                button("tb-pause", "⏸ 暂停", enabled).when_some(sel, |b, id| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.state.pause(id);
                        cx.notify();
                    }))
                }),
            )
            .child(
                button("tb-retry", "⟳ 重试", enabled).when_some(sel, |b, id| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.state.restart(id);
                        cx.notify();
                    }))
                }),
            )
            .child(
                button("tb-delete", "✕ 删除", enabled).when_some(sel, |b, id| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.state.remove(id);
                        this.selected = None;
                        cx.notify();
                    }))
                }),
            )
            .child(
                button("tb-open", "📁 打开目录", true).on_click(cx.listener(
                    move |this, _, _, _| match sel.and_then(|id| this.state.task(id)) {
                        Some(t) => platform::reveal_file(&this.state.target_path(&t)),
                        None => platform::open_dir(&this.state.settings_snapshot().download_dir),
                    },
                )),
            )
            .child(button("tb-clear", "清空已完成", true).on_click(cx.listener(
                |this, _, _, cx| {
                    this.state.clear_completed();
                    cx.notify();
                },
            )))
            // 右侧：关于 / 设置
            .child(div().flex_1())
            .child(
                button("tb-about", "ⓘ 关于", true)
                    .on_click(cx.listener(|this, _, _, cx| this.open_about(cx))),
            )
            .child(
                button("tb-settings", "⚙ 设置", true)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
            .into_any_element()
    }

    pub(crate) fn render_sidebar(&mut self, tasks: &[TaskView], cx: &mut Context<Self>) -> AnyElement {
        let ext = self.state.ext_connected();

        let header = div()
            .flex_none()
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(c::SURFACE_2)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .child(icon_tile("⇩", c::ACCENT, 34.0))
            .child(
                div()
                    .mt(px(2.0))
                    .text_size(px(15.0))
                    .font_weight(FontWeight::BOLD)
                    .child("NDM-RS"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(c::FAINT)
                    .child("轻量下载管理器"),
            )
            .child(div().mt(px(6.0)).child(pulse_status(
                "pulse-sidebar",
                ext,
                if ext { "扩展已连接" } else { "扩展未连接" },
            )));

        let mut nav = div().flex().flex_col().gap(px(2.0)).mt(px(12.0));
        for (i, f) in Filter::ALL.into_iter().enumerate() {
            let count = tasks.iter().filter(|v| f.matches(v)).count();
            let selected = self.filter == f;
            nav = nav.child(
                div()
                    .id(("nav", i))
                    .relative()
                    .h(px(34.0))
                    .px(px(12.0))
                    .rounded(px(9.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.bg(alpha(c::ACCENT, 0.16)).child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(3.0))
                                .rounded(px(2.0))
                                .bg(c::ACCENT),
                        )
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(alpha(c::WHITE, 0.05))))
                    .child(
                        div()
                            .w(px(16.0))
                            .text_size(px(14.0))
                            .text_color(if selected { c::ACCENT } else { c::FAINT })
                            .child(f.glyph()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(if selected { c::WHITE } else { c::MUTED })
                            .child(f.label()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if selected { c::ACCENT } else { c::FAINT })
                            .child(count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = f;
                        cx.notify();
                    })),
            );
        }

        let footer = div()
            .mt(px(12.0))
            .pt(px(10.0))
            .border_t_1()
            .border_color(c::BORDER)
            .flex()
            .flex_col()
            .gap(px(2.0))
            .text_size(px(11.0))
            .text_color(c::FAINT)
            .child(div().text_color(c::MUTED).child("兼容官方扩展"))
            .child("NeatDownloadManager")
            .child("Extension 1.9.92")
            .child(div().mt(px(4.0)).child("ws://127.0.0.1:10007"));

        div()
            .id("sidebar")
            .flex_none()
            .w(px(206.0))
            .h_full()
            .overflow_y_scroll()
            .px(px(12.0))
            .py(px(14.0))
            .bg(c::BG)
            .border_r_1()
            .border_color(c::BORDER)
            .child(header)
            .child(nav)
            .child(footer)
            .into_any_element()
    }

    pub(crate) fn render_status_bar(&self, tasks: &[TaskView]) -> AnyElement {
        let running = tasks
            .iter()
            .filter(|v| v.state == TaskState::Running)
            .count();
        let limit = self.state.throttle.limit_kbps();
        let limit_text = if limit > 0 {
            format!("{limit} KB/s")
        } else {
            "不限".to_string()
        };
        let ext = self.state.ext_connected();

        div()
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(7.0))
            .bg(c::SURFACE)
            .border_t_1()
            .border_color(c::BORDER)
            .child(stat_chip("任务总数", tasks.len().to_string(), c::ACCENT))
            .child(stat_chip("下载中", running.to_string(), c::INFO))
            .child(stat_chip(
                "总速度",
                util::format_speed(self.state.total_speed()),
                c::SUCCESS,
            ))
            .child(stat_chip("限速", limit_text, c::WARN))
            .child(div().flex_1())
            .child(pulse_status(
                "pulse-status",
                ext,
                if ext {
                    "浏览器扩展已连接"
                } else {
                    "等待浏览器扩展连接"
                },
            ))
            .into_any_element()
    }

    /// 右下角通知（最近 8 秒内的最多 3 条事件，过期由刷新定时器带走）。
    pub(crate) fn render_toasts(&self) -> Option<AnyElement> {
        let events = self.state.recent_events();
        if events.is_empty() {
            return None;
        }
        let mut list = div()
            .absolute()
            .right(px(16.0))
            .bottom(px(52.0))
            .max_w(px(420.0))
            .px(px(14.0))
            .py(px(10.0))
            .rounded(px(12.0))
            .bg(c::SURFACE)
            .border_1()
            .border_color(c::BORDER)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(6.0));
        for e in events {
            let color = if e.is_error { c::ERROR } else { c::SUCCESS };
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::BOLD)
                            .text_color(color)
                            .child(if e.is_error { "✕" } else { "✓" }),
                    )
                    .child(div().text_color(c::TEXT).child(e.text)),
            );
        }
        Some(list.into_any_element())
    }
}
