//! Bot trade-stream bridge: owns the per-user lifecycle of OKX trade streams
//! and the latest-wins dispatch queue that feeds running Bots.
//!
//! The bridge is a deep module with two injected seams: a `BotTradeSource`
//! (production: OKX WebSocket) and a `TradeSink` (production: the Bot decision
//! path). Neither seam touches `AppHandle`, so the queueing and reconciliation
//! logic is testable without a Tauri runtime.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use adaq_data_core::TradeStreamEvent;
use adaq_data_pipeline::okx::OkxSpotDataPath;

/// Effect seam: consumes one dispatched trade for a user. Errors are reported
/// through the return value and ignored by the dispatch loop, matching the
/// previous `let _ = dispatch_trade_event(...)` behaviour at the call site.
pub(crate) type TradeSink = Arc<dyn Fn(&str, &str, &str) -> Result<(), String> + Send + Sync>;

/// Continuation callback invoked by the stream source for every snapshot
/// trade; returning `false` asks the source to stop streaming.
pub(crate) type OnTrade = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Source seam: starts one trade stream for a user. Returns a handle the
/// bridge aborts when the desired instrument codes change.
pub(crate) trait BotTradeSource: Send + Sync {
    fn start(
        &self,
        user_id: &str,
        codes: &[String],
        on_trade: OnTrade,
    ) -> tauri::async_runtime::JoinHandle<()>;
}

/// Production source: forwards snapshot trades to the bridge callback over the
/// OKX spot data path.
pub(crate) struct OkxBotTradeSource {
    okx: OkxSpotDataPath,
}

impl OkxBotTradeSource {
    pub(crate) fn new(okx: OkxSpotDataPath) -> Self {
        Self { okx }
    }
}

impl BotTradeSource for OkxBotTradeSource {
    fn start(
        &self,
        user_id: &str,
        codes: &[String],
        on_trade: OnTrade,
    ) -> tauri::async_runtime::JoinHandle<()> {
        let okx = self.okx.clone();
        let user_id = user_id.to_owned();
        let codes = codes.to_vec();
        tauri::async_runtime::spawn(async move {
            let _ = okx
                .stream_trades(&user_id, &codes, |event| {
                    if let TradeStreamEvent::Snapshot(trade) = event {
                        on_trade(&trade.code, &trade.trade_id)
                    } else {
                        true
                    }
                })
                .await;
        })
    }
}

struct ActiveBotTradeStream {
    codes: BTreeSet<String>,
    task: tauri::async_runtime::JoinHandle<()>,
}

#[derive(Default)]
struct TradeDispatchQueue {
    pending: BTreeMap<String, String>,
    running: bool,
}

pub(crate) struct BotTradeBridge {
    source: Arc<dyn BotTradeSource>,
    sink: TradeSink,
    streams: Mutex<BTreeMap<String, ActiveBotTradeStream>>,
    dispatches: Arc<Mutex<BTreeMap<String, TradeDispatchQueue>>>,
}

