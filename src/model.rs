use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::platform;
use crate::settings::Settings;
use crate::throttle::Throttle;
use crate::util;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Category {
    Video,
    Audio,
    Archive,
    Document,
    Program,
    #[default]
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Video => "视频",
            Category::Audio => "音频",
            Category::Archive => "压缩包",
            Category::Document => "文档",
            Category::Program => "程序",
            Category::Other => "其它",
        }
    }
    pub fn dir_name(self) -> &'static str {
        match self {
            Category::Video => "Videos",
            Category::Audio => "Audios",
            Category::Archive => "Compressed",
            Category::Document => "Documents",
            Category::Program => "Programs",
            Category::Other => "Others",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    #[default]
    Queued,
    Running,
    Paused,
    Done,
    Error,
    Canceled,
    Merging,
}

impl TaskState {
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Queued => "等待中",
            TaskState::Running => "下载中",
            TaskState::Paused => "已暂停",
            TaskState::Done => "已完成",
            TaskState::Error => "错误",
            TaskState::Canceled => "已取消",
            TaskState::Merging => "合并中",
        }
    }
    pub fn is_active(self) -> bool {
        matches!(self, TaskState::Queued | TaskState::Running | TaskState::Merging)
    }
    pub fn is_finished(self) -> bool {
        matches!(self, TaskState::Done | TaskState::Canceled)
    }
}

#[derive(Debug, Default)]
pub struct TaskControl {
    pub pause: AtomicBool,
    pub cancel: AtomicBool,
}

impl TaskControl {
    pub fn reset(&self) {
        self.pause.store(false, Ordering::Relaxed);
        self.cancel.store(false, Ordering::Relaxed);
    }
    pub fn should_pause(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }
    pub fn should_cancel(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// 一个下载分段的进度（start/end 为闭区间，字节单位）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub written: u64,
}

impl Segment {
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start) + 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn done(&self) -> bool {
        self.written >= self.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: u64,
    pub url: String,
    pub final_url: String,
    pub method: String,
    pub post_data: Option<String>,
    pub filename: String,
    pub save_dir: PathBuf,
    pub headers: Vec<(String, String)>,
    pub size: u64,
    pub downloaded: u64,
    pub speed: u64,
    pub state: TaskState,
    pub error: String,
    pub threads: usize,
    pub segments: Vec<Segment>,
    pub support_range: bool,
    pub is_hls: bool,
    pub category: Category,
    pub created_at: String,
    pub completed_at: String,
    pub source: String,
    pub media_stage: String,
    /// HLS 等任务按分片数计算进度时覆盖默认百分比
    pub progress_override: Option<f32>,
    #[serde(skip)]
    pub control: Arc<TaskControl>,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: 0,
            url: String::new(),
            final_url: String::new(),
            method: "GET".to_string(),
            post_data: None,
            filename: String::new(),
            save_dir: PathBuf::new(),
            headers: Vec::new(),
            size: 0,
            downloaded: 0,
            speed: 0,
            state: TaskState::Queued,
            error: String::new(),
            threads: 8,
            segments: Vec::new(),
            support_range: false,
            is_hls: false,
            category: Category::Other,
            created_at: String::new(),
            completed_at: String::new(),
            source: String::new(),
            media_stage: String::new(),
            progress_override: None,
            control: Arc::new(TaskControl::default()),
        }
    }
}

/// GUI 使用的只读快照。
#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: u64,
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub downloaded: u64,
    pub speed: u64,
    pub state: TaskState,
    pub error: String,
    pub threads: usize,
    pub source: String,
    pub category: Category,
    pub is_hls: bool,
    pub media_stage: String,
    pub created_at: String,
    pub progress: f32,
    pub eta: String,
}

impl Task {
    fn view(&self) -> TaskView {
        let progress = if let Some(p) = self.progress_override {
            p.clamp(0.0, 1.0)
        } else if self.size > 0 {
            (self.downloaded as f64 / self.size as f64).min(1.0) as f32
        } else {
            0.0
        };
        let eta = if self.size > self.downloaded {
            util::format_eta(self.size - self.downloaded, self.speed)
        } else {
            "--:--".to_string()
        };
        TaskView {
            id: self.id,
            filename: self.filename.clone(),
            url: self.url.clone(),
            size: self.size,
            downloaded: self.downloaded,
            speed: self.speed,
            state: self.state,
            error: self.error.clone(),
            threads: self.threads,
            source: self.source.clone(),
            category: self.category,
            is_hls: self.is_hls,
            media_stage: self.media_stage.clone(),
            created_at: self.created_at.clone(),
            progress,
            eta,
        }
    }
}

