use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// 直连，不使用代理
    None,
    /// 跟随 Windows 系统代理设置（IE / 设置里的代理）
    #[default]
    System,
    /// 手动指定 HTTP 代理
    Http,
    /// 手动指定 SOCKS5 代理
    Socks5,
}

impl ProxyMode {
    pub fn label(self) -> &'static str {
        match self {
            ProxyMode::None => "不使用代理",
            ProxyMode::System => "跟随系统代理",
            ProxyMode::Http => "HTTP 代理",
            ProxyMode::Socks5 => "SOCKS5 代理",
        }
    }
    pub const ALL: [ProxyMode; 4] = [
        ProxyMode::None,
        ProxyMode::System,
        ProxyMode::Http,
        ProxyMode::Socks5,
    ];
}

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 单个下载任务的最大连接（分段）数
    pub max_connections: usize,
    /// 同时进行的任务数，其余排队
    pub max_simultaneous: usize,
    /// 全局限速，单位 KB/s，0 表示不限速
    pub speed_limit_kbps: u64,
    pub user_agent: String,
    /// 是否把自定义 UA 应用到所有请求
    pub ua_for_all: bool,
    pub download_dir: PathBuf,
    /// 按文件类型建立子目录
    pub category_folders: bool,
    /// 临时分片目录，空表示与下载目录相同
    pub temp_dir: PathBuf,
    pub show_completion_dialog: bool,
    /// Windows 开机自启
    pub auto_start: bool,
    /// 启动时自动恢复未完成的任务
    pub auto_resume: bool,
    /// 关闭窗口时最小化到托盘
    pub minimize_to_tray: bool,
    /// 监视剪贴板中的下载链接
    pub watch_clipboard: bool,
    /// 动态分段：下载中根据各段速度重新分配（保留设置项）
    pub dynamic_segmentation: bool,

    pub proxy_mode: ProxyMode,
    /// HTTP 代理，格式 host:port
    pub http_proxy: String,
    /// HTTPS 代理，留空则复用 http_proxy
    pub https_proxy: String,
    /// SOCKS5 代理，格式 host:port
    pub socks_proxy: String,
    pub proxy_user: String,
    pub proxy_pass: String,
}

impl Default for Settings {
    fn default() -> Self {
        let download_dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            max_connections: 8,
            max_simultaneous: 3,
            speed_limit_kbps: 0,
            user_agent: DEFAULT_UA.to_string(),
            ua_for_all: true,
            download_dir,
            category_folders: false,
            temp_dir: PathBuf::new(),
            show_completion_dialog: true,
            auto_start: false,
            auto_resume: true,
            minimize_to_tray: true,
            watch_clipboard: false,
            dynamic_segmentation: true,
            proxy_mode: ProxyMode::System,
            http_proxy: "127.0.0.1:7890".to_string(),
            https_proxy: String::new(),
            socks_proxy: "127.0.0.1:1080".to_string(),
            proxy_user: String::new(),
            proxy_pass: String::new(),
        }
    }
}

/// 数据目录：Windows 下为 `%APPDATA%\ndm-rs`。
/// 可用环境变量 `NDM_DATA_DIR` 覆盖（便于测试与便携版）。
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("NDM_DATA_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ndm-rs")
}

impl Settings {
    pub fn load(dir: &std::path::Path) -> Self {
        let path = dir.join("settings.json");
        match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(mut st) => {
                    st.sanitize();
                    st
                }
                Err(e) => {
                    log::warn!("settings.json 解析失败({e})，使用默认设置");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, dir: &std::path::Path) {
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join("settings.json.tmp");
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if std::fs::write(&tmp, s).is_ok() {
                    let _ = std::fs::rename(&tmp, dir.join("settings.json"));
                }
            }
            Err(e) => log::warn!("保存设置失败: {e}"),
        }
    }

    pub fn sanitize(&mut self) {
        self.max_connections = self.max_connections.clamp(1, 32);
        self.max_simultaneous = self.max_simultaneous.clamp(1, 16);
        if self.user_agent.trim().is_empty() {
            self.user_agent = DEFAULT_UA.to_string();
        }
        if self.download_dir.as_os_str().is_empty() {
            self.download_dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
        }
    }
}
