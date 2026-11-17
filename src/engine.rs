use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_TYPE, RANGE,
};
use reqwest::redirect::Policy;
use reqwest::{Client, Proxy, RequestBuilder, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::model::{AppState, Segment, Task, TaskControl, TaskState};
use crate::platform;
use crate::settings::{ProxyMode, Settings, DEFAULT_UA};
use crate::util;

const MIN_SEGMENT_BYTES: u64 = 256 * 1024;
const COPY_BUF: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub enum TaskOutcome {
    Done(u64),
    Paused,
    Canceled,
}

#[derive(Debug)]
pub(crate) enum Fail {
    Pause,
    Cancel,
    Msg(String),
}

impl From<reqwest::Error> for Fail {
    fn from(e: reqwest::Error) -> Self {
        Fail::Msg(e.to_string())
    }
}
impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Fail::Msg(e.to_string())
    }
}
impl From<String> for Fail {
    fn from(e: String) -> Self {
        Fail::Msg(e)
    }
}
impl From<&str> for Fail {
    fn from(e: &str) -> Self {
        Fail::Msg(e.to_string())
    }
}

struct Probe {
    final_url: String,
    size: u64,
    support_range: bool,
    filename_hint: Option<String>,
    content_type: String,
}

/// 启动调度器：控制同时下载的任务数、统计速度、周期性持久化进度。
pub fn spawn_scheduler(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_save = Instant::now();
        loop {
            state.tick_speeds();
            let max_simul = { state.lock().settings.max_simultaneous };
            let running = {
                state
                    .lock()
                    .tasks
                    .iter()
                    .filter(|t| t.state == TaskState::Running || t.state == TaskState::Merging)
                    .count()
            };
            if running < max_simul {
                let ids: Vec<u64> = {
                    let g = state.lock();
                    g.tasks
                        .iter()
                        .filter(|t| t.state == TaskState::Queued)
                        .take(max_simul - running)
                        .map(|t| t.id)
                        .collect()
                };
                for id in ids {
                    let s = state.clone();
                    tokio::spawn(async move {
                        s.set_state(id, TaskState::Running);
                        run_task(s.clone(), id).await;
                    });
                }
            }
            if last_save.elapsed() >= Duration::from_secs(3) {
                state.save_tasks();
                last_save = Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
}

/// 执行一个任务（普通 HTTP 或 HLS），并把结果写回状态。
pub async fn run_task(state: Arc<AppState>, id: u64) {
    let Some(task) = state.task(id) else { return };
    let control = task.control.clone();

    let result: Result<TaskOutcome, Fail> = if task.is_hls {
        crate::hls::run_hls_job(&state, id, &task, &control).await
    } else {
        run_http_task(&state, id, &task, &control).await
    };

    let outcome: Option<TaskOutcome> = match result {
        Ok(o) => Some(o),
        Err(Fail::Pause) => Some(TaskOutcome::Paused),
        Err(Fail::Cancel) => Some(TaskOutcome::Canceled),
        Err(Fail::Msg(m)) => {
            if control.should_cancel() {
                Some(TaskOutcome::Canceled)
            } else {
                log::error!("任务 #{id} 失败: {m}");
                state.fail(id, &m);
                None
            }
        }
    };

    match outcome {
        Some(TaskOutcome::Done(size)) => state.finish(id, size),
        Some(TaskOutcome::Paused) => {
            state.set_state(id, TaskState::Paused);
            state.save_tasks();
        }
        Some(TaskOutcome::Canceled) => {
            cleanup_parts(&state, id);
            state.set_state(id, TaskState::Canceled);
        }
        None => {}
    }
}

fn cleanup_parts(state: &Arc<AppState>, id: u64) {
    let _ = std::fs::remove_dir_all(state.parts_dir(id));
}

pub(crate) fn build_request_headers(task: &Task) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in &task.headers {
        // 这些是扩展提供的元信息 / hop-by-hop 头，不能作为请求头发送
        let blocked = k.eq_ignore_ascii_case("content-disposition")
            || k.eq_ignore_ascii_case("content-length")
            || k.eq_ignore_ascii_case("host")
            || k.eq_ignore_ascii_case("connection")
            || k.eq_ignore_ascii_case("transfer-encoding");
        if blocked {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v.trim()),
        ) {
            map.insert(name, value);
        }
    }
    map
}

