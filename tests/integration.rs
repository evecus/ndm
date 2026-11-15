//! 端到端测试：本地 HTTP/Range 服务器 + 下载引擎 + 官方扩展协议。
//! 全部使用本机回环地址，不依赖外网。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ndm_rs::engine;
use ndm_rs::extension;
use ndm_rs::model::{AppState, Category, DownloadRequest, Segment, TaskState};
use ndm_rs::settings::ProxyMode;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

fn parse_range(value: &str, total: usize) -> (usize, usize) {
    let spec = value.trim().strip_prefix("bytes=").unwrap_or("");
    let (a, b) = spec.split_once('-').unwrap_or(("0", ""));
    let start: usize = a.parse().unwrap_or(0);
    let end: usize = if b.is_empty() {
        total - 1
    } else {
        b.parse().unwrap_or(total - 1)
    };
    (start.min(total - 1), end.min(total - 1))
}

/// 极简 HTTP 服务器：`/file` 支持 Range 分段，其它路径直接返回内容。
fn spawn_server(routes: HashMap<String, Vec<u8>>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let ranges: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let routes = Arc::new(routes);
    let log = ranges.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                handle_conn(&mut stream, &routes, &log);
            });
        }
    });
    (port, ranges)
}

fn handle_conn(
    stream: &mut TcpStream,
    routes: &HashMap<String, Vec<u8>>,
    log: &Arc<Mutex<Vec<String>>>,
) {
    let mut req = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        req.extend_from_slice(&buf[..n]);
        if req.windows(4).any(|w| w == b"\r\n\r\n") || req.len() > 64 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&req);
    let first = text.lines().next().unwrap_or("");
    let mut it = first.split_whitespace();
    let method = it.next().unwrap_or("GET");
    let path = it.next().unwrap_or("/").split('?').next().unwrap_or("/");
    let range = text.lines().find_map(|l| {
        l.to_ascii_lowercase()
            .starts_with("range:")
            .then(|| l.split_once(':').unwrap().1.trim().to_string())
    });
    if let Some(r) = &range {
        log.lock().unwrap().push(r.clone());
    }

    if path == "/redirect" {
        let body = format!(
            "HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(body.as_bytes());
        return;
    }
    let Some(body) = routes.get(path) else {
        let _ = stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let total = body.len();
    let (start, end) = match &range {
        Some(r) => parse_range(r, total),
        None => (0, total - 1),
    };
    let partial = range.is_some() && path == "/file";
    let status = if partial {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let len = end - start + 1;
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"server.bin\"\r\n"
    );
    if partial {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    if method != "HEAD" {
        let _ = stream.write_all(&body[start..=end]);
    }
}

fn test_data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ndm_rs_test_{tag}_{nanos}"));
    std::fs::create_dir_all(dir.join("dl")).unwrap();
    dir
}

fn base_state(dir: &PathBuf) -> Arc<AppState> {
    let state = AppState::new(dir.clone());
    state.update_settings(|s| {
        s.download_dir = dir.join("dl");
        s.temp_dir = PathBuf::new();
        s.max_connections = 4;
        s.max_simultaneous = 3;
        s.speed_limit_kbps = 0;
        s.proxy_mode = ProxyMode::None;
        s.auto_resume = false;
        s.ua_for_all = false;
        s.show_completion_dialog = false;
    });
    state
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn segmented_download_merges_correctly() {
    let data = test_data(1024 * 1024 + 12345);
    let mut routes = HashMap::new();
    routes.insert("/file".to_string(), data.clone());
    let (port, ranges) = spawn_server(routes);
    let dir = temp_dir("seg");
    let state = base_state(&dir);
    let url = format!("http://127.0.0.1:{port}/file");

    let id = state
        .add_task(DownloadRequest {
            url: url.clone(),
            filename: "out.bin".to_string(),
            threads: Some(4),
            save_dir: Some(dir.join("dl")),
            source: "test".to_string(),
            ..Default::default()
        })
        .unwrap();
    state.set_state(id, TaskState::Running);
    engine::run_task(state.clone(), id).await;

    let t = state.task(id).unwrap();
    assert_eq!(t.state, TaskState::Done, "任务失败: {}", t.error);
    assert_eq!(t.size, data.len() as u64);
    let out = std::fs::read(dir.join("dl/out.bin")).unwrap();
    assert_eq!(out, data, "合并后的文件字节必须完全一致");
    assert!(!state.parts_dir(id).exists(), "临时分片应已清理");
    let ranges = ranges.lock().unwrap();
    assert!(ranges.len() >= 4, "应有多个 Range 请求: {ranges:?}");
    assert!(ranges.iter().any(|r| r.starts_with("bytes=0-0")), "应先做探测请求");
}

#[tokio::test]
async fn resumes_from_existing_part_files() {
    let data = test_data(700_000);
    let mut routes = HashMap::new();
    routes.insert("/file".to_string(), data.clone());
    let (port, ranges) = spawn_server(routes);
    let dir = temp_dir("resume");
    let state = base_state(&dir);
    let url = format!("http://127.0.0.1:{port}/file");
    let half = data.len() / 2;

    let id = state
        .add_task(DownloadRequest {
            url: url.clone(),
            filename: "resume.bin".to_string(),
            threads: Some(2),
            save_dir: Some(dir.join("dl")),
            ..Default::default()
        })
        .unwrap();

    // 模拟"上次下载到一半"：写入 part_0 并登记分段进度
    let parts = state.parts_dir(id);
    std::fs::create_dir_all(&parts).unwrap();
    std::fs::write(parts.join("part_0.bin"), &data[..half]).unwrap();
    state.set_meta(
        id,
        data.len() as u64,
        "resume.bin".to_string(),
        url.clone(),
        vec![
            Segment { start: 0, end: (half - 1) as u64, written: half as u64 },
            Segment { start: half as u64, end: (data.len() - 1) as u64, written: 0 },
        ],
        true,
    );
    assert_eq!(state.task(id).unwrap().downloaded, half as u64);

    state.set_state(id, TaskState::Running);
    engine::run_task(state.clone(), id).await;

    let t = state.task(id).unwrap();
    assert_eq!(t.state, TaskState::Done, "续传失败: {}", t.error);
    assert_eq!(std::fs::read(dir.join("dl/resume.bin")).unwrap(), data);
    let ranges = ranges.lock().unwrap();
    assert!(
        ranges.iter().any(|r| r.starts_with(&format!("bytes={half}-"))),
        "第二段应从 half 处继续: {ranges:?}"
    );
}

#[tokio::test]
async fn follows_redirect_and_single_connection_when_no_range() {
    let data = test_data(50_000);
    let mut routes = HashMap::new();
    routes.insert("/file".to_string(), data.clone());
    let (port, _) = spawn_server(routes);
    let dir = temp_dir("redir");
    let state = base_state(&dir);
    let id = state
        .add_task(DownloadRequest {
            url: format!("http://127.0.0.1:{port}/redirect"),
            filename: "redir.bin".to_string(),
            save_dir: Some(dir.join("dl")),
            ..Default::default()
        })
        .unwrap();
    state.set_state(id, TaskState::Running);
    engine::run_task(state.clone(), id).await;
    let t = state.task(id).unwrap();
    assert_eq!(t.state, TaskState::Done, "{}", t.error);
    assert_eq!(std::fs::read(dir.join("dl/redir.bin")).unwrap(), data);
}

#[tokio::test]
async fn official_extension_protocol_end_to_end() {
    let data = test_data(300_000);
    let mut routes = HashMap::new();
    routes.insert("/file".to_string(), data.clone());
    let (port, _) = spawn_server(routes);
    let dir = temp_dir("ws");
    let state = base_state(&dir);
    let ws_port = free_port();
    tokio::spawn(extension::start_ws_server(state.clone(), ws_port));
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 模拟官方扩展：带子协议连接 ws://127.0.0.1:10007/download
    let mut request = format!("ws://127.0.0.1:{ws_port}/download")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "neatextension.v1".parse().unwrap(),
    );
    let (mut ws, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .unwrap()
            .to_str()
            .unwrap(),
        "neatextension.v1",
        "服务端必须回显子协议，否则官方扩展会断开"
    );

    // 官方扩展 onmessage 期待的三条控制消息
    for expected in ["nowaiting", "Version 2.0", "ShowPanelChrome=1"] {
        let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(t) => assert_eq!(t.as_str(), expected),
            other => panic!("意外的控制消息: {other:?}"),
        }
    }

    let payload = format!(
        "1:GET\r\n2:http://127.0.0.1:{port}/file\r\n6:normal\r\n4:测试页面\r\nOrigin: http://example.com\r\nReferer: http://example.com/page\r\nCookie: sid=abc\r\n"
    );
    ws.send(Message::text(payload)).await.unwrap();

    let id = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(v) = state.tasks_snapshot().first() {
                break v.id;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("扩展消息应创建下载任务");

    let t = state.task(id).unwrap();
    assert_eq!(t.source, "浏览器扩展");
    assert!(t.headers.iter().any(|(k, v)| k == "Cookie" && v == "sid=abc"));
    assert_eq!(t.category, Category::Other);

    engine::run_task(state.clone(), id).await;
    let t = state.task(id).unwrap();
    assert_eq!(t.state, TaskState::Done, "{}", t.error);
    let expected_file = dir.join("dl").join("file.bin");
    assert!(expected_file.exists(), "应生成 file.bin");
    assert_eq!(std::fs::read(expected_file).unwrap(), data);
}

#[tokio::test]
async fn hls_playlist_is_merged() {
    let seg0 = test_data(10_000);
    let seg1 = test_data(20_000);
    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:10.0,\nseg0.ts\n#EXTINF:10.0,\nseg1.ts\n#EXT-X-ENDLIST\n"
    );
    let mut routes = HashMap::new();
    routes.insert("/index.m3u8".to_string(), playlist.into_bytes());
    routes.insert("/seg0.ts".to_string(), seg0.clone());
    routes.insert("/seg1.ts".to_string(), seg1.clone());
    let (port, _) = spawn_server(routes);
    let dir = temp_dir("hls");
    let state = base_state(&dir);

    let id = state
        .add_task(DownloadRequest {
            url: format!("http://127.0.0.1:{port}/index.m3u8"),
            file_type: "hls".to_string(),
            save_dir: Some(dir.join("dl")),
            ..Default::default()
        })
        .unwrap();
    state.set_state(id, TaskState::Running);
    engine::run_task(state.clone(), id).await;

    let t = state.task(id).unwrap();
    assert_eq!(t.state, TaskState::Done, "HLS 失败: {}", t.error);
    let mut expected = seg0.clone();
    expected.extend_from_slice(&seg1);
    assert_eq!(std::fs::read(dir.join("dl/index.ts")).unwrap(), expected);
}
