//! 浏览器扩展桥：兼容官方 `NeatDownloadManager Extension 1.9.92`。
//!
//! 官方扩展连接 `ws://127.0.0.1:10007/download`，子协议 `neatextension.v1`。
//! 握手成功后它期待服务端先发 `nowaiting`（表示由下载器自己探测文件大小）、
//! `Version ...` 与 `ShowPanelChrome=1`（启用网页媒体抓取面板）。
//! 之后扩展会以文本帧推送逐行字段构成的下载请求。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::model::{AppState, DownloadRequest};
use crate::util;

pub const DEFAULT_PORT: u16 = 10007;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// 解析官方扩展推送的逐行 payload。
///
/// 字段来自官方 bg.js 的 `V.I`：
/// `1:` 方法、`2:` URL、`3:` 文件名、`4:` 标题、`5:` 来源页、
/// `6:` 类型(normal/media/hls)、`7:` 大小、`8:` Content-Type、
/// `9:` 备注、`10:` 请求 Content-Type、`11:` Content-Disposition，
/// 以及 `Referer:`/`Cookie:`/`Origin:`/`X-*` 等请求头行。
/// 另外兼容 `fileName:`/`fEx:`/`fS:`/`fDu:` 与 POST 数据标记。
pub fn parse_ext_message(msg: &str) -> Option<DownloadRequest> {
    // POST 数据可能包含换行，且总是 payload 的最后一段
    let marker = "__0NeatPostData9__:";
    let (head, post_data) = match msg.find(marker) {
        Some(i) => (
            &msg[..i],
            Some(
                msg[i + marker.len()..]
                    .trim_end_matches(['\r', '\n'])
                    .to_string(),
            ),
        ),
        None => (msg, None),
    };

    let mut url = String::new();
    let mut method = "GET".to_string();
    let mut filename = String::new();
    let mut file_name_field = String::new();
    let mut title = String::new();
    let mut referrer_field = String::new();
    let mut file_type = "normal".to_string();
    let mut size_hint = 0u64;
    let mut content_type = String::new();
    let mut content_disposition = String::new();
    let mut f_ex = String::new();
    let mut headers: Vec<(String, String)> = Vec::new();

    for raw in head.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        // 数字字段（扩展协议主体）
        if key.len() <= 2 && key.bytes().all(|b| b.is_ascii_digit()) {
            match key {
                "1" => method = value.to_string(),
                "2" => url = value.to_string(),
                "3" => filename = value.to_string(),
                "4" => title = value.to_string(),
                "5" => referrer_field = value.to_string(),
                "6" => file_type = value.to_string(),
                "7" => size_hint = value.parse().unwrap_or(0),
                "8" => content_type = value.to_string(),
                "9" => {
                    if title.is_empty() {
                        title = value.to_string();
                    }
                }
                "10" => headers.push(("Content-Type".to_string(), value.to_string())),
                "11" => content_disposition = value.to_string(),
                _ => {}
            }
            continue;
        }

        // 兼容旧版/第三方扩展的补充字段
        match key {
            "fEx" | "fex" => {
                f_ex = value.to_string();
                continue;
            }
            "fileName" | "filename" => {
                file_name_field = value.to_string();
                continue;
            }
            "fS" => {
                if size_hint == 0 {
                    size_hint = value.parse().unwrap_or(0);
                }
                continue;
            }
            "fDu" | "fType" => continue,
            _ => {}
        }

        // 官方扩展把响应里的 Content-Disposition 作为独立头行推送（字段 11）。
        // 它绝不能作为请求头发送，但其中的 filename 是最可信的命名依据。
        if key.eq_ignore_ascii_case("content-disposition") {
            content_disposition = value.to_string();
            continue;
        }

        // HTTP 头行：只透传对下载请求有意义且安全的头
        if is_forwardable_header(key) {
            headers.push((key.to_string(), value.to_string()));
        }
    }

    if url.is_empty() {
        return None;
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    if method.is_empty() {
        method = "GET".to_string();
    }

    // 文件名优先级：显式字段 → Content-Disposition → URL 路径
    let mut fname = if !file_name_field.is_empty() {
        file_name_field
    } else {
        filename
    };
    if fname.is_empty() {
        fname = util::filename_from_content_disposition(&content_disposition).unwrap_or_default();
    }
    if fname.is_empty() {
        fname = util::filename_from_url(&url).unwrap_or_default();
    }
    if !fname.is_empty() && !fname.contains('.') && !f_ex.is_empty() {
        fname = format!("{fname}.{f_ex}");
    }

    // Referer 兜底：官方扩展一般会显式给 `Referer:` 行
    let has_referer = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("referer"));
    if !has_referer && !referrer_field.is_empty() {
        headers.push(("Referer".to_string(), referrer_field));
    }

    Some(DownloadRequest {
        url,
        method,
        filename: fname,
        title,
        file_type,
        headers,
        post_data,
        size_hint,
        content_type,
        source: "浏览器扩展".to_string(),
        threads: None,
        save_dir: None,
    })
}