pub(crate) fn build_client(st: &Settings, url: &str) -> Result<Client, String> {
    let scheme = url.split("://").next().unwrap_or("http");
    let mut b = Client::builder()
        .user_agent(if st.ua_for_all {
            st.user_agent.clone()
        } else {
            DEFAULT_UA.to_string()
        })
        .timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(15))
        .redirect(Policy::limited(10));

    match st.proxy_mode {
        ProxyMode::None => {
            b = b.no_proxy();
        }
        ProxyMode::System => {
            if let Some(p) = platform::system_proxy_for(scheme) {
                b = b.proxy(Proxy::all(p).map_err(|e| e.to_string())?);
            } else {
                b = b.no_proxy();
            }
        }
        ProxyMode::Http => {
            let raw = if scheme.eq_ignore_ascii_case("https") && !st.https_proxy.is_empty() {
                st.https_proxy.trim()
            } else {
                st.http_proxy.trim()
            };
            if raw.is_empty() {
                b = b.no_proxy();
            } else {
                let url = if raw.starts_with("http://") || raw.starts_with("https://") {
                    raw.to_string()
                } else {
                    format!("http://{raw}")
                };
                let mut p = Proxy::all(url).map_err(|e| e.to_string())?;
                if !st.proxy_user.is_empty() {
                    p = p.basic_auth(&st.proxy_user, &st.proxy_pass);
                }
                b = b.proxy(p);
            }
        }
        ProxyMode::Socks5 => {
            let raw = st.socks_proxy.trim();
            if raw.is_empty() {
                b = b.no_proxy();
            } else {
                let url = if raw.starts_with("socks5://") || raw.starts_with("socks://") {
                    raw.to_string()
                } else {
                    let mut p = format!("socks5://{raw}");
                    if !st.proxy_user.is_empty() {
                        p = format!(
                            "socks5://{}:{}@{raw}",
                            urlencoding(&st.proxy_user),
                            urlencoding(&st.proxy_pass)
                        );
                    }
                    p
                };
                b = b.proxy(Proxy::all(url).map_err(|e| e.to_string())?);
            }
        }
    }
    b.build().map_err(|e| e.to_string())
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn request(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    task: &Task,
) -> RequestBuilder {
    let is_post = task.method.eq_ignore_ascii_case("POST");
    if is_post {
        client
            .post(url)
            .headers(headers.clone())
            .body(task.post_data.clone().unwrap_or_default())
    } else {
        client.get(url).headers(headers.clone())
    }
}

async fn probe_file(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    task: &Task,
) -> Result<Probe, Fail> {
    // 用 Range 探测比 HEAD 更可靠：能同时拿到总大小、是否支持分段、重定向后的真实 URL。
    let is_post = task.method.eq_ignore_ascii_case("POST");
    let mut req = request(client, url, headers, task);
    if !is_post {
        req = req.header(RANGE, "bytes=0-0");
    }

    let mut resp = req.send().await?;
    let status = resp.status();
    if status != StatusCode::OK
        && status != StatusCode::PARTIAL_CONTENT
        && !status.is_success()
    {
        return Err(Fail::Msg(format!("服务器返回 HTTP {status}")));
    }
    let final_url = resp.url().to_string();
    let h = resp.headers().clone();
    let content_type = h
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let filename_hint = h
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(util::filename_from_content_disposition);
    let content_range_total = h
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(util::parse_content_range_total);
    let content_length = h
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(util::parse_content_length);
    let support_range = !is_post && status == StatusCode::PARTIAL_CONTENT;
    let size = content_range_total.or(content_length).unwrap_or(0);

    // 只读取一个 chunk 就丢弃响应，避免服务器忽略 Range 时把整个文件拉进内存。
    let _ = resp.chunk().await;
    drop(resp);

    Ok(Probe {
        final_url,
        size,
        support_range,
        filename_hint,
        content_type,
    })
}

