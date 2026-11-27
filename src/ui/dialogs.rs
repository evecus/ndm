//! 弹窗：新建任务、设置、关于。统一使用全窗口遮罩 + 居中卡片，点遮罩或 Esc 关闭。

use gpui::{deferred, div, prelude::*, px, AnyElement, Context, Div, FontWeight, Stateful};
use gpui_component::input::Input;

use crate::app::{Dialog, NdmApp, NumField};
use crate::settings::ProxyMode;
use crate::theme::{alpha, c};
use crate::ui::widgets::{
    button, form_row, icon_tile, primary_button, section_title, toggle_row,
};

const OVERLAY: gpui::Rgba = gpui::Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};

impl NdmApp {
    pub(crate) fn render_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (width, body) = match self.dialog? {
            Dialog::NewTask => (580.0, self.render_new_task(cx)),
            Dialog::Settings => (640.0, self.render_settings(cx)),
            Dialog::About => (460.0, self.render_about(cx)),
        };

        let card = div()
            .id("dialog-card")
            .occlude()
            .w(px(width))
            .p(px(18.0))
            .rounded(px(14.0))
            .bg(c::SURFACE)
            .border_1()
            .border_color(c::BORDER)
            .shadow_lg()
            // 点卡片本身不冒泡到遮罩
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(body);

        Some(
            deferred(
                div()
                    .id("dialog-overlay")
                    .occlude()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(OVERLAY)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(cx.listener(|this, _, window, cx| this.close_dialog(window, cx)))
                    .child(card),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn dialog_header(glyph: &'static str, title: &'static str) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mb(px(12.0))
            .child(icon_tile(glyph, c::ACCENT, 26.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
    }

    /// 整数步进器：[-] 值 [+]
    fn stepper(&self, f: NumField, cx: &mut Context<Self>) -> AnyElement {
        let id = match f {
            NumField::NewThreads => "num-new-threads",
            NumField::MaxConnections => "num-max-conn",
            NumField::MaxSimultaneous => "num-max-sim",
        };
        let step_btn = |suffix: &'static str, label: &'static str| -> Stateful<Div> {
            div()
                .id((id, if suffix == "-" { 0usize } else { 1usize }))
                .size(px(26.0))
                .rounded(px(7.0))
                .bg(c::SURFACE_2)
                .border_1()
                .border_color(c::BORDER)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.border_color(c::ACCENT).text_color(c::WHITE))
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                step_btn("-", "−")
                    .on_click(cx.listener(move |this, _, _, cx| this.step_num(f, -1, cx))),
            )
            .child(
                div()
                    .w(px(44.0))
                    .h(px(26.0))
                    .rounded(px(7.0))
                    .bg(c::BG)
                    .border_1()
                    .border_color(c::BORDER)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.num_value(f).to_string()),
            )
            .child(
                step_btn("+", "+")
                    .on_click(cx.listener(move |this, _, _, cx| this.step_num(f, 1, cx))),
            )
            .into_any_element()
    }

    fn render_new_task(&mut self, cx: &mut Context<Self>) -> AnyElement {
        const LW: f32 = 104.0;
        let i = &self.inputs;
        div()
            .flex()
            .flex_col()
            .child(Self::dialog_header("＋", "新建下载任务"))
            .child(form_row("下载地址", LW, Input::new(&i.url)))
            .child(form_row("文件名（可选）", LW, Input::new(&i.filename)))
            .child(form_row("保存目录", LW, Input::new(&i.dir)))
            .child(form_row("Referer（可选）", LW, Input::new(&i.referer)))
            .child(form_row("Cookie（可选）", LW, Input::new(&i.cookie)))
            .child(form_row("连接数", LW, self.stepper(NumField::NewThreads, cx)))
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .gap(px(8.0))
                    .child(primary_button("new-submit", "添加任务").on_click(cx.listener(
                        |this, _, window, cx| this.submit_new_task(window, cx),
                    )))
                    .child(button("new-cancel", "取消", true).on_click(cx.listener(
                        |this, _, window, cx| this.close_dialog(window, cx),
                    ))),
            )
            .into_any_element()
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        const LW: f32 = 150.0;
        let s = self.settings_edit.clone();

        // 勾选项：点击直接翻转 settings_edit 里的对应字段
        macro_rules! toggle {
            ($id:literal, $label:literal, $field:ident) => {
                toggle_row($id, $label, s.$field).on_click(cx.listener(|this, _, _, cx| {
                    this.settings_edit.$field = !this.settings_edit.$field;
                    cx.notify();
                }))
            };
        }

        let mut modes = div().flex().gap(px(6.0));
        for (idx, m) in ProxyMode::ALL.into_iter().enumerate() {
            let on = s.proxy_mode == m;
            modes = modes.child(
                div()
                    .id(("proxy-mode", idx))
                    .h(px(28.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .border_1()
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(alpha(c::ACCENT, 0.2))
                            .border_color(c::ACCENT)
                            .text_color(c::WHITE)
                    })
                    .when(!on, |d| {
                        d.bg(c::SURFACE_2)
                            .border_color(c::BORDER)
                            .text_color(c::MUTED)
                            .hover(|s| s.text_color(c::WHITE))
                    })
                    .child(m.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_proxy_mode(m, cx))),
            );
        }

