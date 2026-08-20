//! 限速器,对齐 ccxt Throttler(`ts/src/base/functions/throttle.ts`)语义。
//!
//! 两种算法:
//! - `LeakyBucket`:令牌以 `refill_rate`(个/秒)补充,容量 `capacity`;请求消耗 `cost` 个令牌。
//! - `RollingWindow`:滑动窗口内累计 `cost`,总量不超过窗口允许上限;超限则等待最旧记录过期。
//!
//! 每个交易所持有自己的 [`Throttler`],`cost` 由端点声明(默认 1)。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// 限速算法配置。
#[derive(Debug, Clone, Copy)]
pub enum ThrottleMode {
    /// 令牌桶。
    LeakyBucket {
        /// 桶容量(最大令牌数)。
        capacity: f64,
        /// 补充速率(令牌/秒)。
        refill_rate: f64,
    },
    /// 滑动窗口。
    RollingWindow {
        /// 窗口长度。
        window: Duration,
        /// 限速(请求/秒,窗口内允许 `window_secs * rate_limit` 的总 cost)。
        rate_limit: f64,
    },
}

struct LeakyState {
    tokens: f64,
    last_refill: Instant,
}

struct WindowEntry {
    at: Instant,
    cost: u32,
}

struct WindowState {
    entries: VecDeque<WindowEntry>,
    sum: u64,
}

struct Inner {
    mode: ThrottleMode,
    leaky: Mutex<LeakyState>,
    window: Mutex<WindowState>,
}

/// 线程安全的异步限速器。
pub struct Throttler {
    inner: Inner,
    notify: Notify,
}

impl Throttler {
    /// 构造限速器。
    pub fn new(mode: ThrottleMode) -> Self {
        Self {
            inner: Inner {
                mode,
                leaky: Mutex::new(LeakyState {
                    tokens: match mode {
                        ThrottleMode::LeakyBucket { capacity, .. } => capacity,
                        _ => 0.0,
                    },
                    last_refill: Instant::now(),
                }),
                window: Mutex::new(WindowState {
                    entries: VecDeque::new(),
                    sum: 0,
                }),
            },
            notify: Notify::new(),
        }
    }

    /// 按 `cost` 等待放行(异步)。
    pub async fn throttle(&self, cost: u32) {
        loop {
            let wait = {
                let inner = &self.inner;
                match inner.mode {
                    ThrottleMode::LeakyBucket { .. } => self.leaky_wait(cost),
                    ThrottleMode::RollingWindow { .. } => self.window_wait(cost),
                }
            };
            match wait {
                None => return,
                Some(delay) => {
                    self.notify.notify_waiters();
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// 返回需要等待的时长;`None` 表示放行。
    fn leaky_wait(&self, cost: u32) -> Option<Duration> {
        let ThrottleMode::LeakyBucket {
            capacity,
            refill_rate,
        } = self.inner.mode
        else {
            return None;
        };
        if refill_rate <= 0.0 {
            return Some(Duration::from_secs(1));
        }
        let mut state = self.inner.leaky.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        state.last_refill = now;
        // 容量只约束"瞬时突发";当 cost 超过容量时(罕见),缺口按
        // refill_rate 回补、不设上限,保证任意 cost 都收敛。
        let cap = if cost as f64 <= capacity {
            capacity
        } else {
            f64::INFINITY
        };
        state.tokens = (state.tokens + elapsed * refill_rate).min(cap);
        if state.tokens >= cost as f64 {
            state.tokens -= cost as f64;
            None
        } else {
            let needed = cost as f64 - state.tokens;
            Some(Duration::from_secs_f64(needed / refill_rate))
        }
    }

    fn window_wait(&self, cost: u32) -> Option<Duration> {
        let ThrottleMode::RollingWindow { window, rate_limit } = self.inner.mode else {
            return None;
        };
        if rate_limit <= 0.0 {
            return Some(Duration::from_secs(1));
        }
        let max_sum = (window.as_secs_f64() * rate_limit) as u64;
        let mut state = self.inner.window.lock().unwrap();
        let now = Instant::now();
        // 清理过期记录
        while let Some(front) = state.entries.front() {
            if now.duration_since(front.at) >= window {
                state.sum -= front.cost as u64;
                state.entries.pop_front();
            } else {
                break;
            }
        }
        let cost = cost as u64;
        if state.sum + cost <= max_sum.max(1) {
            state.sum += cost;
            state.entries.push_back(WindowEntry {
                at: now,
                cost: cost as u32,
            });
            None
        } else if let Some(front) = state.entries.front() {
            let until = front.at + window;
            Some(until.saturating_duration_since(now))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn leaky_bucket_basic() {
        // 容量 2,速率 2/秒:连续 4 个 cost=1 请求,总耗时 >= 1 秒
        let t = Throttler::new(ThrottleMode::LeakyBucket {
            capacity: 2.0,
            refill_rate: 2.0,
        });
        let start = Instant::now();
        for _ in 0..4 {
            t.throttle(1).await;
        }
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[tokio::test]
    async fn leaky_bucket_high_cost_waits() {
        // cost=5 > 容量 2:缺口 (5-2) 按速率 2/秒 回补,需等 1.5 秒
        let t = Throttler::new(ThrottleMode::LeakyBucket {
            capacity: 2.0,
            refill_rate: 2.0,
        });
        let start = Instant::now();
        t.throttle(5).await;
        assert!(start.elapsed() >= Duration::from_millis(1400));
    }

    #[tokio::test]
    async fn rolling_window_allows_burst_limited() {
        // 窗口 1 秒,限速 10/秒 → 窗口内最多 10 cost
        let t = Throttler::new(ThrottleMode::RollingWindow {
            window: Duration::from_secs(1),
            rate_limit: 10.0,
        });
        let start = Instant::now();
        for _ in 0..10 {
            t.throttle(1).await;
        }
        // 第 11 个需要等第一个过期
        t.throttle(1).await;
        assert!(start.elapsed() >= Duration::from_millis(900));
    }
}
