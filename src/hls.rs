//! HLS 支持：官方扩展捕获到 m3u8 时会推送 `6:hls` 任务。
//! 这里完成 master/media playlist 解析、AES-128 解密、分片并发下载与合并。
//! 若拿到的 URL 其实不是清单（例如扩展直接推了某个 .ts），回退到普通 HTTP 引擎。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aes::cipher::{Block, BlockCipherDecrypt, KeyInit};
use aes::Aes128;
use tokio::task::JoinSet;
use reqwest::header::{HeaderMap, RANGE};
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use crate::engine::{self, Fail, TaskOutcome};
use crate::model::{AppState, Task, TaskControl, TaskState};
use crate::util;

const CONCURRENT_SEGMENTS: usize = 8;
pub(crate) const NOT_A_PLAYLIST: &str = "__NOT_A_PLAYLIST__";

#[derive(Clone, Debug)]
struct KeyInfo {
    uri: String,
    iv: Option<[u8; 16]>,
}

#[derive(Clone, Debug)]
struct HlsSegment {
    url: String,
    key: Option<KeyInfo>,
    byterange: Option<(u64, u64)>,
    seq: u64,
}

#[derive(Debug, Default)]
pub struct Playlist {
    /// (绝对 URL, BANDWIDTH)
    variants: Vec<(String, u64)>,
    segments: Vec<HlsSegment>,
    init_uri: Option<String>,
    unsupported_encryption: bool,
}

fn absolute(base: &str, relative: &str) -> String {
    match Url::parse(base).and_then(|u| u.join(relative)) {
        Ok(u) => u.to_string(),
        Err(_) => relative.to_string(),
    }
}

fn attr(value: &str, name: &str) -> Option<String> {
    for part in value.split(',') {
        let Some((k, v)) = part.trim().split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(name) {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn parse_hex_iv(s: &str) -> Option<[u8; 16]> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_byterange(s: &str) -> Option<(u64, u64)> {
    let s = s.trim();
    if let Some((len, off)) = s.split_once('@') {
        Some((len.trim().parse().ok()?, off.trim().parse().ok()?))
    } else {
        Some((s.parse().ok()?, 0))
    }
}

/// 解析 m3u8 文本。非清单内容返回 `NOT_A_PLAYLIST`。
pub fn parse_playlist(text: &str, base: &str) -> Result<Playlist, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if !trimmed.starts_with("#EXTM3U") {
        return Err(NOT_A_PLAYLIST.to_string());
    }

    let mut pl = Playlist::default();
    let mut current_key: Option<KeyInfo> = None;
    let mut pending_byterange: Option<(u64, u64)> = None;
    let mut expect_variant = false;
    let mut pending_bandwidth = 0u64;
    let mut seq_start = 0u64;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if line.starts_with("#EXT-X-STREAM-INF") {
                expect_variant = true;
                pending_bandwidth = attr(
                    line.split_once(':').map(|(_, r)| r).unwrap_or(""),
                    "BANDWIDTH",
                )
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
                let method = attr(rest, "METHOD").unwrap_or_default();
                if method.eq_ignore_ascii_case("NONE") {
                    current_key = None;
                } else if method.eq_ignore_ascii_case("AES-128") {
                    let uri = attr(rest, "URI").unwrap_or_default();
                    current_key = Some(KeyInfo {
                        uri: absolute(base, &uri),
                        iv: attr(rest, "IV").and_then(|v| parse_hex_iv(&v)),
                    });
                } else {
                    pl.unsupported_encryption = true;
                }
            } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                seq_start = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
                pending_byterange = parse_byterange(rest);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
                if let Some(uri) = attr(rest, "URI") {
                    pl.init_uri = Some(absolute(base, &uri));
                }
            }
            continue;
        }

        if expect_variant {
            pl.variants
                .push((absolute(base, line), pending_bandwidth));
            pending_bandwidth = 0;
            expect_variant = false;
            continue;
        }
        pl.segments.push(HlsSegment {
            url: absolute(base, line),
            key: current_key.clone(),
            byterange: pending_byterange.take(),
            seq: seq_start + pl.segments.len() as u64,
        });
    }
    Ok(pl)
}

async fn resolve_playlist(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    depth: u32,
) -> Result<(Playlist, String), Fail> {
    let mut resp = client.get(url).headers(headers.clone()).send().await?;
    if !resp.status().is_success() {
        return Err(Fail::Msg(format!("获取 m3u8 失败：HTTP {}", resp.status())));
    }
    let base = resp.url().to_string();
    let mut text = String::new();
    while let Some(chunk) = resp.chunk().await? {
        text.push_str(&String::from_utf8_lossy(&chunk));
        if text.len() > 4 * 1024 * 1024 {
            break;
        }
    }
    let pl = parse_playlist(&text, &base)?;
    if !pl.variants.is_empty() && depth < 5 {
        // master playlist：选带宽最高的一条
        let best = pl
            .variants
            .iter()
            .max_by_key(|(_, bw)| *bw)
            .map(|(u, _)| u.clone())
            .unwrap_or_else(|| pl.variants[0].0.clone());
        return Box::pin(resolve_playlist(client, &best, headers, depth + 1)).await;
    }
    Ok((pl, base))
}

