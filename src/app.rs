//! GPUI 桌面界面：应用状态、窗口生命周期、托盘、刷新调度与布局编排。
//! 具体绘制在 `ui::*`，视觉常量在 `theme`。
//!
//! 与 egui 版的关键差异：
//! - egui 每帧轮询 `AppState`；这里由后台定时器按需 `notify`（有活动任务 200ms，空闲 500ms），
//!   流光 / 呼吸 / 脉冲动画交给 GPUI 的 `with_animation`，不再靠高频整窗重绘驱动。
//! - 「关闭到托盘」不再隐藏窗口：直接关闭窗口、进程继续运行，托盘「显示主窗口」时重新打开。
//!   下载引擎与任务状态都在 `Arc<AppState>` 里，窗口重建不影响任何下载。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, size, AnyView, App, AppContext, Application, Bounds, Context,
    Entity, FocusHandle, Focusable, KeyBinding, Task, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use gpui_component::input::InputState;
use gpui_component::Root;

use crate::model::{AppState, Category, DownloadRequest, TaskState, TaskView};
use crate::platform::{Tray, TrayAction};
use crate::settings::{ProxyMode, Settings};
use crate::theme::{self, c};

const REFRESH_ACTIVE: Duration = Duration::from_millis(200);
const REFRESH_IDLE: Duration = Duration::from_millis(500);
const TRAY_POLL: Duration = Duration::from_millis(150);

actions!(ndm, [NewTask, CloseDialog]);

// ── 分类过滤 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Active,
    Completed,
    Video,
    Audio,
    Archive,
    Document,
    Program,
    Other,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "全部任务",
            Filter::Active => "进行中",
            Filter::Completed => "已完成",
            Filter::Video => "视频",
            Filter::Audio => "音频",
            Filter::Archive => "压缩包",
            Filter::Document => "文档",
            Filter::Program => "程序",
            Filter::Other => "其它",
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            Filter::All => "▦",
            Filter::Active => "◉",
            Filter::Completed => "✓",
            Filter::Video => "▶",
            Filter::Audio => "♪",
            Filter::Archive => "▣",
            Filter::Document => "▤",
            Filter::Program => "⚙",
            Filter::Other => "⋯",
        }
    }
    pub fn matches(self, v: &TaskView) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => v.state.is_active() || v.state == TaskState::Paused,
            Filter::Completed => v.state == TaskState::Done,
            Filter::Video => v.category == Category::Video,
            Filter::Audio => v.category == Category::Audio,
            Filter::Archive => v.category == Category::Archive,
            Filter::Document => v.category == Category::Document,
            Filter::Program => v.category == Category::Program,
            Filter::Other => v.category == Category::Other,
        }
    }
    pub const ALL: [Filter; 9] = [
        Filter::All,
        Filter::Active,
        Filter::Completed,
        Filter::Video,
        Filter::Audio,
        Filter::Archive,
        Filter::Document,
        Filter::Program,
        Filter::Other,
    ];
}

pub fn category_glyph(cat: Category) -> &'static str {
    match cat {
        Category::Video => "▶",
        Category::Audio => "♪",
        Category::Archive => "▣",
        Category::Document => "▤",
        Category::Program => "⚙",
        Category::Other => "⋯",
    }
}

pub fn category_color(cat: Category) -> gpui::Rgba {
    match cat {
        Category::Video => c::ACCENT,
        Category::Audio => c::SUCCESS,
        Category::Archive => c::WARN,
        Category::Document => c::INFO,
        Category::Program => c::ACCENT_2,
        Category::Other => c::FAINT,
    }
}

pub fn state_color(s: TaskState) -> gpui::Rgba {
    match s {
        TaskState::Done => c::SUCCESS,
        TaskState::Error => c::ERROR,
        TaskState::Running | TaskState::Merging => c::INFO,
        TaskState::Paused => c::WARN,
        TaskState::Queued => c::ACCENT,
        TaskState::Canceled => c::FAINT,
    }
}

// ── 应用状态 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    NewTask,
    Settings,
    About,
}

/// 用 +/- 步进器编辑的整数字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumField {
    NewThreads,
    MaxConnections,
    MaxSimultaneous,
}