fn choose_filename(task: &Task, probe: &Probe) -> String {
    let mut name = task.filename.clone();
    // 只有自动生成的占位名才允许被响应 Content-Disposition 覆盖；
    // 扩展/URL 给出的名字（即使没有扩展名）应被尊重，只做扩展名补全。
    let generic = name.is_empty() || name.starts_with("download_");
    if generic {
        if let Some(hint) = &probe.filename_hint {
            name = util::safe_filename(hint);
        } else if let Some(from_url) = util::filename_from_url(&probe.final_url) {
            if from_url.contains('.') {
                name = from_url;
            }
        }
    }
    if !name.contains('.') {
        if let Some(ext) = util::ext_from_content_type(&probe.content_type) {
            name = format!("{name}.{ext}");
        }
    }
    util::safe_filename(&name)
}

fn plan_segments(size: u64, threads: usize, existing: &[Segment]) -> Vec<Segment> {
    if !existing.is_empty()
        && existing.iter().map(|s| s.len()).sum::<u64>() == size
        && size > 0
    {
        return existing.to_vec();
    }
    let by_size = (size / MIN_SEGMENT_BYTES) as usize;
    let n = threads.max(1).min(by_size.max(1));
    let chunk = size / n as u64;
    (0..n)
        .map(|i| {
            let start = i as u64 * chunk;
            let end = if i == n - 1 {
                size - 1
            } else {
                (i as u64 + 1) * chunk - 1
            };
            Segment {
                start,
                end,
                written: 0,
            }
        })
        .collect()
}

pub(crate) async fn run_http_task(
    state: &Arc<AppState>,
    id: u64,
    task: &Task,
    control: &Arc<TaskControl>,
) -> Result<TaskOutcome, Fail> {
    if control.should_cancel() {
        return Ok(TaskOutcome::Canceled);
    }
    let st = state.settings_snapshot();
    let client = build_client(&st, &task.url)?;
    let headers = build_request_headers(task);
    let is_post = task.method.eq_ignore_ascii_case("POST");

    // 探测，失败自动重试（刚从浏览器接管链接时偶发 403/网络抖动）
    let mut probe = None;
    for attempt in 0..3u32 {
        if control.should_cancel() {
            return Ok(TaskOutcome::Canceled);
        }
        match probe_file(&client, &task.url, &headers, task).await {
            Ok(p) => {
                probe = Some(p);
                break;
            }
            Err(e) => {
                log::warn!("任务 #{id} 探测失败({attempt}): {e:?}");
                if attempt == 2 {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(700 * (attempt + 1) as u64)).await;
            }
        }
    }
    let probe = probe.ok_or("探测失败")?;

    let filename = choose_filename(task, &probe);
    let size = probe.size;
    let segments = plan_segments(size, task.threads, &task.segments);
    let multi = !is_post
        && probe.support_range
        && size >= MIN_SEGMENT_BYTES * 2
        && segments.len() > 1;

    state.set_meta(
        id,
        size,
        filename,
        probe.final_url.clone(),
        segments.clone(),
        probe.support_range,
    );

    // 分段文件目录
    let parts = state.parts_dir(id);
    std::fs::create_dir_all(&parts)?;

    if multi {
        download_all_segments(
            state,
            id,
            &client,
            task,
            &headers,
            &probe.final_url,
            &segments,
            &parts,
            control,
        )
        .await?;
        state.set_state(id, TaskState::Merging);
        let fresh = state.task(id).unwrap_or_else(|| task.clone());
        let total = merge_parts(&fresh, &parts, &segments).await?;
        Ok(TaskOutcome::Done(total))
    } else {
        let total = download_single(
            state,
            id,
            &client,
            task,
            &headers,
            &probe.final_url,
            size,
            probe.support_range,
            &parts,
            control,
        )
        .await?;
        Ok(TaskOutcome::Done(total))
    }
}

