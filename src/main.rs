// Release 版使用 Windows GUI 子系统：双击启动不弹出黑色控制台窗口。
// 从 cmd/PowerShell 启动 --headless 时仍会继承父进程的控制台，输出照常可见。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use ndm_rs::model::{AppState, DownloadRequest};
use ndm_rs::{app, engine, extension, settings, util};

#[derive(Debug, Default)]
struct Args {
    headless: bool,
    minimized: bool,
    port: u16,
    data_dir: Option<PathBuf>,
    save_dir: Option<PathBuf>,
    threads: Option<usize>,
    add_urls: Vec<String>,
    help: bool,
    version: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        port: extension::DEFAULT_PORT,
        ..Default::default()
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--minimized" => a.minimized = true,
            "--port" => {
                if let Some(v) = it.next() {
                    a.port = v.parse().unwrap_or(extension::DEFAULT_PORT);
                }
            }
            "--data-dir" => a.data_dir = it.next().map(PathBuf::from),
            "--dir" => a.save_dir = it.next().map(PathBuf::from),
            "--threads" => a.threads = it.next().and_then(|v| v.parse().ok()),
            "--add-url" => {
                if let Some(v) = it.next() {
                    a.add_urls.push(v);
                }
            }
            "--help" | "-h" => a.help = true,
            "--version" | "-V" => a.version = true,
            other => log::warn!("未知参数：{other}"),
        }
    }
    a
}

fn print_help() {
    println!(
        "NDM-RS — 兼容官方 NeatDownloadManager 扩展的 Rust 下载管理器\n\
         \n\
         用法: ndm-rs [选项]\n\
         \n\
           --headless            无窗口运行（仅启动下载引擎与扩展服务）\n\
           --port <端口>         扩展 WebSocket 端口，默认 10007\n\
           --data-dir <目录>     配置与任务记录目录\n\
           --add-url <URL>       启动时添加下载任务，可重复\n\
           --dir <目录>          --add-url 任务的保存目录\n\
           --threads <数量>      --add-url 任务的连接数\n\
           --minimized           启动后最小化到系统托盘\n\
           -h, --help            显示帮助\n\
           -V, --version         显示版本\n\
         \n\
         浏览器扩展：安装官方 NeatDownloadManager Extension 1.9.92 后\n\
         它会自动连接 ws://127.0.0.1:10007/download，无需额外配置。"
    );
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = parse_args();
    if args.help {
        print_help();
        return;
    }
    if args.version {
        println!("ndm-rs {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // 数据目录可通过 --data-dir 或 NDM_DATA_DIR 覆盖（便携版/测试用）
    if let Some(dir) = &args.data_dir {
        std::env::set_var("NDM_DATA_DIR", dir);
    }
    let data_dir = args.data_dir.clone().unwrap_or_else(settings::data_dir);
    let state = AppState::new(data_dir);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("创建 Tokio 运行时失败");

    rt.block_on(async {
        engine::spawn_scheduler(state.clone());
        tokio::spawn(extension::start_ws_server(state.clone(), args.port));
    });
    log::info!(
        "NDM-RS 已启动，数据目录 {:?}，扩展端口 {}",
        state.data_dir,
        args.port
    );

    for url in &args.add_urls {
        state.add_task(DownloadRequest {
            url: url.clone(),
            save_dir: args.save_dir.clone(),
            threads: args.threads,
            source: "命令行".to_string(),
            ..Default::default()
        });
    }

    if args.headless {
        println!("NDM-RS 无界面模式，扩展服务 ws://127.0.0.1:{}/download", args.port);
        println!("按 Ctrl-C 退出");
        rt.block_on(async {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let views = state.tasks_snapshot();
                let active = views.iter().filter(|v| v.state.is_active()).count();
                println!(
                    "[{}] 任务 {} | 进行中 {} | 已完成 {} | 速度 {} | 扩展 {}",
                    util::now_str(),
                    views.len(),
                    active,
                    views.iter().filter(|v| v.state == ndm_rs::model::TaskState::Done).count(),
                    util::format_speed(state.total_speed()),
                    if state.ext_connected() { "已连接" } else { "未连接" },
                );
            }
        });
    } else {
        app::run_gui(state, args.minimized);
    }
}
