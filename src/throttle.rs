use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 全局令牌桶限速器（所有任务、所有连接共享）。
/// 0 表示不限速；下载过程中修改设置即时生效。
pub struct Throttle {
    bytes_per_sec: AtomicU64,
    state: Mutex<Window>,
}

struct Window {
    tokens: f64,
    last: Instant,
}

impl Throttle {
    pub fn new() -> Self {
        Self {
            bytes_per_sec: AtomicU64::new(0),
            state: Mutex::new(Window {
                tokens: 0.0,
                last: Instant::now(),
            }),
        }
    }

    pub fn set_limit_kbps(&self, kbps: u64) {
        self.bytes_per_sec
            .store(kbps.saturating_mul(1024), Ordering::Relaxed);
    }

    pub fn limit_kbps(&self) -> u64 {
        self.bytes_per_sec.load(Ordering::Relaxed) / 1024
    }

    /// 消费 `bytes` 字节；令牌不足时异步等待。
    pub async fn consume(&self, bytes: usize) {
        let limit = self.bytes_per_sec.load(Ordering::Relaxed);
        if limit == 0 || bytes == 0 {
            return;
        }
        let lim = limit as f64;
        loop {
            let wait_secs = {
                let mut w = self.state.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(w.last).as_secs_f64();
                // 桶容量最多为 1 秒的额度，避免长时间不限速后突然爆发
                w.tokens = (w.tokens + elapsed * lim).min(lim);
                w.last = now;
                let need = bytes as f64;
                if w.tokens >= need {
                    w.tokens -= need;
                    return;
                }
                ((need - w.tokens) / lim).min(1.0)
            };
            if wait_secs <= 0.0 {
                return;
            }
            tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
        }
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new()
    }
}