async fn open_part(path: &PathBuf, append: bool) -> Result<tokio::fs::File, Fail> {
    let f = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .await?;
    Ok(f)
}

/// 单个分段下载，支持断点续传与失败重试。
#[allow(clippy::too_many_arguments, clippy::ptr_arg)]
async fn download_segment(
    state: &Arc<AppState>,
    id: u64,
    idx: usize,
    seg: Segment,
    client: &Client,
    task: &Task,
    headers: &HeaderMap,
    final_url: &str,
    parts: &PathBuf,
    control: &Arc<TaskControl>,
) -> Result<(), Fail> {
    let path = parts.join(format!("part_{idx}.bin"));
    let mut written = seg.written.min(seg.len());

    for attempt in 0..5u32 {
        if control.should_cancel() {
            return Err(Fail::Cancel);
        }
        if control.should_pause() {
            return Err(Fail::Pause);
        }
        if written >= seg.len() {
            return Ok(());
        }

        let start = seg.start + written;
        let resp = request(client, final_url, headers, task)
            .header(RANGE, format!("bytes={start}-{}", seg.end))
            .send()
            .await;
        let mut resp = match resp {
            Ok(r) => r,
            Err(e) => {
                if attempt == 4 {
                    return Err(Fail::Msg(format!("分段 {idx} 连接失败: {e}")));
                }
                tokio::time::sleep(Duration::from_millis(500 * (attempt + 1) as u64)).await;
                continue;
            }
        };
        let status = resp.status();
        if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
            if attempt == 4 {
                return Err(Fail::Msg(format!("分段 {idx} 返回 HTTP {status}")));
            }
            tokio::time::sleep(Duration::from_millis(500 * (attempt + 1) as u64)).await;
            continue;
        }

        // 服务器忽略 Range 且我们已有进度：从头重写这一分段
        if status == StatusCode::OK && written > 0 {
            state.set_written(id, idx, 0);
            written = 0;
        }
        let append = written > 0 && status == StatusCode::PARTIAL_CONTENT;
        let mut file = open_part(&path, append).await?;

        while let Some(chunk) = resp.chunk().await? {
            if control.should_cancel() {
                file.flush().await.ok();
                return Err(Fail::Cancel);
            }
            if control.should_pause() {
                file.flush().await.ok();
                return Err(Fail::Pause);
            }
            state.throttle.consume(chunk.len()).await;
            let need = (seg.len() - written) as usize;
            let take = chunk.len().min(need);
            file.write_all(&chunk[..take]).await?;
            state.add_progress(id, idx, take as u64);
            written += take as u64;
            if written >= seg.len() {
                break;
            }
        }
        file.flush().await?;
        drop(file);

        if written >= seg.len() {
            return Ok(());
        }
        // 短读：下一轮从已有字节继续（Range + append）
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(Fail::Msg(format!("分段 {idx} 多次重试后仍未完成")))
}