fn sequence_iv(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

/// AES-128 / CBC 解密（HLS 规范使用 PKCS#7 填充）。
fn aes128_cbc_decrypt(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>, String> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err("加密分片长度不是 16 的倍数".to_string());
    }
    let cipher = Aes128::new_from_slice(key).map_err(|_| "无效的 AES 密钥".to_string())?;
    let mut prev = *iv;
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks_exact(16) {
        let mut b = Block::<Aes128>::try_from(block).expect("分片长度应为 16 字节");
        cipher.decrypt_block(&mut b);
        for i in 0..16 {
            b[i] ^= prev[i];
        }
        out.extend_from_slice(&b);
        prev.copy_from_slice(block);
    }
    if let Some(&pad) = out.last() {
        let p = pad as usize;
        if p > 0 && p <= 16 && out.len() >= p && out[out.len() - p..].iter().all(|&x| x == pad) {
            out.truncate(out.len() - p);
        }
    }
    Ok(out)
}

type KeyCache = Arc<Mutex<HashMap<String, [u8; 16]>>>;

async fn fetch_key(
    client: &Client,
    headers: &HeaderMap,
    uri: &str,
    cache: &KeyCache,
) -> Result<[u8; 16], Fail> {
    if let Some(k) = cache.lock().unwrap().get(uri).copied() {
        return Ok(k);
    }
    let bytes = client
        .get(uri)
        .headers(headers.clone())
        .send()
        .await?
        .bytes()
        .await?;
    if bytes.len() < 16 {
        return Err(Fail::Msg("HLS 密钥长度不足 16 字节".to_string()));
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&bytes[..16]);
    cache.lock().unwrap().insert(uri.to_string(), key);
    Ok(key)
}

async fn fetch_segment(
    client: &Client,
    headers: &HeaderMap,
    seg: &HlsSegment,
    path: PathBuf,
    cache: KeyCache,
    state: &Arc<AppState>,
    control: &Arc<TaskControl>,
) -> Result<(), Fail> {
    if control.should_cancel() {
        return Err(Fail::Cancel);
    }
    if control.should_pause() {
        return Err(Fail::Pause);
    }
    let mut req = client.get(&seg.url).headers(headers.clone());
    if let Some((len, off)) = seg.byterange {
        req = req.header(RANGE, format!("bytes={off}-{}", off + len - 1));
    }
    let mut resp = req.send().await?;
    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Fail::Msg(format!("分片下载失败：HTTP {}", resp.status())));
    }
    let mut data = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if control.should_cancel() {
            return Err(Fail::Cancel);
        }
        state.throttle.consume(chunk.len()).await;
        data.extend_from_slice(&chunk);
    }
    if let Some(key) = &seg.key {
        let bytes = fetch_key(client, headers, &key.uri, &cache).await?;
        let iv = key.iv.unwrap_or_else(|| sequence_iv(seg.seq));
        data = aes128_cbc_decrypt(&data, &bytes, &iv).map_err(Fail::Msg)?;
    }
    tokio::fs::write(path, data).await?;
    Ok(())
}