fn is_forwardable_header(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    if matches!(
        l.as_str(),
        "content-disposition"
            | "content-length"
            | "content-range"
            | "host"
            | "connection"
            | "transfer-encoding"
            | "server"
            | "date"
            | "accept-ranges"
    ) {
        return false;
    }
    matches!(
        l.as_str(),
        "referer" | "referrer" | "cookie" | "origin" | "user-agent" | "accept"
            | "accept-language" | "accept-encoding" | "authorization" | "content-type"
            | "cache-control"
    ) || l.starts_with("x-")
}

fn ws_accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

fn header_value(request: &str, name: &str) -> Option<String> {
    for line in request.lines() {
        let line = line.trim_end_matches('\r');
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

async fn read_handshake(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut tmp = [0u8; 1024];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "握手前连接关闭",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos + 4]).to_string();
            let rest = buf[pos + 4..].to_vec();
            return Ok((head, rest));
        }
        if buf.len() > 32 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "握手请求过大",
            ));
        }
    }
}

struct ClientGuard(Arc<AppState>);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.dec_ext_clients();
    }
}

async fn handle_client(mut stream: TcpStream, state: Arc<AppState>) -> Result<(), String> {
    let (request, leftover) = read_handshake(&mut stream)
        .await
        .map_err(|e| e.to_string())?;

    let lower = request.to_ascii_lowercase();
    if !lower.contains("upgrade: websocket") || !lower.contains("connection: upgrade") {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Err("非 WebSocket 请求".to_string());
    }
    let key = header_value(&request, "sec-websocket-key").unwrap_or_default();
    if key.is_empty() {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Err("缺少 Sec-WebSocket-Key".to_string());
    }
    let accept = ws_accept_key(&key);
    // 官方扩展以 Sec-WebSocket-Protocol: neatextension.v1 发起，
    // 服务端必须回显该子协议，否则浏览器会直接断开连接。
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\
         Sec-WebSocket-Protocol: neatextension.v1\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut ws: WebSocketStream<TcpStream> =
        WebSocketStream::from_partially_read(stream, leftover, Role::Server, None).await;

    state.inc_ext_clients();
    let _guard = ClientGuard(state.clone());
    log::info!("浏览器扩展已连接（neatextension.v1）");

    // 官方扩展 onmessage：waiting / nowaiting / Version / ShowPanelChrome
    ws.send(Message::text("nowaiting"))
        .await
        .map_err(|e| e.to_string())?;
    ws.send(Message::text("Version 2.0"))
        .await
        .map_err(|e| e.to_string())?;
    ws.send(Message::text("ShowPanelChrome=1"))
        .await
        .map_err(|e| e.to_string())?;

    while let Some(frame) = ws.next().await {
        match frame {
            Ok(Message::Text(text)) => {
                if let Some(req) = parse_ext_message(text.as_str()) {
                    state.add_task(req);
                } else {
                    let preview: String = text.as_str().chars().take(120).collect();
                    log::debug!("忽略无法解析的扩展消息: {preview}");
                }
            }
            Ok(Message::Binary(bytes)) => {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    if let Some(req) = parse_ext_message(s) {
                        state.add_task(req);
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {} // Ping 由 tungstenite 自动回复 Pong
            Err(e) => {
                log::debug!("扩展连接读取结束: {e}");
                break;
            }
        }
    }
    Ok(())
}

/// 启动扩展监听服务（只绑定回环地址，其它机器无法访问）。
pub async fn start_ws_server(state: Arc<AppState>, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("无法监听 127.0.0.1:{port}: {e}");
            state.push_event(
                format!("扩展端口 {port} 被占用（是否已有 NDM-RS 或 NDM 在运行？）：{e}"),
                true,
            );
            return;
        }
    };
    log::info!("扩展桥已就绪：ws://127.0.0.1:{port}/download (neatextension.v1)");
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let s = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, s).await {
                        log::debug!("扩展连接关闭: {e}");
                    }
                });
            }
            Err(e) => {
                log::warn!("accept 失败: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_payload() {
        // 参考官方 bg.js V.I 生成的 payload（含页面头）
        let payload = concat!(
            "1:GET\r\n",
            "2:https://cdn.example.com/files/%E6%8A%A5%E5%91%8A.pdf?token=abc\r\n",
            "6:normal\r\n",
            "4:示例页面 - 标题\r\n",
            "Origin: https://example.com\r\n",
            "Referer: https://example.com/page.html\r\n",
            "5:https://example.com/page.html\r\n",
            "Cookie: session=xyz; theme=dark\r\n",
            "Content-Disposition: attachment; filename=\"report-2026.pdf\"\r\n",
        );
        let req = parse_ext_message(payload).expect("应解析成功");
        assert_eq!(req.method, "GET");
        assert!(req.url.starts_with("https://cdn.example.com/"));
        assert_eq!(req.filename, "report-2026.pdf");
        assert_eq!(req.file_type, "normal");
        assert_eq!(req.source, "浏览器扩展");
        assert!(req.headers.iter().any(|(k, v)| k == "Referer" && v.contains("page.html")));
        assert!(req.headers.iter().any(|(k, v)| k == "Cookie" && v.contains("session=xyz")));
        assert!(req.headers.iter().any(|(k, v)| k == "Origin" && v == "https://example.com"));
        // Content-Disposition 是元信息，不应作为请求头发送
        assert!(!req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-disposition")));
    }

    #[test]
    fn parses_hls_and_fex() {
        let payload = concat!(
            "1:GET\r\n",
            "2:https://v.example.com/hls/master.m3u8\r\n",
            "6:hls\r\n",
            "4:TS File 02 min 30 sec\r\n",
            "fEx:ts\r\n",
            "fileName:my-video\r\n",
            "Referer: https://v.example.com/\r\n",
            "7:0\r\n",
            "8:application/vnd.apple.mpegurl\r\n",
        );
        let req = parse_ext_message(payload).unwrap();
        assert_eq!(req.file_type, "hls");
        assert_eq!(req.filename, "my-video.ts");
        assert_eq!(req.content_type, "application/vnd.apple.mpegurl");
    }

    #[test]
    fn parses_post_data() {
        let payload = "1:POST\r\n2:https://api.example.com/dl\r\n6:normal\r\nContent-Type: application/x-www-form-urlencoded\r\n__0NeatPostData9__:id=42&name=test\n";
        let req = parse_ext_message(payload).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.post_data.as_deref(), Some("id=42&name=test"));
        assert!(req.headers.iter().any(|(k, v)| k == "Content-Type" && v.contains("form")));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_ext_message("hello world").is_none());
        assert!(parse_ext_message("2:ftp://x/y.zip\r\n").is_none());
    }

    #[test]
    fn accept_key_matches_rfc6455() {
        // RFC 6455 示例
        assert_eq!(
            ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