impl NumField {
    pub fn range(self) -> (usize, usize) {
        match self {
            NumField::NewThreads | NumField::MaxConnections => (1, 32),
            NumField::MaxSimultaneous => (1, 16),
        }
    }
}

/// 所有文本输入框。gpui-component 的输入框是独立实体，打开对话框时 `set_value` 回填。
pub struct Inputs {
    // 新建任务
    pub url: Entity<InputState>,
    pub filename: Entity<InputState>,
    pub dir: Entity<InputState>,
    pub referer: Entity<InputState>,
    pub cookie: Entity<InputState>,
    // 设置
    pub user_agent: Entity<InputState>,
    pub speed_limit: Entity<InputState>,
    pub download_dir: Entity<InputState>,
    pub temp_dir: Entity<InputState>,
    pub http_proxy: Entity<InputState>,
    pub https_proxy: Entity<InputState>,
    pub socks_proxy: Entity<InputState>,
    pub proxy_user: Entity<InputState>,
    pub proxy_pass: Entity<InputState>,
}

impl Inputs {
    fn new(window: &mut Window, cx: &mut Context<NdmApp>) -> Self {
        let mut mk = |placeholder: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let url = mk("https://...");
        let filename = mk("留空则自动推断");
        let dir = mk("");
        let referer = mk("");
        let cookie = mk("");
        let user_agent = mk("");
        let speed_limit = mk("0 表示不限速");
        let download_dir = mk("");
        let temp_dir = mk("留空则与下载目录相同");
        let http_proxy = mk("127.0.0.1:7890");
        let https_proxy = mk("留空复用 HTTP 代理");
        let socks_proxy = mk("127.0.0.1:1080");
        let proxy_user = mk("");
        let proxy_pass = cx.new(|cx| InputState::new(window, cx).masked(true));
        Self {
            url,
            filename,
            dir,
            referer,
            cookie,
            user_agent,
            speed_limit,
            download_dir,
            temp_dir,
            http_proxy,
            https_proxy,
            socks_proxy,
            proxy_user,
            proxy_pass,
        }
    }
}

pub fn input_value(e: &Entity<InputState>, cx: &App) -> String {
    e.read(cx).value().to_string()
}

fn set_input(e: &Entity<InputState>, v: impl Into<String>, window: &mut Window, cx: &mut App) {
    let v: String = v.into();
    e.update(cx, |s, cx| s.set_value(v, window, cx));
}

pub struct NdmApp {
    pub state: Arc<AppState>,
    pub filter: Filter,
    pub selected: Option<u64>,
    pub dialog: Option<Dialog>,
    pub inputs: Inputs,
    pub new_threads: usize,
    /// 设置对话框里正在编辑的副本（勾选项 / 步进器 / 代理模式直接改它，文本框保存时读取）。
    pub settings_edit: Settings,
    has_active: bool,
    focus_handle: FocusHandle,
    _refresh: Task<()>,
}

impl NdmApp {
    pub fn new(state: Arc<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings_edit = state.settings_snapshot();
        let inputs = Inputs::new(window, cx);
        set_input(
            &inputs.dir,
            settings_edit.download_dir.to_string_lossy(),
            window,
            cx,
        );

        // 刷新调度：任务进度、速度、扩展连接、通知过期都来自 AppState（其它线程在写），
        // 这里定时让视图重新渲染。
        let refresh = cx.spawn(async move |this, cx| loop {
            let active = this.update(cx, |this, cx| {
                cx.notify();
                this.has_active
            });
            let Ok(active) = active else { break };
            let wait = if active { REFRESH_ACTIVE } else { REFRESH_IDLE };
            cx.background_executor().timer(wait).await;
        });

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        Self {
            state,
            filter: Filter::All,
            selected: None,
            dialog: None,
            inputs,
            new_threads: settings_edit.max_connections,
            settings_edit,
            has_active: false,
            focus_handle,
            _refresh: refresh,
        }
    }

    pub fn selected_task(&self) -> Option<u64> {
        self.selected.filter(|id| self.state.task(*id).is_some())
    }

    // ── 对话框 ─────────────────────────────────────────────────────────────