/// 执行一个 HLS 任务。
pub(crate) async fn run_hls_job(
    state: &Arc<AppState>,
    id: u64,
    task: &Task,
    control: &Arc<TaskControl>,
) -> Result<TaskOutcome, Fail> {
    let st = state.settings_snapshot();
    let client = engine::build_client(&st, &task.url).map_err(Fail::Msg)?;
    let headers = engine::build_request_headers(task);
    state.set_media_stage(id, "解析 m3u8 清单");
    state.set_progress_override(id, Some(0.0));

    let (playlist, _base) = match resolve_playlist(&client, &task.url, &headers, 0).await {
        Ok(v) => v,
        Err(Fail::Msg(m)) if m == NOT_A_PLAYLIST => {
            // 不是清单（扩展有时直接推送分片 URL），退回普通下载引擎
            log::info!("任务 #{id} 不是 m3u8 清单，按普通文件下载");
            state.set_media_stage(id, "");
            state.set_progress_override(id, None);
            return engine::run_http_task(state, id, task, control).await;
        }
        Err(e) => return Err(e),
    };

    if playlist.unsupported_encryption {
        return Err(Fail::Msg("不支持的 HLS 加密方式（仅支持 AES-128）".to_string()));
    }
    if playlist.segments.is_empty() {
        return Err(Fail::Msg("m3u8 清单中没有可下载的分片".to_string()));
    }

    // fMP4（#EXT-X-MAP）需要输出 .mp4；MPEG-TS 输出 .ts
    let mut filename = task.filename.clone();
    if playlist.init_uri.is_some() {
        let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(&filename);
        filename = format!("{stem}.mp4");
    }
    let filename = util::safe_filename(&filename);
    state.set_meta(id, 0, filename, task.url.clone(), Vec::new(), false);

    let parts = state.parts_dir(id);
    std::fs::create_dir_all(&parts)?;
    let total = playlist.segments.len();
    let cache: KeyCache = Arc::new(Mutex::new(HashMap::new()));

    // init 段（fMP4）先取
    if let Some(init) = &playlist.init_uri {
        state.set_media_stage(id, "下载初始化分片");
        let bytes = client
            .get(init)
            .headers(headers.clone())
            .send()
            .await?
            .bytes()
            .await?;
        tokio::fs::write(parts.join("init.bin"), bytes).await?;
    }

    let mut done = 0usize;
    for (batch_no, batch) in playlist.segments.chunks(CONCURRENT_SEGMENTS).enumerate() {
        let mut set = JoinSet::new();
        for (offset, seg) in batch.iter().enumerate() {
            let idx = batch_no * CONCURRENT_SEGMENTS + offset;
            let c = client.clone();
            let h = headers.clone();
            let s = seg.clone();
            let p = parts.join(format!("seg_{idx:06}.bin"));
            let cache = cache.clone();
            let st = state.clone();
            let ctrl = control.clone();
            set.spawn(async move {
                fetch_segment(&c, &h, &s, p, cache, &st, &ctrl).await
            });
        }
        let mut first_err: Option<Fail> = None;
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(())) => {
                    done += 1;
                    state.set_progress_override(id, Some(done as f32 / total as f32));
                    state.set_media_stage(id, &format!("分片 {done}/{total}"));
                }
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(Fail::Msg(e.to_string()));
                    }
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
    }

    state.set_state(id, TaskState::Merging);
    state.set_media_stage(id, "合并分片");
    let fresh = state.task(id).unwrap_or_else(|| task.clone());
    let target = util::unique_path(&fresh.save_dir, &fresh.filename);
    let mut out = tokio::fs::File::create(&target).await?;
    let mut buf = vec![0u8; 64 * 1024];
    if playlist.init_uri.is_some() {
        let mut f = tokio::fs::File::open(parts.join("init.bin")).await?;
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).await?;
        }
    }
    for idx in 0..total {
        let mut f = tokio::fs::File::open(parts.join(format!("seg_{idx:06}.bin"))).await?;
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).await?;
        }
    }
    out.flush().await?;
    drop(out);
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let _ = tokio::fs::remove_dir_all(&parts).await;
    state.set_media_stage(id, "完成");
    Ok(TaskOutcome::Done(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_media_playlist() {
        let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://v.example.com/key.bin\",IV=0x00000000000000000000000000000001\n\
#EXT-X-MEDIA-SEQUENCE:5\n#EXTINF:10.0,\nseg5.ts\n#EXTINF:10.0,\nseg6.ts\n#EXT-X-ENDLIST\n";
        let pl = parse_playlist(text, "https://v.example.com/hls/index.m3u8").unwrap();
        assert_eq!(pl.segments.len(), 2);
        assert_eq!(pl.segments[0].url, "https://v.example.com/hls/seg5.ts");
        assert_eq!(pl.segments[0].seq, 5);
        assert_eq!(pl.segments[1].seq, 6);
        assert!(pl.segments[0].key.is_some());
        assert!(!pl.unsupported_encryption);
    }

    #[test]
    fn parses_master_playlist() {
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1920x1080\nhigh/index.m3u8\n";
        let pl = parse_playlist(text, "https://v.example.com/master.m3u8").unwrap();
        assert_eq!(pl.variants.len(), 2);
        assert_eq!(pl.variants[1].0, "https://v.example.com/high/index.m3u8");
        assert_eq!(pl.variants[1].1, 2_800_000);
        assert!(pl.segments.is_empty());
    }

    #[test]
    fn detects_not_playlist_and_sample_aes() {
        assert_eq!(
            parse_playlist("hello", "https://x/").unwrap_err(),
            NOT_A_PLAYLIST
        );
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:1,\na.ts\n";
        assert!(parse_playlist(text, "https://x/").unwrap().unsupported_encryption);
    }

    #[test]
    fn aes_decrypt_roundtrip() {
        use aes::cipher::BlockCipherEncrypt;
        let key = [0x11u8; 16];
        let iv = [0x22u8; 16];
        let mut plain = b"hello hls segment!!".to_vec();
        let pad = 16 - (plain.len() % 16);
        plain.extend(std::iter::repeat(pad as u8).take(pad));
        let cipher = Aes128::new_from_slice(&key).unwrap();
        let mut prev = iv;
        let mut enc = Vec::new();
        for block in plain.chunks_exact(16) {
            let mut b = Block::<Aes128>::try_from(block).expect("分片长度应为 16 字节");
            for i in 0..16 {
                b[i] ^= prev[i];
            }
            cipher.encrypt_block(&mut b);
            enc.extend_from_slice(&b);
            prev.copy_from_slice(&b);
        }
        let dec = aes128_cbc_decrypt(&enc, &key, &iv).unwrap();
        assert_eq!(dec, b"hello hls segment!!");
    }
}
