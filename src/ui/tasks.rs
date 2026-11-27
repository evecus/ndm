//! 中央任务区：任务卡片列表、分类为空提示、全局空状态。

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight};

use crate::app::{category_color, category_glyph, state_color, NdmApp};
use crate::model::{TaskState, TaskView};
use crate::platform;
use crate::theme::{alpha, c};
use crate::ui::widgets::{empty_illustration, icon_tile, primary_button, progress_bar, small_button, status_badge};
use crate::util;

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut out: String = s.chars().take(max.saturating_sub(2)).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

impl NdmApp {
    pub(crate) fn render_task_area(&mut self, tasks: &[TaskView], cx: &mut Context<Self>) -> AnyElement {
        let area = div()
            .id("task-area")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(c::BG);

        if tasks.is_empty() {
            return area
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(empty_illustration())
                .child(
                    div()
                        .mt(px(10.0))
                        .text_size(px(19.0))
                        .font_weight(FontWeight::BOLD)
                        .child("还没有下载任务"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_size(px(12.5))
                        .text_color(c::FAINT)
                        .child("点击「新建任务」手动添加，或安装官方")
                        .child("NeatDownloadManager 扩展后，点击网页下载链接即可自动接管"),
                )
                .child(
                    div().mt(px(18.0)).child(
                        primary_button("empty-new", "＋  新建任务").on_click(
                            cx.listener(|this, _, window, cx| this.open_new_task(window, cx)),
                        ),
                    ),
                )
                .into_any_element();
        }

        let visible: Vec<&TaskView> = tasks.iter().filter(|v| self.filter.matches(v)).collect();
        if visible.is_empty() {
            return area
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(c::FAINT)
                .child("该分类下没有任务")
                .into_any_element();
        }

        let mut list = div().flex().flex_col().gap(px(8.0));
        for v in visible {
            list = list.child(self.render_task_row(v, cx));
        }
        area.overflow_y_scroll()
            .p(px(14.0))
            .child(list)
            .into_any_element()
    }

    fn render_task_row(&mut self, v: &TaskView, cx: &mut Context<Self>) -> AnyElement {
        let id = v.id;
        let key = id as usize;
        let selected = self.selected == Some(id);
        let s_color = state_color(v.state);

        // 第一行：分类图标、文件名、HLS 标记、状态胶囊
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon_tile(category_glyph(v.category), category_color(v.category), 24.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(truncate(&v.filename, 44)),
                    )
                    .when(v.is_hls, |d| {
                        d.child(div().text_size(px(11.0)).text_color(c::ACCENT_2).child("HLS"))
                    }),
            )
            .child(status_badge(v.state.label(), s_color));

        // 第二行：大小 · 连接数 · 速度 · 剩余时间 · 来源
        let size_text = if v.size > 0 {
            util::format_size(v.size)
        } else if v.is_hls {
            v.media_stage.clone()
        } else {
            "未知大小".to_string()
        };
        let running = v.state == TaskState::Running;
        let meta = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(11.5))
            .text_color(c::MUTED)
            .child(format!("{size_text}  ·  {} 连接", v.threads))
            .when(running, |d| {
                d.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c::INFO)
                        .child(util::format_speed(v.speed)),
                )
                .child(v.eta.clone())
            })
            .child(div().text_color(c::FAINT).child(format!("·  {}", v.source)));

        let bar_color = match v.state {
            TaskState::Done => c::SUCCESS,
            TaskState::Error => c::ERROR,
            TaskState::Paused | TaskState::Canceled => c::WARN,
            _ => c::ACCENT,
        };
        let progress = progress_bar(id, v.progress, bar_color, running);

        // 底部：创建时间 + 操作按钮
        let in_flight = matches!(
            v.state,
            TaskState::Running | TaskState::Queued | TaskState::Merging
        );
        let actions = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                small_button(("retry", key), "⟳ 重试").on_click(cx.listener(move |this, _, _, cx| {
                    this.state.restart(id);
                    cx.notify();
                })),
            )
            .child(if in_flight {
                small_button(("pause", key), "⏸ 暂停").on_click(cx.listener(move |this, _, _, cx| {
                    this.state.pause(id);
                    cx.notify();
                }))
            } else {
                small_button(("start", key), "▶ 开始").on_click(cx.listener(move |this, _, _, cx| {
                    this.state.resume(id);
                    cx.notify();
                }))
            })
            .when(v.state == TaskState::Done, |d| {
                d.child(small_button(("open", key), "打开").on_click(cx.listener(
                    move |this, _, _, _| {
                        if let Some(t) = this.state.task(id) {
                            platform::reveal_file(&this.state.target_path(&t));
                        }
                    },
                )))
            })
            .child(
                small_button(("del", key), "✕ 删除").on_click(cx.listener(move |this, _, _, cx| {
                    this.state.remove(id);
                    if this.selected == Some(id) {
                        this.selected = None;
                    }
                    cx.notify();
                })),
            );

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(c::FAINT)
                    .child(v.created_at.clone()),
            )
            .child(actions);

        div()
            .id(("task", key))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(10.0))
            .border_1()
            .when(selected, |d| {
                d.bg(alpha(c::ACCENT, 0.10))
                    .border_color(alpha(c::ACCENT, 0.6))
            })
            .when(!selected, |d| {
                d.bg(c::SURFACE)
                    .border_color(c::BORDER)
                    .hover(|s| s.border_color(alpha(c::ACCENT, 0.35)))
            })
            .cursor_pointer()
            .child(head)
            .child(meta)
            .child(progress)
            .when(!v.error.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(c::ERROR)
                        .child(format!("错误: {}", v.error)),
                )
            })
            .child(footer)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id);
                cx.notify();
            }))
            .into_any_element()
    }
}