/// 从浏览器扩展 / 手动新建任务 / CLI 传入的统一下载请求。
#[derive(Debug, Clone, Default)]
pub struct DownloadRequest {
    pub url: String,
    pub method: String,
    pub filename: String,
    pub title: String,
    pub file_type: String,
    pub headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub size_hint: u64,
    pub content_type: String,
    pub source: String,
    pub threads: Option<usize>,
    pub save_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AppEvent {
    pub text: String,
    pub at: Instant,
    pub is_error: bool,
}

pub struct Inner {
    pub tasks: Vec<Task>,
    pub settings: Settings,
    pub next_id: u64,
    pub events: VecDeque<AppEvent>,
    speed_samples: HashMap<u64, (u64, Instant)>,
}

pub struct AppState {
    pub inner: Mutex<Inner>,
    pub throttle: Throttle,
    pub data_dir: PathBuf,
    pub ext_clients: AtomicUsize,
}

fn header_get(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&data_dir);
        let mut settings = Settings::load(&data_dir);
        settings.sanitize();

        let mut tasks: Vec<Task> = std::fs::read_to_string(data_dir.join("tasks.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Task>>(&s).ok())
            .unwrap_or_default();

        for t in tasks.iter_mut() {
            if matches!(t.state, TaskState::Running | TaskState::Merging) {
                t.state = if settings.auto_resume {
                    TaskState::Queued
                } else {
                    TaskState::Paused
                };
            }
            t.control.reset();
            t.speed = 0;
            if t.method.is_empty() {
                t.method = "GET".to_string();
            }
            if t.save_dir.as_os_str().is_empty() {
                t.save_dir = settings.download_dir.clone();
            }
        }

        let next_id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let throttle = Throttle::new();
        throttle.set_limit_kbps(settings.speed_limit_kbps);
        platform::set_auto_start(settings.auto_start);

        Arc::new(Self {
            inner: Mutex::new(Inner {
                tasks,
                settings,
                next_id,
                events: VecDeque::new(),
                speed_samples: HashMap::new(),
            }),
            throttle,
            data_dir,
            ext_clients: AtomicUsize::new(0),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ── 持久化 ────────────────────────────────────────────────────────────
    pub fn save_tasks(&self) {
        let tasks = {
            let g = self.lock();
            g.tasks.clone()
        };
        let tmp = self.data_dir.join("tasks.json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(&tasks) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, self.data_dir.join("tasks.json"));
            }
        }
    }

    pub fn save_all(&self) {
        self.save_tasks();
        let st = { self.lock().settings.clone() };
        st.save(&self.data_dir);
    }

    // ── 设置 ──────────────────────────────────────────────────────────────
    pub fn settings_snapshot(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn update_settings<F: FnOnce(&mut Settings)>(&self, f: F) {
        let st = {
            let mut g = self.lock();
            f(&mut g.settings);
            g.settings.sanitize();
            g.settings.clone()
        };
        self.throttle.set_limit_kbps(st.speed_limit_kbps);
        platform::set_auto_start(st.auto_start);
        st.save(&self.data_dir);
    }

    // ── 任务查询 ──────────────────────────────────────────────────────────
    pub fn tasks_snapshot(&self) -> Vec<TaskView> {
        self.lock().tasks.iter().map(|t| t.view()).collect()
    }

    pub fn task(&self, id: u64) -> Option<Task> {
        self.lock().tasks.iter().find(|t| t.id == id).cloned()
    }

    pub fn control(&self, id: u64) -> Option<Arc<TaskControl>> {
        self.lock()
            .tasks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.control.clone())
    }

    pub fn total_speed(&self) -> u64 {
        self.lock()
            .tasks
            .iter()
            .filter(|t| t.state == TaskState::Running)
            .map(|t| t.speed)
            .sum()
    }

    pub fn ext_connected(&self) -> bool {
        self.ext_clients.load(Ordering::Relaxed) > 0
    }

    pub fn inc_ext_clients(&self) {
        self.ext_clients.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_ext_clients(&self) {
        let mut cur = self.ext_clients.load(Ordering::Relaxed);
        while cur > 0 {
            match self.ext_clients.compare_exchange_weak(
                cur,
                cur - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    // ── 新增任务 ──────────────────────────────────────────────────────────
    pub fn add_task(&self, mut req: DownloadRequest) -> Option<u64> {
        req.url = req.url.trim().to_string();
        if req.url.is_empty() {
            return None;
        }
        if !(req.url.starts_with("http://") || req.url.starts_with("https://")) {
            self.push_event(format!("不支持的协议：{}", req.url), true);
            log::warn!("不支持的协议: {}", req.url);
            return None;
        }

        let st = self.settings_snapshot();
        let is_hls = req.file_type.eq_ignore_ascii_case("hls") || util::is_hls_url(&req.url);

        let mut filename = util::safe_filename(&req.filename);
        if filename.is_empty() || filename == "download" {
            filename = util::filename_from_url(&req.url).unwrap_or_else(|| {
                format!("download_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"))
            });
        }
        if is_hls {
            let stem = filename.rsplit_once('.').map(|(a, _)| a).unwrap_or(&filename);
            filename = format!("{stem}.ts");
        } else if !filename.contains('.') {
            let ext = header_get(&req.headers, "content-disposition")
                .and_then(|v| util::filename_from_content_disposition(&v))
                .and_then(|f| f.rsplit_once('.').map(|(_, e)| e.to_string()))
                .or_else(|| {
                    util::ext_from_content_type(&req.content_type).map(|s| s.to_string())
                })
                .or_else(|| {
                    util::ext_from_content_type(
                        &header_get(&req.headers, "content-type").unwrap_or_default(),
                    )
                    .map(|s| s.to_string())
                });
            if let Some(ext) = ext {
                filename = format!("{filename}.{ext}");
            }
        }
        let filename = util::safe_filename(&filename);
        let category = util::category_from_filename(&filename);

        let has_custom_dir = req.save_dir.is_some();
        let mut save_dir = req
            .save_dir
            .unwrap_or_else(|| st.download_dir.clone());
        if st.category_folders && !has_custom_dir {
            save_dir = save_dir.join(category.dir_name());
        }
        if let Err(e) = std::fs::create_dir_all(&save_dir) {
            self.push_event(format!("无法创建下载目录：{e}"), true);
            return None;
        }

        if req.method.is_empty() {
            req.method = "GET".to_string();
        }
        // 补齐常见请求头
        if header_get(&req.headers, "user-agent").is_none() && st.ua_for_all {
            req.headers
                .push(("User-Agent".to_string(), st.user_agent.clone()));
        }
        if header_get(&req.headers, "referer").is_none() {
            let page = req.title.clone();
            if page.starts_with("http://") || page.starts_with("https://") {
                req.headers.push(("Referer".to_string(), page));
            }
        }

        let t = Task {
            filename: filename.clone(),
            save_dir,
            url: req.url.clone(),
            final_url: req.url,
            method: req.method,
            post_data: req.post_data,
            headers: req.headers,
            size: req.size_hint,
            threads: req.threads.unwrap_or(st.max_connections),
            is_hls,
            category,
            created_at: util::now_str(),
            source: if req.source.is_empty() {
                "手动添加".to_string()
            } else {
                req.source
            },
            media_stage: if is_hls { "等待解析清单".to_string() } else { String::new() },
            ..Default::default()
        };

        let mut g = self.lock();
        let id = g.next_id;
        g.next_id += 1;
        let mut task = t;
        task.id = id;
        g.tasks.push(task);
        drop(g);

        self.push_event(format!("已添加下载：{filename}"), false);
        self.save_tasks();
        log::info!("新任务 #{id}: {filename}");
        Some(id)
    }

    // ── 引擎回调 ──────────────────────────────────────────────────────────
    pub fn set_state(&self, id: u64, state: TaskState) {
        {
            let mut g = self.lock();
            if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
                t.state = state;
                if state != TaskState::Running {
                    t.speed = 0;
                    g.speed_samples.remove(&id);
                }
            }
        }
        self.save_tasks();
    }

    pub fn set_meta(
        &self,
        id: u64,
        size: u64,
        filename: String,
        final_url: String,
        segments: Vec<Segment>,
        support_range: bool,
    ) {
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            t.size = size;
            t.filename = filename;
            t.final_url = final_url;
            t.segments = segments;
            t.support_range = support_range;
            if !t.segments.is_empty() {
                // 恢复任务时以各分段的实际落盘进度为准
                t.downloaded = t.segments.iter().map(|s| s.written).sum();
            }
            if t.size > 0 && t.size < t.downloaded {
                t.downloaded = t.size;
            }
        }
    }

    pub fn set_progress_override(&self, id: u64, progress: Option<f32>) {
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            t.progress_override = progress;
        }
    }

    pub fn set_media_stage(&self, id: u64, stage: &str) {
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            t.media_stage = stage.to_string();
        }
    }

    pub fn add_progress(&self, id: u64, seg_idx: usize, bytes: u64) {
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            t.downloaded = t.downloaded.saturating_add(bytes);
            if let Some(s) = t.segments.get_mut(seg_idx) {
                s.written = s.written.saturating_add(bytes);
            }
        }
    }

    /// 校正某个分段的已写字节数，并同步修正任务总进度。
    pub fn set_written(&self, id: u64, seg_idx: usize, written: u64) {
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(s) = t.segments.get_mut(seg_idx) {
                let old = s.written;
                s.written = written;
                t.downloaded =
                    (t.downloaded as i64 - old as i64 + written as i64).max(0) as u64;
            }
        }
    }