    pub fn open_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(Dialog::NewTask);
        let handle = self.inputs.url.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.state.settings_snapshot();
        let i = &self.inputs;
        set_input(&i.user_agent, s.user_agent.clone(), window, cx);
        set_input(&i.speed_limit, s.speed_limit_kbps.to_string(), window, cx);
        set_input(&i.download_dir, s.download_dir.to_string_lossy(), window, cx);
        set_input(&i.temp_dir, s.temp_dir.to_string_lossy(), window, cx);
        set_input(&i.http_proxy, s.http_proxy.clone(), window, cx);
        set_input(&i.https_proxy, s.https_proxy.clone(), window, cx);
        set_input(&i.socks_proxy, s.socks_proxy.clone(), window, cx);
        set_input(&i.proxy_user, s.proxy_user.clone(), window, cx);
        set_input(&i.proxy_pass, s.proxy_pass.clone(), window, cx);
        self.settings_edit = s;
        self.dialog = Some(Dialog::Settings);
        cx.notify();
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        self.dialog = Some(Dialog::About);
        cx.notify();
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    pub fn submit_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = input_value(&self.inputs.url, cx).trim().to_string();
        if url.is_empty() {
            self.state.push_event("请填写下载地址".to_string(), true);
            cx.notify();
            return;
        }
        let referer = input_value(&self.inputs.referer, cx);
        let cookie = input_value(&self.inputs.cookie, cx);
        let mut headers = Vec::new();
        if !referer.trim().is_empty() {
            headers.push(("Referer".to_string(), referer.trim().to_string()));
        }
        if !cookie.trim().is_empty() {
            headers.push(("Cookie".to_string(), cookie.trim().to_string()));
        }
        let dir = input_value(&self.inputs.dir, cx);
        let save_dir = if dir.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(dir.trim()))
        };
        self.state.add_task(DownloadRequest {
            url,
            filename: input_value(&self.inputs.filename, cx).trim().to_string(),
            headers,
            threads: Some(self.new_threads),
            save_dir,
            source: "手动添加".to_string(),
            ..Default::default()
        });
        set_input(&self.inputs.url, "", window, cx);
        set_input(&self.inputs.filename, "", window, cx);
        self.close_dialog(window, cx);
    }

    pub fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = &self.inputs;
        let speed_raw = input_value(&i.speed_limit, cx);
        let speed = if speed_raw.trim().is_empty() {
            0
        } else {
            match speed_raw.trim().parse::<u64>() {
                Ok(v) => v.min(1_000_000),
                Err(_) => {
                    self.state
                        .push_event("全局限速需填写整数（KB/s）".to_string(), true);
                    cx.notify();
                    return;
                }
            }
        };

        let mut edited = self.settings_edit.clone();
        edited.speed_limit_kbps = speed;
        edited.user_agent = input_value(&i.user_agent, cx).trim().to_string();
        let dl = input_value(&i.download_dir, cx);
        if !dl.trim().is_empty() {
            edited.download_dir = PathBuf::from(dl.trim());
        }
        let tmp = input_value(&i.temp_dir, cx);
        edited.temp_dir = if tmp.trim().is_empty() {
            PathBuf::new()
        } else {
            PathBuf::from(tmp.trim())
        };
        edited.http_proxy = input_value(&i.http_proxy, cx).trim().to_string();
        edited.https_proxy = input_value(&i.https_proxy, cx).trim().to_string();
        edited.socks_proxy = input_value(&i.socks_proxy, cx).trim().to_string();
        edited.proxy_user = input_value(&i.proxy_user, cx).trim().to_string();
        edited.proxy_pass = input_value(&i.proxy_pass, cx);

        // update_settings 内部会 sanitize、应用限速、写开机自启并落盘。
        self.state.update_settings(|s| *s = edited);
        self.settings_edit = self.state.settings_snapshot();
        self.state.push_event("设置已保存".to_string(), false);
        self.close_dialog(window, cx);
    }

    pub fn num_value(&self, f: NumField) -> usize {
        match f {
            NumField::NewThreads => self.new_threads,
            NumField::MaxConnections => self.settings_edit.max_connections,
            NumField::MaxSimultaneous => self.settings_edit.max_simultaneous,
        }
    }

    pub fn step_num(&mut self, f: NumField, delta: i32, cx: &mut Context<Self>) {
        let (lo, hi) = f.range();
        let v = (self.num_value(f) as i64 + delta as i64).clamp(lo as i64, hi as i64) as usize;
        match f {
            NumField::NewThreads => self.new_threads = v,
            NumField::MaxConnections => self.settings_edit.max_connections = v,
            NumField::MaxSimultaneous => self.settings_edit.max_simultaneous = v,
        }
        cx.notify();
    }

    pub fn set_proxy_mode(&mut self, m: ProxyMode, cx: &mut Context<Self>) {
        self.settings_edit.proxy_mode = m;
        cx.notify();
    }

    // ── 动作 ───────────────────────────────────────────────────────────────

    fn on_new_task(&mut self, _: &NewTask, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_task(window, cx);
    }

    fn on_close_dialog(&mut self, _: &CloseDialog, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            self.close_dialog(window, cx);
        }
    }
}