#[allow(clippy::too_many_arguments, clippy::ptr_arg)]
async fn download_all_segments(
    state: &Arc<AppState>,
    id: u64,
    client: &Client,
    task: &Task,
    headers: &HeaderMap,
    final_url: &str,
    segments: &[Segment],
    parts: &PathBuf,
    control: &Arc<TaskControl>,
) -> Result<(), Fail> {
    let mut set = JoinSet::new();
    for (idx, seg) in segments.iter().copied().enumerate() {
        let s = state.clone();
        let c = client.clone();
        let t = task.clone();
        let h = headers.clone();
        let u = final_url.to_string();
        let p = parts.clone();
        let ctrl = control.clone();
        set.spawn(async move {
            download_segment(&s, id, idx, seg, &c, &t, &h, &u, &p, &ctrl).await
        });
    }
    let mut first_err: Option<Fail> = None;
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                // 通知其它分段尽快停下
                control.pause.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(Fail::Msg(e.to_string()));
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_single(
    state: &Arc<AppState>,
    id: u64,
    client: &Client,
    task: &Task,
    headers: &HeaderMap,
    final_url: &str,
    size: u64,
    support_range: bool,
    parts: &PathBuf,
    control: &Arc<TaskControl>,
) -> Result<u64, Fail> {
    let path = parts.join("part_0.bin");
    let is_post = task.method.eq_ignore_ascii_case("POST");
    let mut written: u64 = if support_range && !is_post && size > 0 {
        let on_disk = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if on_disk >= size {
            0
        } else {
            on_disk
        }
    } else {
        0
    };
    if written > 0 {
        state.set_written(id, 0, written);
    }

    for attempt in 0..4u32 {
        if control.should_cancel() {
            return Err(Fail::Cancel);
        }
        if control.should_pause() {
            return Err(Fail::Pause);
        }
        let mut req = request(client, final_url, headers, task);
        if written > 0 && support_range && !is_post {
            req = req.header(RANGE, format!("bytes={written}-"));
        }
        let mut resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                if attempt == 3 {
                    return Err(Fail::Msg(e.to_string()));
                }
                tokio::time::sleep(Duration::from_millis(500 * (attempt + 1) as u64)).await;
                continue;
            }
        };
        let status = resp.status();
        if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
            return Err(Fail::Msg(format!("服务器返回 HTTP {status}")));
        }
        if status == StatusCode::OK && written > 0 {
            state.set_written(id, 0, 0);
            written = 0;
        }
        let append = written > 0 && status == StatusCode::PARTIAL_CONTENT;
        let mut file = open_part(&path, append).await?;
        let mut got_eof = true;
        while let Some(chunk) = resp.chunk().await? {
            if control.should_cancel() {
                file.flush().await.ok();
                return Err(Fail::Cancel);
            }
            if control.should_pause() {
                file.flush().await.ok();
                return Err(Fail::Pause);
            }
            state.throttle.consume(chunk.len()).await;
            file.write_all(&chunk).await?;
            state.add_progress(id, 0, chunk.len() as u64);
            written += chunk.len() as u64;
        }
        if size > 0 && written < size && support_range && !is_post {
            got_eof = false;
        }
        file.flush().await?;
        drop(file);
        if got_eof {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let final_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(written);
    let fresh = state.task(id).unwrap_or_else(|| task.clone());
    let target = util::unique_path(&fresh.save_dir, &fresh.filename);
    if let Err(e) = tokio::fs::rename(&path, &target).await {
        log::warn!("跨分区移动失败，改用复制: {e}");
        tokio::fs::copy(&path, &target).await?;
        let _ = tokio::fs::remove_file(&path).await;
    }
    let _ = tokio::fs::remove_dir_all(parts).await;
    Ok(std::fs::metadata(&target).map(|m| m.len()).unwrap_or(final_size))
}

/// 按序把所有分段合并成最终文件，随后删除临时分片。
pub(crate) async fn merge_parts(
    task: &Task,
    parts: &PathBuf,
    segments: &[Segment],
) -> Result<u64, Fail> {
    let target = util::unique_path(&task.save_dir, &task.filename);
    let mut out = tokio::fs::File::create(&target).await?;
    let mut buf = vec![0u8; COPY_BUF];
    for idx in 0..segments.len() {
        let p = parts.join(format!("part_{idx}.bin"));
        let mut f = tokio::fs::File::open(&p).await?;
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
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(task.size);
    let _ = tokio::fs::remove_dir_all(parts).await;
    Ok(size)
}
