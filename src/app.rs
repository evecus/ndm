//! egui 桌面界面：任务列表、分类侧栏、新建任务、设置、托盘与完成提示。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use crate::model::{AppState, DownloadRequest, TaskState, TaskView};
use crate::platform::{self, Tray, TrayAction};
use crate::settings::{ProxyMode, Settings};
use crate::util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
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
    fn label(self) -> &'static str {
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
    fn matches(self, v: &TaskView) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => v.state.is_active() || v.state == TaskState::Paused,
            Filter::Completed => v.state == TaskState::Done,
            Filter::Video => v.category == crate::model::Category::Video,
            Filter::Audio => v.category == crate::model::Category::Audio,
            Filter::Archive => v.category == crate::model::Category::Archive,
            Filter::Document => v.category == crate::model::Category::Document,
            Filter::Program => v.category == crate::model::Category::Program,
            Filter::Other => v.category == crate::model::Category::Other,
        }
    }
    const ALL: [Filter; 9] = [
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

pub struct NdmApp {
    state: Arc<AppState>,
    tray: Tray,
    filter: Filter,
    selected: Option<u64>,
    show_new: bool,
    show_settings: bool,
    show_about: bool,
    allow_close: bool,
    hide_requested: bool,
    font_warning: bool,

    set_download_dir: String,
    set_temp_dir: String,

    new_url: String,
    new_filename: String,
    new_dir: String,
    new_referer: String,
    new_cookie: String,
    new_threads: usize,

    settings_edit: Settings,
}

impl NdmApp {
    fn new(state: Arc<AppState>) -> Self {
        let settings_edit = state.settings_snapshot();
        let new_dir = settings_edit.download_dir.to_string_lossy().to_string();
        let set_download_dir = new_dir.clone();
        let set_temp_dir = settings_edit.temp_dir.to_string_lossy().to_string();
        Self {
            state,
            tray: Tray::new(),
            filter: Filter::All,
            selected: None,
            show_new: false,
            show_settings: false,
            show_about: false,
            allow_close: false,
            hide_requested: false,
            font_warning: false,
            set_download_dir,
            set_temp_dir,
            new_url: String::new(),
            new_filename: String::new(),
            new_dir,
            new_referer: String::new(),
            new_cookie: String::new(),
            new_threads: 8,
            settings_edit,
        }
    }

    fn selected_task(&self) -> Option<u64> {
        self.selected.filter(|id| self.state.task(*id).is_some())
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        if ui.button("新建任务").clicked() {
            self.show_new = true;
        }
        if ui.button("全部开始").clicked() {
            self.state.resume_all();
        }
        if ui.button("全部暂停").clicked() {
            self.state.pause_all();
        }
        ui.separator();
        let sel = self.selected_task();
        let enabled = sel.is_some();
        if ui.add_enabled(enabled, egui::Button::new("开始")).clicked() {
            if let Some(id) = sel {
                self.state.resume(id);
            }
        }
        if ui.add_enabled(enabled, egui::Button::new("暂停")).clicked() {
            if let Some(id) = sel {
                self.state.pause(id);
            }
        }
        if ui.add_enabled(enabled, egui::Button::new("重试")).clicked() {
            if let Some(id) = sel {
                self.state.restart(id);
            }
        }
        if ui.add_enabled(enabled, egui::Button::new("删除")).clicked() {
            if let Some(id) = sel {
                self.state.remove(id);
                self.selected = None;
            }
        }
        if ui.button("打开目录").clicked() {
            if let Some(id) = sel {
                if let Some(t) = self.state.task(id) {
                    platform::reveal_file(&self.state.target_path(&t));
                }
            } else {
                platform::open_dir(&self.state.settings_snapshot().download_dir);
            }
        }
        if ui.button("清空已完成").clicked() {
            self.state.clear_completed();
        }
        ui.separator();
        if ui.button("设置").clicked() {
            self.settings_edit = self.state.settings_snapshot();
            self.set_download_dir = self.settings_edit.download_dir.to_string_lossy().to_string();
            self.set_temp_dir = self.settings_edit.temp_dir.to_string_lossy().to_string();
            self.show_settings = true;
        }
        if ui.button("关于").clicked() {
            self.show_about = true;
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, tasks: &[TaskView]) {
        ui.heading("NDM-RS");
        ui.label("下载管理器");
        ui.add_space(8.0);
        ui.separator();
        for f in Filter::ALL {
            let count = tasks.iter().filter(|v| f.matches(v)).count();
            let text = format!("{}  ({})", f.label(), count);
            if ui.selectable_label(self.filter == f, text).clicked() {
                self.filter = f;
            }
        }
        ui.add_space(12.0);
        ui.separator();
        let ext = self.state.ext_connected();
        ui.horizontal(|ui| {
            ui.colored_label(
                if ext {
                    egui::Color32::from_rgb(30, 150, 70)
                } else {
                    egui::Color32::GRAY
                },
                if ext { "● 扩展已连接" } else { "○ 扩展未连接" },
            );
        });
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("官方扩展\nNeatDownloadManager\nExtension 1.9.92")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("ws://127.0.0.1:10007\n/download")
                .small()
                .color(egui::Color32::GRAY),
        );
    }

    fn task_row(&mut self, ui: &mut egui::Ui, v: &TaskView) {
        let selected = self.selected == Some(v.id);
        let bg = if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().faint_bg_color
        };
        egui::Frame::NONE
            .fill(bg)
            .corner_radius(4.0)
            .inner_margin(egui::vec2(8.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let name = if v.filename.chars().count() > 42 {
                        let mut s: String = v.filename.chars().take(40).collect();
                        s.push_str("…");
                        s
                    } else {
                        v.filename.clone()
                    };
                    if ui
                        .add_sized(
                            [300.0, 20.0],
                            egui::SelectableLabel::new(selected, egui::RichText::new(name).strong()),
                        )
                        .clicked()
                    {
                        self.selected = Some(v.id);
                    }
                    ui.add_sized(
                        [90.0, 20.0],
                        egui::Label::new(if v.is_hls {
                            v.media_stage.clone()
                        } else if v.size > 0 {
                            util::format_size(v.size)
                        } else {
                            "未知大小".to_string()
                        }),
                    );
                    ui.add_sized([180.0, 16.0], egui::ProgressBar::new(v.progress));
                    ui.add_sized(
                        [70.0, 20.0],
                        egui::Label::new(util::format_speed(v.speed)),
                    );
                    ui.add_sized([62.0, 20.0], egui::Label::new(v.eta.clone()));
                    let (color, text) = state_color(v.state);
                    ui.add_sized(
                        [70.0, 20.0],
                        egui::Label::new(egui::RichText::new(text).color(color)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(format!("来源: {}  ·  {} 连接  ·  {}", v.source, v.threads, v.created_at))
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    if !v.error.is_empty() {
                        ui.colored_label(egui::Color32::RED, format!("错误: {}", v.error));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("删除").clicked() {
                            self.state.remove(v.id);
                            if self.selected == Some(v.id) {
                                self.selected = None;
                            }
                        }
                        if v.state == TaskState::Done {
                            if ui.small_button("打开").clicked() {
                                if let Some(t) = self.state.task(v.id) {
                                    platform::reveal_file(&self.state.target_path(&t));
                                }
                            }
                        }
                        if matches!(v.state, TaskState::Running | TaskState::Queued | TaskState::Merging) {
                            if ui.small_button("暂停").clicked() {
                                self.state.pause(v.id);
                            }
                        } else if ui.small_button("开始").clicked() {
                            self.state.resume(v.id);
                        }
                        if ui.small_button("重试").clicked() {
                            self.state.restart(v.id);
                        }
                    });
                });
            });
        ui.add_space(4.0);
    }

    fn new_task_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_new;
        egui::Window::new("新建下载任务")
            .open(&mut open)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::Grid::new("new_task_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("下载地址");
                        ui.add_sized(
                            [430.0, 22.0],
                            egui::TextEdit::singleline(&mut self.new_url).hint_text("https://..."),
                        );
                        ui.end_row();

                        ui.label("文件名（可选）");
                        ui.add_sized(
                            [430.0, 22.0],
                            egui::TextEdit::singleline(&mut self.new_filename)
                                .hint_text("留空则自动推断"),
                        );
                        ui.end_row();

                        ui.label("保存目录");
                        ui.add_sized(
                            [430.0, 22.0],
                            egui::TextEdit::singleline(&mut self.new_dir),
                        );
                        ui.end_row();

                        ui.label("Referer（可选）");
                        ui.add_sized(
                            [430.0, 22.0],
                            egui::TextEdit::singleline(&mut self.new_referer),
                        );
                        ui.end_row();

                        ui.label("Cookie（可选）");
                        ui.add_sized(
                            [430.0, 22.0],
                            egui::TextEdit::singleline(&mut self.new_cookie),
                        );
                        ui.end_row();

                        ui.label("连接数");
                        ui.add(egui::DragValue::new(&mut self.new_threads).range(1.0..=32.0));
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("添加任务").clicked() {
                        self.submit_new_task();
                    }
                    if ui.button("取消").clicked() {
                        self.show_new = false;
                    }
                });
            });
        self.show_new = open;
    }

    fn submit_new_task(&mut self) {
        let url = self.new_url.trim().to_string();
        if url.is_empty() {
            self.state.push_event("请填写下载地址".to_string(), true);
            return;
        }
        let mut headers = Vec::new();
        if !self.new_referer.trim().is_empty() {
            headers.push(("Referer".to_string(), self.new_referer.trim().to_string()));
        }
        if !self.new_cookie.trim().is_empty() {
            headers.push(("Cookie".to_string(), self.new_cookie.trim().to_string()));
        }
        let dir = self.new_dir.trim();
        let save_dir = if dir.is_empty() {
            None
        } else {
            Some(PathBuf::from(dir))
        };
        self.state.add_task(DownloadRequest {
            url,
            filename: self.new_filename.trim().to_string(),
            headers,
            threads: Some(self.new_threads),
            save_dir,
            source: "手动添加".to_string(),
            ..Default::default()
        });
        self.new_url.clear();
        self.new_filename.clear();
        self.show_new = false;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("设置")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .show(ui, |ui| {
                        ui.heading("连接");
                        egui::Grid::new("set_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("每个任务最大连接数")
                                    .on_hover_text("分块下载，需服务器支持 Range");
                                ui.add(
                                    egui::DragValue::new(&mut self.settings_edit.max_connections)
                                        .range(1.0..=32.0),
                                );
                                ui.end_row();

                                ui.label("同时下载任务数");
                                ui.add(
                                    egui::DragValue::new(&mut self.settings_edit.max_simultaneous)
                                        .range(1.0..=16.0),
                                );
                                ui.end_row();

                                ui.label("全局限速 (KB/s)")
                                    .on_hover_text("0 表示不限速，修改立即生效");
                                ui.add(
                                    egui::DragValue::new(&mut self.settings_edit.speed_limit_kbps)
                                        .range(0.0..=1_000_000.0),
                                );
                                ui.end_row();

                                ui.label("User-Agent");
                                ui.add_sized(
                                    [390.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.user_agent),
                                );
                                ui.end_row();
                            });
                        ui.checkbox(&mut self.settings_edit.ua_for_all, "对所有请求使用自定义 UA");
                        ui.checkbox(
                            &mut self.settings_edit.dynamic_segmentation,
                            "动态分段（按服务器能力自动选择连接数）",
                        );

                        ui.add_space(10.0);
                        ui.heading("目录");
                        ui.horizontal(|ui| {
                            ui.label("下载目录");
                            ui.add_sized(
                                [390.0, 22.0],
                                egui::TextEdit::singleline(&mut self.set_download_dir),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("临时目录");
                            ui.add_sized(
                                [390.0, 22.0],
                                egui::TextEdit::singleline(&mut self.set_temp_dir)
                                    .hint_text("留空则与下载目录相同"),
                            );
                        });
                        ui.checkbox(
                            &mut self.settings_edit.category_folders,
                            "按文件类型建立子目录（Videos / Documents ...）",
                        );

                        ui.add_space(10.0);
                        ui.heading("代理");
                        egui::ComboBox::from_label("代理模式")
                            .selected_text(self.settings_edit.proxy_mode.label())
                            .show_ui(ui, |ui| {
                                for m in ProxyMode::ALL {
                                    ui.selectable_value(
                                        &mut self.settings_edit.proxy_mode,
                                        m,
                                        m.label(),
                                    );
                                }
                            });
                        egui::Grid::new("proxy_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("HTTP 代理 (host:port)");
                                ui.add_sized(
                                    [300.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.http_proxy),
                                );
                                ui.end_row();
                                ui.label("HTTPS 代理 (留空复用)");
                                ui.add_sized(
                                    [300.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.https_proxy),
                                );
                                ui.end_row();
                                ui.label("SOCKS5 代理 (host:port)");
                                ui.add_sized(
                                    [300.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.socks_proxy),
                                );
                                ui.end_row();
                                ui.label("代理用户名");
                                ui.add_sized(
                                    [300.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.proxy_user),
                                );
                                ui.end_row();
                                ui.label("代理密码");
                                ui.add_sized(
                                    [300.0, 22.0],
                                    egui::TextEdit::singleline(&mut self.settings_edit.proxy_pass)
                                        .password(true),
                                );
                                ui.end_row();
                            });

                        ui.add_space(10.0);
                        ui.heading("常规");
                        ui.checkbox(
                            &mut self.settings_edit.show_completion_dialog,
                            "下载完成时显示提示",
                        );
                        ui.checkbox(&mut self.settings_edit.auto_resume, "启动时恢复未完成任务");
                        ui.checkbox(
                            &mut self.settings_edit.minimize_to_tray,
                            "关闭窗口时最小化到系统托盘",
                        );
                        ui.checkbox(&mut self.settings_edit.auto_start, "开机自动启动（Windows）");
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("保存设置").clicked() {
                        let mut edited = self.settings_edit.clone();
                        if !self.set_download_dir.trim().is_empty() {
                            edited.download_dir = PathBuf::from(self.set_download_dir.trim());
                        }
                        edited.temp_dir = if self.set_temp_dir.trim().is_empty() {
                            PathBuf::new()
                        } else {
                            PathBuf::from(self.set_temp_dir.trim())
                        };
                        self.state.update_settings(|s| *s = edited);
                        self.settings_edit = self.state.settings_snapshot();
                        self.state.push_event("设置已保存".to_string(), false);
                        self.show_settings = false;
                    }
                    if ui.button("取消").clicked() {
                        self.settings_edit = self.state.settings_snapshot();
                        self.set_download_dir = self.settings_edit.download_dir.to_string_lossy().to_string();
                        self.set_temp_dir = self.settings_edit.temp_dir.to_string_lossy().to_string();
                        self.show_settings = false;
                    }
                });
            });
        self.show_settings = open;
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("关于 NDM-RS")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading(format!("NDM-RS {}", env!("CARGO_PKG_VERSION")));
                ui.label("用 Rust 编写的 Windows 桌面下载管理器");
                ui.add_space(8.0);
                ui.label("· 多连接分段下载 / 断点续传 / 全局限速");
                ui.label("· HTTP / SOCKS5 代理、跟随系统代理");
                ui.label("· 兼容官方 NeatDownloadManager Extension 1.9.92");
                ui.label("· HLS (m3u8) 分片下载、AES-128 解密与合并");
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "扩展协议：ws://127.0.0.1:10007/download，子协议 neatextension.v1",
                    )
                    .small(),
                );
                ui.hyperlink_to(
                    "Neat Download Manager 官网",
                    "https://www.neatdownloadmanager.com/",
                );
            });
        self.show_about = open;
    }
}