impl Focusable for NdmApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NdmApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks = self.state.tasks_snapshot();
        self.has_active = tasks.iter().any(|v| v.state.is_active());

        let toolbar = self.render_toolbar(cx);
        let sidebar = self.render_sidebar(&tasks, cx);
        let content = self.render_task_area(&tasks, cx);
        let status = self.render_status_bar(&tasks);
        let toasts = self.render_toasts();
        let dialog = self.render_dialog(cx);

        div()
            .id("ndm-root")
            .key_context("Ndm")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_task))
            .on_action(cx.listener(Self::on_close_dialog))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme::UI_FONT)
            .text_size(px(13.0))
            .text_color(c::TEXT)
            .bg(c::BG)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(sidebar)
                    .child(content),
            )
            .child(status)
            .children(toasts)
            .children(dialog)
    }
}

// ── 窗口与托盘 ──────────────────────────────────────────────────────────────

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewTask, None),
        KeyBinding::new("escape", CloseDialog, None),
    ]);
}

fn open_main_window(state: Arc<AppState>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1200.0), px(760.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(980.0), px(600.0))),
        titlebar: Some(TitlebarOptions {
            title: Some("NDM-RS 下载管理器".into()),
            ..Default::default()
        }),
        app_id: Some("ndm-rs".into()),
        ..Default::default()
    };
    let result = cx.open_window(options, move |window, cx| {
        let view = cx.new(|cx| NdmApp::new(state, window, cx));
        let view: AnyView = view.into();
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(e) = result {
        log::error!("创建窗口失败: {e}");
    }
}

/// 托盘「显示主窗口」：已有窗口则激活，否则重新创建。
fn show_main_window(state: Arc<AppState>, cx: &mut App) {
    if let Some(handle) = cx.windows().into_iter().next() {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    } else {
        open_main_window(state, cx);
    }
    cx.activate(true);
}

/// 托盘只在 Windows 上存在；其它平台关闭窗口即退出。
const HAS_TRAY: bool = cfg!(windows);

pub fn run_gui(state: Arc<AppState>, minimized: bool) {
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        theme::init(cx);
        bind_keys(cx);

        // 最后一个窗口关闭时：开启了「最小化到托盘」就留在托盘里继续下载，否则退出。
        {
            let state = state.clone();
            cx.on_window_closed(move |cx| {
                if cx.windows().is_empty() {
                    let to_tray = HAS_TRAY && state.settings_snapshot().minimize_to_tray;
                    if !to_tray {
                        cx.quit();
                    }
                }
            })
            .detach();
        }

        // 退出前把任务进度与设置落盘。
        {
            let state = state.clone();
            cx.on_app_quit(move |_cx| {
                state.save_all();
                async {}
            })
            .detach();
        }

        // 托盘必须在主线程（GPUI 的消息循环线程）创建，事件在前台定时轮询。
        let tray = Tray::new();
        {
            let state = state.clone();
            cx.spawn(async move |cx| loop {
                cx.background_executor().timer(TRAY_POLL).await;
                let action = tray.poll();
                let alive = cx.update(|cx| match action {
                    TrayAction::Show => show_main_window(state.clone(), cx),
                    TrayAction::Quit => cx.quit(),
                    TrayAction::None => {}
                });
                if alive.is_err() {
                    break;
                }
            })
            .detach();
        }

        // --minimized 只在有托盘时生效，否则窗口永远出不来。
        if !(minimized && HAS_TRAY) {
            open_main_window(state.clone(), cx);
            cx.activate(true);
        }
    });
}
