//! NDM-RS — 用 Rust 编写的 Windows 桌面下载管理器。
//!
//! 与官方 `NeatDownloadManager Extension 1.9.92` 完全兼容：
//! 监听 `ws://127.0.0.1:10007/download`，子协议 `neatextension.v1`。

pub mod app;
pub mod engine;
pub mod extension;
pub mod hls;
pub mod model;
pub mod platform;
pub mod settings;
pub mod throttle;
pub mod util;

pub use model::{AppState, DownloadRequest};
pub use settings::Settings;