fn state_color(s: TaskState) -> (egui::Color32, &'static str) {
    match s {
        TaskState::Done => (egui::Color32::from_rgb(27, 130, 60), s.label()),
        TaskState::Error => (egui::Color32::from_rgb(198, 40, 40), s.label()),
        TaskState::Running | TaskState::Merging => {
            (egui::Color32::from_rgb(21, 101, 192), s.label())
        }
        TaskState::Paused => (egui::Color32::from_rgb(200, 100, 10), s.label()),
        TaskState::Queued => (egui::Color32::GRAY, s.label()),
        TaskState::Canceled => (egui::Color32::DARK_GRAY, s.label()),
    }
}

impl eframe::App for NdmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(400));

        // 关闭窗口时按设置最小化到托盘（eframe 0.31 无 on_close_event 钩子，
        // 需在 update 中检查 close_requested 并取消关闭）。
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.allow_close && self.state.settings_snapshot().minimize_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide_requested = true;
            }
        }

        if self.hide_requested {
            self.hide_requested = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        match self.tray.poll() {
            TrayAction::Show => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            TrayAction::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            TrayAction::None => {}
        }

        let tasks = self.state.tasks_snapshot();
        let total_speed = self.state.total_speed();

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("NDM-RS");
                ui.separator();
                self.toolbar(ui);
            });
            if self.font_warning {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 100, 10),
                    "未找到系统中文字体，界面文字可能显示为方块；可设置环境变量 NDM_FONT 指向一个中文 .ttf 文件",
                );
            }
        });

        egui::SidePanel::left("sidebar")
            .resizable(false)
            .default_width(190.0)
            .show(ctx, |ui| {
                self.sidebar(ui, &tasks);
            });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("共 {} 个任务", tasks.len()));
                ui.separator();
                ui.label(format!(
                    "下载中 {}",
                    tasks.iter().filter(|v| v.state == TaskState::Running).count()
                ));
                ui.separator();
                ui.label(format!("总速度 {}", util::format_speed(total_speed)));
                ui.separator();
                let limit = self.state.throttle.limit_kbps();
                ui.label(if limit > 0 {
                    format!("限速 {} KB/s", limit)
                } else {
                    "未限速".to_string()
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ext = self.state.ext_connected();
                    ui.colored_label(
                        if ext {
                            egui::Color32::from_rgb(30, 150, 70)
                        } else {
                            egui::Color32::GRAY
                        },
                        if ext {
                            "浏览器扩展已连接"
                        } else {
                            "等待浏览器扩展连接"
                        },
                    );
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if tasks.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "暂无下载任务\n\n点击「新建任务」，或在浏览器中安装官方\nNeatDownloadManager Extension 后，点击下载链接即可自动接管",
                        )
                        .size(16.0)
                        .color(egui::Color32::GRAY),
                    );
                });
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let visible: Vec<TaskView> = tasks
                    .iter()
                    .filter(|v| self.filter.matches(v))
                    .cloned()
                    .collect();
                for v in &visible {
                    self.task_row(ui, v);
                }
                if visible.is_empty() {
                    ui.label("该分类下没有任务");
                }
            });
        });

        if self.show_new {
            self.new_task_window(ctx);
        }
        if self.show_settings {
            self.settings_window(ctx);
        }
        if self.show_about {
            self.about_window(ctx);
        }

        // 右下角通知
        let events = self.state.recent_events();
        if !events.is_empty() {
            egui::Window::new("通知")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -14.0))
                .title_bar(false)
                .resizable(false)
                .show(ctx, |ui| {
                    for e in events {
                        ui.colored_label(
                            if e.is_error {
                                egui::Color32::RED
                            } else {
                                egui::Color32::from_rgb(27, 130, 60)
                            },
                            &e.text,
                        );
                    }
                });
        }
    }

}