    pub fn finish(&self, id: u64, final_size: u64) {
        {
            let mut g = self.lock();
            let mut completed: Option<String> = None;
            if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
                t.state = TaskState::Done;
                t.downloaded = final_size.max(t.downloaded);
                t.size = final_size.max(t.size);
                t.speed = 0;
                t.media_stage = String::new();
                t.progress_override = Some(1.0);
                t.completed_at = util::now_str();
                completed = Some(t.filename.clone());
            }
            if let Some(name) = completed {
                g.speed_samples.remove(&id);
                if g.settings.show_completion_dialog {
                    g.events.push_back(AppEvent {
                        text: format!("下载完成：{name}"),
                        at: Instant::now(),
                        is_error: false,
                    });
                }
            }
        }
        self.save_tasks();
    }

    pub fn fail(&self, id: u64, msg: &str) {
        {
            let mut g = self.lock();
            let mut failed: Option<String> = None;
            if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
                t.state = TaskState::Error;
                t.error = msg.to_string();
                t.speed = 0;
                failed = Some(t.filename.clone());
            }
            if let Some(name) = failed {
                g.speed_samples.remove(&id);
                g.events.push_back(AppEvent {
                    text: format!("下载失败（{name}）：{msg}"),
                    at: Instant::now(),
                    is_error: true,
                });
            }
        }
        self.save_tasks();
    }

    pub fn tick_speeds(&self) {
        let now = Instant::now();
        let mut g = self.lock();
        let running: Vec<(u64, u64)> = g
            .tasks
            .iter()
            .filter(|t| t.state == TaskState::Running)
            .map(|t| (t.id, t.downloaded))
            .collect();
        let mut speeds: Vec<(u64, u64)> = Vec::new();
        for (id, downloaded) in &running {
            let entry = g.speed_samples.entry(*id).or_insert((*downloaded, now));
            let dt = now.duration_since(entry.1).as_secs_f64();
            if dt >= 0.4 {
                let delta = downloaded.saturating_sub(entry.0);
                speeds.push((*id, (delta as f64 / dt) as u64));
                *entry = (*downloaded, now);
            }
        }
        for t in g.tasks.iter_mut() {
            if let Some((_, sp)) = speeds.iter().find(|(id, _)| *id == t.id) {
                t.speed = *sp;
            }
        }
        let active: std::collections::HashSet<u64> = g
            .tasks
            .iter()
            .filter(|t| t.state == TaskState::Running)
            .map(|t| t.id)
            .collect();
        g.speed_samples.retain(|id, _| active.contains(id));
    }

    // ── 用户操作 ──────────────────────────────────────────────────────────
    pub fn pause(&self, id: u64) {
        if let Some(c) = self.control(id) {
            c.pause.store(true, Ordering::Relaxed);
        }
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            if t.state == TaskState::Queued {
                t.state = TaskState::Paused;
            }
        }
        drop(g);
        self.save_tasks();
    }

    pub fn resume(&self, id: u64) {
        if let Some(c) = self.control(id) {
            c.reset();
        }
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            if matches!(t.state, TaskState::Paused | TaskState::Error | TaskState::Canceled) {
                t.state = TaskState::Queued;
                t.error.clear();
            }
        }
        drop(g);
        self.save_tasks();
    }

    pub fn cancel(&self, id: u64) {
        if let Some(c) = self.control(id) {
            c.cancel.store(true, Ordering::Relaxed);
        }
        let mut g = self.lock();
        if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
            if t.state != TaskState::Running {
                t.state = TaskState::Canceled;
                t.speed = 0;
                g.speed_samples.remove(&id);
            }
        }
        drop(g);
        self.save_tasks();
    }

    pub fn remove(&self, id: u64) {
        if let Some(c) = self.control(id) {
            c.cancel.store(true, Ordering::Relaxed);
        }
        {
            let mut g = self.lock();
            g.tasks.retain(|t| t.id != id);
            g.speed_samples.remove(&id);
        }
        let dir = self.parts_dir(id);
        let _ = std::fs::remove_dir_all(dir);
        self.save_tasks();
    }

    pub fn clear_completed(&self) {
        let removed: Vec<u64> = {
            let mut g = self.lock();
            let ids: Vec<u64> = g
                .tasks
                .iter()
                .filter(|t| t.state == TaskState::Done)
                .map(|t| t.id)
                .collect();
            g.tasks.retain(|t| t.state != TaskState::Done);
            ids
        };
        for id in removed {
            let _ = std::fs::remove_dir_all(self.parts_dir(id));
        }
        self.save_tasks();
    }

    pub fn restart(&self, id: u64) {
        if let Some(c) = self.control(id) {
            c.cancel.store(true, Ordering::Relaxed);
            c.reset();
        }
        {
            let mut g = self.lock();
            if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
                t.state = TaskState::Queued;
                t.downloaded = 0;
                t.speed = 0;
                t.error.clear();
                t.segments.clear();
                t.support_range = false;
                t.progress_override = None;
                t.control = Arc::new(TaskControl::default());
            }
        }
        let _ = std::fs::remove_dir_all(self.parts_dir(id));
        self.save_tasks();
    }

    pub fn pause_all(&self) {
        let ids: Vec<u64> = self
            .lock()
            .tasks
            .iter()
            .filter(|t| t.state.is_active())
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        let ids: Vec<u64> = self
            .lock()
            .tasks
            .iter()
            .filter(|t| matches!(t.state, TaskState::Paused | TaskState::Error))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.resume(id);
        }
    }

    pub fn parts_dir(&self, id: u64) -> PathBuf {
        let st = self.settings_snapshot();
        let base = if st.temp_dir.as_os_str().is_empty() {
            st.download_dir.join(".ndm-rs-parts")
        } else {
            st.temp_dir.join("ndm-rs-parts")
        };
        base.join(id.to_string())
    }

    pub fn target_path(&self, task: &Task) -> PathBuf {
        util::unique_path(&task.save_dir, &task.filename)
    }

    // ── 事件（GUI 通知） ──────────────────────────────────────────────────
    pub fn push_event(&self, text: String, is_error: bool) {
        let mut g = self.lock();
        g.events.push_back(AppEvent {
            text,
            at: Instant::now(),
            is_error,
        });
        while g.events.len() > 12 {
            g.events.pop_front();
        }
    }

    pub fn recent_events(&self) -> Vec<AppEvent> {
        let g = self.lock();
        g.events
            .iter()
            .filter(|e| e.at.elapsed().as_secs() < 8)
            .rev()
            .take(3)
            .cloned()
            .collect()
    }
}