impl BotTradeBridge {
    pub(crate) fn new(source: Arc<dyn BotTradeSource>, sink: TradeSink) -> Self {
        Self {
            source,
            sink,
            streams: Mutex::new(BTreeMap::new()),
            dispatches: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Reconcile the active trade stream for `user_id` with the desired
    /// instrument `codes`: keep an unchanged stream, abort a superseded one,
    /// and start a new one only for a non-empty code set.
    pub(crate) fn refresh(&self, user_id: &str, codes: BTreeSet<String>) -> Result<(), String> {
        let mut active = self.streams.lock().map_err(|error| error.to_string())?;
        if active
            .get(user_id)
            .is_some_and(|stream| stream.codes == codes)
        {
            return Ok(());
        }
        if let Some(previous) = active.remove(user_id) {
            previous.task.abort();
        }
        if codes.is_empty() {
            return Ok(());
        }

        let dispatches = Arc::clone(&self.dispatches);
        let sink = Arc::clone(&self.sink);
        let task_user_id = user_id.to_owned();
        let on_trade: OnTrade = Arc::new(move |instrument_code, trade_id| {
            enqueue_trade_dispatch(&dispatches, &sink, &task_user_id, instrument_code, trade_id)
                .is_ok()
        });
        let stream_codes = codes.iter().cloned().collect::<Vec<_>>();
        let task = self.source.start(user_id, &stream_codes, on_trade);
        active.insert(user_id.to_owned(), ActiveBotTradeStream { codes, task });
        Ok(())
    }

    /// Queue one trade for dispatch. Shared by every producer feeding the
    /// Bot decision path (Bot streams and UI trade streams alike).
    pub(crate) fn enqueue(
        &self,
        user_id: &str,
        instrument_code: &str,
        trade_id: &str,
    ) -> Result<(), String> {
        enqueue_trade_dispatch(
            &self.dispatches,
            &self.sink,
            user_id,
            instrument_code,
            trade_id,
        )
    }
}

/// Queue one trade per user and instrument (latest wins) and keep at most one
/// draining loop per user.
fn enqueue_trade_dispatch(
    dispatches: &Arc<Mutex<BTreeMap<String, TradeDispatchQueue>>>,
    sink: &TradeSink,
    user_id: &str,
    instrument_code: &str,
    trade_id: &str,
) -> Result<(), String> {
    let should_spawn = {
        let mut queues = dispatches.lock().map_err(|error| error.to_string())?;
        let queue = queues.entry(user_id.to_owned()).or_default();
        queue
            .pending
            .insert(instrument_code.to_owned(), trade_id.to_owned());
        if queue.running {
            false
        } else {
            queue.running = true;
            true
        }
    };
    if !should_spawn {
        return Ok(());
    }

    let dispatches = Arc::clone(dispatches);
    let sink = Arc::clone(sink);
    let task_user_id = user_id.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        loop {
            let next = {
                let mut queues = match dispatches.lock() {
                    Ok(queues) => queues,
                    Err(_) => return,
                };
                let Some(queue) = queues.get_mut(&task_user_id) else {
                    return;
                };
                let next = queue
                    .pending
                    .iter()
                    .next()
                    .map(|(instrument_code, trade_id)| (instrument_code.clone(), trade_id.clone()));
                if let Some((instrument_code, _)) = &next {
                    queue.pending.remove(instrument_code);
                } else {
                    queue.running = false;
                }
                next
            };
            let Some((instrument_code, trade_id)) = next else {
                break;
            };
            let _ = sink(&task_user_id, &instrument_code, &trade_id);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Poll;
    use std::time::{Duration, Instant};

    /// A future that never resolves and sets its flag when dropped, so an
    /// abort is observable no matter whether the task was polled first (an
    /// async-body Drop guard would miss an abort before the first poll).
    struct CancellingFuture {
        cancelled: Arc<AtomicBool>,
    }

    impl Future for CancellingFuture {
        type Output = ();

        fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }

    impl Drop for CancellingFuture {
        fn drop(&mut self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    struct RecordingSource {
        starts: Mutex<Vec<Vec<String>>>,
        cancelled: Mutex<Vec<Arc<AtomicBool>>>,
    }

    impl RecordingSource {
        fn new() -> Self {
            Self {
                starts: Mutex::new(Vec::new()),
                cancelled: Mutex::new(Vec::new()),
            }
        }

        fn starts(&self) -> Vec<Vec<String>> {
            self.starts.lock().unwrap().clone()
        }

        fn all_cancelled(&self) -> Vec<bool> {
            self.cancelled
                .lock()
                .unwrap()
                .iter()
                .map(|flag| flag.load(Ordering::SeqCst))
                .collect()
        }
    }

    impl BotTradeSource for RecordingSource {
        fn start(
            &self,
            _user_id: &str,
            codes: &[String],
            _on_trade: OnTrade,
        ) -> tauri::async_runtime::JoinHandle<()> {
            self.starts.lock().unwrap().push(codes.to_vec());
            let cancelled = Arc::new(AtomicBool::new(false));
            self.cancelled.lock().unwrap().push(Arc::clone(&cancelled));
            tauri::async_runtime::spawn(CancellingFuture { cancelled })
        }
    }

    #[derive(Clone, Default)]
    struct SinkLog {
        entries: Arc<Mutex<Vec<(String, String, String)>>>,
        in_flight: Arc<Mutex<BTreeMap<String, usize>>>,
        max_in_flight: Arc<Mutex<BTreeMap<String, usize>>>,
    }

    impl SinkLog {
        fn sink(&self) -> TradeSink {
            let log = self.clone();
            Arc::new(move |user_id, instrument_code, trade_id| {
                let now = {
                    let mut in_flight = log.in_flight.lock().unwrap();
                    let now = in_flight.entry(user_id.to_owned()).or_default();
                    *now += 1;
                    *now
                };
                log.max_in_flight
                    .lock()
                    .unwrap()
                    .entry(user_id.to_owned())
                    .and_modify(|max| *max = (*max).max(now))
                    .or_insert(now);
                log.entries.lock().unwrap().push((
                    user_id.to_owned(),
                    instrument_code.to_owned(),
                    trade_id.to_owned(),
                ));
                log.in_flight
                    .lock()
                    .unwrap()
                    .entry(user_id.to_owned())
                    .and_modify(|count| {
                        *count -= 1;
                    });
                Ok(())
            })
        }

        fn entries(&self) -> Vec<(String, String, String)> {
            self.entries.lock().unwrap().clone()
        }

        fn wait_for_entries(&self, count: usize) -> Vec<(String, String, String)> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let entries = self.entries();
                if entries.len() >= count || Instant::now() > deadline {
                    return entries;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn codes(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn wait_for_cancelled(source: &RecordingSource, index: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if source.all_cancelled()[index] {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("stream task at index {index} was not aborted");
    }

    #[test]
    fn trade_dispatch_queue_keeps_only_the_latest_trade_per_instrument() {
        let mut queue = TradeDispatchQueue::default();
        queue.pending.insert("BTC-USDT".into(), "trade-1".into());
        queue.pending.insert("ETH-USDT".into(), "trade-2".into());
        queue.pending.insert("BTC-USDT".into(), "trade-3".into());

        assert_eq!(queue.pending.len(), 2);
        assert_eq!(
            queue.pending.get("BTC-USDT").map(String::as_str),
            Some("trade-3")
        );
    }

    #[test]
    fn refresh_keeps_the_stream_when_codes_are_unchanged() {
        let source = Arc::new(RecordingSource::new());
        let bridge = BotTradeBridge::new(source.clone(), SinkLog::default().sink());

        bridge.refresh("user-a", codes(&["BTC-USDT"])).unwrap();
        bridge.refresh("user-a", codes(&["BTC-USDT"])).unwrap();

        assert_eq!(source.starts(), vec![vec!["BTC-USDT".to_owned()]]);
    }

    #[test]
    fn refresh_aborts_the_previous_stream_when_codes_change() {
        let source = Arc::new(RecordingSource::new());
        let bridge = BotTradeBridge::new(source.clone(), SinkLog::default().sink());

        bridge.refresh("user-a", codes(&["BTC-USDT"])).unwrap();
        bridge
            .refresh("user-a", codes(&["BTC-USDT", "ETH-USDT"]))
            .unwrap();

        assert_eq!(
            source.starts(),
            vec![
                vec!["BTC-USDT".to_owned()],
                vec!["BTC-USDT".to_owned(), "ETH-USDT".to_owned()],
            ]
        );
        wait_for_cancelled(&source, 0);
        assert!(!source.all_cancelled()[1]);
    }

    #[test]
    fn refresh_with_empty_codes_stops_the_stream_and_allows_a_new_start() {
        let source = Arc::new(RecordingSource::new());
        let bridge = BotTradeBridge::new(source.clone(), SinkLog::default().sink());

        bridge.refresh("user-a", codes(&["BTC-USDT"])).unwrap();
        bridge.refresh("user-a", codes(&[])).unwrap();
        bridge.refresh("user-a", codes(&["ETH-USDT"])).unwrap();

        assert_eq!(
            source.starts(),
            vec![vec!["BTC-USDT".to_owned()], vec!["ETH-USDT".to_owned()]]
        );
        wait_for_cancelled(&source, 0);
        assert!(!source.all_cancelled()[1]);
    }

    #[test]
    fn dispatch_keeps_latest_trade_per_instrument_and_drains_serially() {
        let log = SinkLog::default();
        let source = Arc::new(RecordingSource::new());
        let bridge = BotTradeBridge::new(source, log.sink());
        let dispatches = Arc::clone(&bridge.dispatches);

        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-a", "BTC-USDT", "trade-1").unwrap();
        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-a", "ETH-USDT", "trade-2").unwrap();
        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-a", "BTC-USDT", "trade-3").unwrap();

        let entries = log.wait_for_entries(2);
        assert_eq!(
            entries,
            vec![
                ("user-a".into(), "BTC-USDT".into(), "trade-3".into()),
                ("user-a".into(), "ETH-USDT".into(), "trade-2".into()),
            ]
        );
        assert_eq!(log.max_in_flight.lock().unwrap().get("user-a"), Some(&1));

        // The queue re-arms after the drain loop retires.
        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-a", "SOL-USDT", "trade-4").unwrap();
        let entries = log.wait_for_entries(3);
        assert_eq!(
            entries[2],
            ("user-a".into(), "SOL-USDT".into(), "trade-4".into())
        );
    }

    #[test]
    fn dispatch_keeps_user_queues_independent() {
        let log = SinkLog::default();
        let source = Arc::new(RecordingSource::new());
        let bridge = BotTradeBridge::new(source, log.sink());
        let dispatches = Arc::clone(&bridge.dispatches);

        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-a", "BTC-USDT", "trade-a").unwrap();
        enqueue_trade_dispatch(&dispatches, &log.sink(), "user-b", "BTC-USDT", "trade-b").unwrap();

        let entries = log.wait_for_entries(2);
        assert!(entries.contains(&("user-a".into(), "BTC-USDT".into(), "trade-a".into())));
        assert!(entries.contains(&("user-b".into(), "BTC-USDT".into(), "trade-b".into())));
        // Each user's own drain loop is serial; different users may run concurrently.
        let max_in_flight = log.max_in_flight.lock().unwrap();
        assert_eq!(max_in_flight.get("user-a"), Some(&1));
        assert_eq!(max_in_flight.get("user-b"), Some(&1));
    }
}