        let i = &self.inputs;
        let body = div()
            .id("settings-scroll")
            .max_h(px(540.0))
            .overflow_y_scroll()
            .pr(px(6.0))
            .flex()
            .flex_col()
            // 连接
            .child(section_title("连接"))
            .child(form_row("每个任务最大连接数", LW, self.stepper(NumField::MaxConnections, cx)))
            .child(form_row("同时下载任务数", LW, self.stepper(NumField::MaxSimultaneous, cx)))
            .child(form_row("全局限速 (KB/s)", LW, Input::new(&i.speed_limit)))
            .child(form_row("User-Agent", LW, Input::new(&i.user_agent)))
            .child(toggle!("t-ua-all", "对所有请求使用自定义 UA", ua_for_all))
            .child(toggle!(
                "t-dynseg",
                "动态分段（按服务器能力自动选择连接数）",
                dynamic_segmentation
            ))
            // 目录
            .child(section_title("目录"))
            .child(form_row("下载目录", LW, Input::new(&i.download_dir)))
            .child(form_row("临时目录", LW, Input::new(&i.temp_dir)))
            .child(toggle!(
                "t-catdir",
                "按文件类型建立子目录（Videos / Documents ...）",
                category_folders
            ))
            // 代理
            .child(section_title("代理"))
            .child(form_row("代理模式", LW, modes))
            .child(form_row("HTTP 代理 (host:port)", LW, Input::new(&i.http_proxy)))
            .child(form_row("HTTPS 代理", LW, Input::new(&i.https_proxy)))
            .child(form_row("SOCKS5 代理 (host:port)", LW, Input::new(&i.socks_proxy)))
            .child(form_row("代理用户名", LW, Input::new(&i.proxy_user)))
            .child(form_row("代理密码", LW, Input::new(&i.proxy_pass)))
            // 常规
            .child(section_title("常规"))
            .child(toggle!("t-done-tip", "下载完成时显示提示", show_completion_dialog))
            .child(toggle!("t-resume", "启动时恢复未完成任务", auto_resume))
            .child(toggle!("t-tray", "关闭窗口时最小化到系统托盘", minimize_to_tray))
            .child(toggle!("t-autostart", "开机自动启动（Windows）", auto_start));

        div()
            .flex()
            .flex_col()
            .child(Self::dialog_header("⚙", "设置"))
            .child(body)
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .gap(px(8.0))
                    .child(primary_button("settings-save", "保存设置").on_click(cx.listener(
                        |this, _, window, cx| this.save_settings(window, cx),
                    )))
                    .child(button("settings-cancel", "取消", true).on_click(cx.listener(
                        |this, _, window, cx| this.close_dialog(window, cx),
                    ))),
            )
            .into_any_element()
    }

    fn render_about(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut features = div().flex().flex_col().gap(px(4.0)).text_size(px(12.5));
        for line in [
            "· 多连接分段下载 / 断点续传 / 全局限速",
            "· HTTP / SOCKS5 代理、跟随系统代理",
            "· 兼容官方 NeatDownloadManager Extension 1.9.92",
            "· HLS (m3u8) 分片下载、AES-128 解密与合并",
        ] {
            features = features.child(line);
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(icon_tile("N", c::ACCENT, 38.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("NDM-RS {}", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(c::MUTED)
                                    .child("用 Rust + GPUI 编写的 Windows 桌面下载管理器"),
                            ),
                    ),
            )
            .child(div().my(px(12.0)).h(px(1.0)).bg(c::BORDER))
            .child(features)
            .child(
                div()
                    .mt(px(10.0))
                    .text_size(px(11.0))
                    .text_color(c::FAINT)
                    .child("扩展协议：ws://127.0.0.1:10007/download · neatextension.v1"),
            )
            .child(
                div()
                    .id("about-link")
                    .mt(px(6.0))
                    .text_size(px(12.5))
                    .text_color(c::ACCENT)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child("Neat Download Manager 官网")
                    .on_click(|_, _, cx| cx.open_url("https://www.neatdownloadmanager.com/")),
            )
            .child(
                div().mt(px(14.0)).flex().justify_end().child(
                    button("about-close", "关闭", true).on_click(cx.listener(
                        |this, _, window, cx| this.close_dialog(window, cx),
                    )),
                ),
            )
            .into_any_element()
    }
}