/// 加载中文字体。优先使用 Windows 自带的 TrueType 中文字体；
/// 可用 `NDM_FONT` 环境变量指定任意 .ttf。找不到时返回 false（界面给出提示）。
fn setup_fonts(ctx: &egui::Context) -> bool {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("NDM_FONT") {
        candidates.push(PathBuf::from(p));
    }
    if cfg!(windows) {
        for name in [
            "simhei.ttf",   // 黑体（TrueType，Win7+ 中文版自带）
            "Deng.ttf",     // 等线（Win10+）
            "Dengb.ttf",
            "simkai.ttf",   // 楷体
            "simfang.ttf",  // 仿宋
            "simsun.ttc",   // 宋体（TTC）
            "msyh.ttc",     // 微软雅黑（TTC，放最后）
        ] {
            candidates.push(PathBuf::from(format!(r"C:\Windows\Fonts\{name}")));
        }
        // 也扫描字体目录里的其它中文 TrueType 字体
        if let Ok(rd) = std::fs::read_dir(r"C:\Windows\Fonts") {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("ttf") {
                    candidates.push(p);
                }
            }
        }
    } else {
        for p in [
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ] {
            candidates.push(PathBuf::from(p));
        }
    }

    let mut font_bytes: Option<Vec<u8>> = None;
    for p in &candidates {
        if let Ok(data) = std::fs::read(p) {
            // 只接受 TrueType (glyf) 字体，避免 OTF/CFF 或 TTC 解析失败导致 egui panic
            if is_truetype(&data) {
                font_bytes = Some(data);
                log::info!("使用界面字体: {}", p.display());
                break;
            }
        }
    }
    let Some(data) = font_bytes else {
        log::warn!("未找到中文 TrueType 字体，可设置 NDM_FONT 指定");
        return false;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_string(), Arc::new(egui::FontData::from_owned(data)));
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
    ] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.insert(0, "cjk".to_string());
        }
    }
    ctx.set_fonts(fonts);
    true
}

fn is_truetype(data: &[u8]) -> bool {
    // 只接受 TrueType(glyf) 字体：OTF/CFF、TTC 合集在部分 egui 后端会解析失败
    data.len() > 12 && (&data[..4] == b"\x00\x01\x00\x00" || &data[..4] == b"true")
}

pub fn run_gui(state: Arc<AppState>, minimized: bool) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NDM-RS 下载管理器")
            .with_inner_size([1160.0, 720.0])
            .with_min_inner_size([940.0, 560.0])
            .with_visible(!minimized),
        ..Default::default()
    };
    let s = state.clone();
    let result = eframe::run_native(
        "NDM-RS",
        options,
        Box::new(move |cc| {
            let ok = setup_fonts(&cc.egui_ctx);
            let mut app = NdmApp::new(s.clone());
            app.font_warning = !ok;
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        eprintln!("图形界面启动失败：{e}");
        eprintln!("可使用 --headless 以无界面模式运行");
    }
}
