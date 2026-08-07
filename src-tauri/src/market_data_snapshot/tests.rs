use super::*;
use adaq_data_core::{BarInterval, OhlcvBar};
use rusqlite::Connection;
use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn snapshot_module(name: &str) -> (PathBuf, MarketDataSnapshots) {
    let root = std::env::temp_dir().join(format!(
        "adaq-snapshot-module-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = Connection::open(root.join("adaq.db")).unwrap();
    let store = SnapshotStore::new(root.join("market-data")).unwrap();
    let source = Arc::new(LocalSnapshotSource::new(
        Arc::new(Mutex::new(database)),
        Arc::new(store),
    ));
    (root, MarketDataSnapshots::open(source).unwrap())
}

fn series(code: &str, open_time_ms: i64) -> BarSeries {
    BarSeries {
        src: "okx".into(),
        code: code.into(),
        interval: BarInterval::OneHour,
        bars: vec![bar(open_time_ms)],
        gaps: vec![],
    }
}

fn bar(open_time_ms: i64) -> OhlcvBar {
    OhlcvBar {
        open_time_ms,
        open: 1.into(),
        high: 1.into(),
        low: 1.into(),
        close: 1.into(),
        base_volume: 1.into(),
        quote_volume: 1.into(),
    }
}

/// Drives one future on a private runtime so parallel tests never share a
/// global executor.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    private_runtime().block_on(future)
}

fn private_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A minimal OKX-shaped HTTP stub; an optional delay before each response
/// keeps a download in flight long enough for the test to act on it.
fn serve_pages(bodies: Vec<String>, delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            let _request = String::from_utf8_lossy(&request[..size]);
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    format!("http://{address}")
}

const OKX_DAY_BAR_PAGE: &str =
    r#"{"code":"0","msg":"","data":[["1704067200000","1","2","0.5","1.5","1","1.5","1.5","1"]]}"#;

fn download_request(task_id: &str) -> SnapshotDownloadRequest {
    SnapshotDownloadRequest {
        user_id: "alice".into(),
        task_id: task_id.into(),
        src: "okx".into(),
        code: "BTC-USDT".into(),
        interval: BarInterval::OneDay,
        start_time_ms: 1_704_067_200_000,
        end_time_ms: 1_704_153_600_000,
    }
}

#[test]
fn snapshots_are_user_scoped_and_listed_by_matching_coverage() {
    let (root, module) = snapshot_module("access");
    let later = module
        .persist_for_user("alice", &series("BTC-USDT", 3_600_000))
        .unwrap();
    let earlier = module
        .persist_for_user("alice", &series("BTC-USDT", 0))
        .unwrap();
    let mut expected = vec![earlier.snapshot_id.clone(), later.snapshot_id.clone()];
    for hour in 2..12 {
        expected.push(
            module
                .persist_for_user("alice", &series("BTC-USDT", hour * 3_600_000))
                .unwrap()
                .snapshot_id,
        );
    }
    module
        .persist_for_user("bob", &series("BTC-USDT", 12 * 3_600_000))
        .unwrap();

    let listed = module
        .list(&SnapshotListRequest {
            user_id: "alice".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            page: 1,
        })
        .unwrap();
    assert_eq!(
        listed
            .items
            .iter()
            .map(|snapshot| &snapshot.snapshot_id)
            .collect::<Vec<_>>(),
        expected[..10].iter().collect::<Vec<_>>()
    );
    assert_eq!(listed.total, 12);
    let second = module
        .list(&SnapshotListRequest {
            user_id: "alice".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            page: 2,
        })
        .unwrap();
    assert_eq!(second.items.len(), 2);
    assert_eq!(
        second
            .items
            .iter()
            .map(|snapshot| &snapshot.snapshot_id)
            .collect::<Vec<_>>(),
        expected[10..].iter().collect::<Vec<_>>()
    );
    assert!(
        module
            .snapshot_for_user("bob", &earlier.snapshot_id)
            .is_err()
    );
    // Request validation happens inside the module before any network
    // access, so an unreachable client never gets used.
    let error = block_on(module.create_for_user(
        &SnapshotCreateRequest {
            user_id: "alice".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            start_time_ms: 1,
            end_time_ms: 1,
        },
        &OkxClient::new("http://127.0.0.1:9"),
    ))
    .unwrap_err();
    assert_eq!(error, "Snapshot time range is invalid");

    drop(module);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn readable_snapshots_are_user_scoped_and_ordered_for_cross_market_selection() {
    let (root, module) = snapshot_module("readable");
    module
        .persist_for_user("alice", &series("ETH-USDT", 3_600_000))
        .unwrap();
    module
        .persist_for_user("alice", &series("BTC-USDT", 0))
        .unwrap();
    module
        .persist_for_user("bob", &series("SOL-USDT", 0))
        .unwrap();

    assert_eq!(
        module
            .list_readable("alice")
            .unwrap()
            .iter()
            .map(|snapshot| snapshot.code.as_str())
            .collect::<Vec<_>>(),
        vec!["BTC-USDT", "ETH-USDT"]
    );
    assert!(module.list_readable(" ").is_err());

    drop(module);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn download_persists_the_snapshot_and_emits_progress_and_completion() {
    let (root, module) = snapshot_module("download");
    let base_url = serve_pages(vec![OKX_DAY_BAR_PAGE.to_owned()], Duration::ZERO);
    let events = Arc::new(Mutex::new(Vec::<SnapshotDownloadEvent>::new()));
    let snapshot = block_on(module.download_for_user(
        &download_request("task-1"),
        &OkxClient::new(base_url),
        |event| events.lock().unwrap().push(event),
    ))
    .unwrap();
    assert_eq!(snapshot.bar_count, 1);
    assert!(snapshot.parquet_path.is_file());

    let recorded = events.lock().unwrap();
    assert!(matches!(
        recorded.first(),
        Some(SnapshotDownloadEvent::Progress {
            downloaded_bars: 1,
            ..
        })
    ));
    assert!(matches!(
        recorded.last(),
        Some(SnapshotDownloadEvent::Completed { bar_count: 1, .. })
    ));
    drop(recorded);

    assert_eq!(module.list_readable("alice").unwrap().len(), 1);
    assert!(module.list_readable("bob").unwrap().is_empty());
    let (loaded, bars) = module
        .snapshot_for_user("alice", &snapshot.snapshot_id)
        .unwrap();
    assert_eq!(loaded.snapshot_id, snapshot.snapshot_id);
    assert_eq!(bars.len(), 1);

    drop(module);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn download_cancellation_stops_the_in_flight_download() {
    let (root, module) = snapshot_module("cancel");
    let base_url = serve_pages(
        vec![OKX_DAY_BAR_PAGE.to_owned()],
        Duration::from_millis(2_000),
    );
    let events = Arc::new(Mutex::new(Vec::<SnapshotDownloadEvent>::new()));
    // While the download below awaits its delayed HTTP response, a control
    // thread observes the in-flight map and signals cancellation. It polls
    // for the in-flight slot because the download's startup time is not
    // guaranteed; one shared runtime and client keep each probe cheap.
    let control_module = module.clone();
    let control = thread::spawn(move || {
        let runtime = private_runtime();
        let probe_client = OkxClient::new("http://127.0.0.1:9");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let duplicate = runtime
                .block_on(control_module.download_for_user(
                    &download_request("task-cancel"),
                    &probe_client,
                    |_| (),
                ))
                .unwrap_err();
            if duplicate == "Snapshot download is already in progress" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the in-flight download never registered: {duplicate}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        control_module.cancel_download("task-cancel").unwrap();
    });
    let download_events = events.clone();
    let error = block_on(module.download_for_user(
        &download_request("task-cancel"),
        &OkxClient::new(base_url),
        move |event| download_events.lock().unwrap().push(event),
    ))
    .unwrap_err();
    control.join().unwrap();
    assert!(error.contains("cancelled"));

    let recorded = events.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(matches!(recorded[0], SnapshotDownloadEvent::Cancelled));
    drop(recorded);
    // A cancelled download persists no Snapshot and frees its task slot.
    assert!(module.list_readable("alice").unwrap().is_empty());
    module.cancel_download("task-cancel").unwrap();

    drop(module);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_drops_only_the_reset_user_and_reports_orphaned_parquet() {
    let (root, module) = snapshot_module("reset");
    let exclusive = module
        .persist_for_user("alice", &series("BTC-USDT", 0))
        .unwrap();
    let shared = series("ETH-USDT", 0);
    module.persist_for_user("alice", &shared).unwrap();
    module.persist_for_user("bob", &shared).unwrap();

    let summary = module.summary_for_user("alice").unwrap();
    assert_eq!(summary.snapshot_count, 2);
    assert!(summary.market_data_bytes > 0);
    let mut database = module.0.source.database().unwrap();
    let orphaned = module.orphaned_parquet_paths(&database, "alice").unwrap();
    assert_eq!(orphaned, vec![exclusive.parquet_path.clone()]);
    assert!(orphaned[0].is_file());

    let transaction = database.transaction().unwrap();
    module.reset_for_user(&transaction, "alice").unwrap();
    transaction.commit().unwrap();
    drop(database);

    assert_eq!(module.summary_for_user("alice").unwrap().snapshot_count, 0);
    assert!(module.list_readable("alice").unwrap().is_empty());
    assert_eq!(module.list_readable("bob").unwrap().len(), 1);
    // File deletion belongs to the composition root's staged reset; the
    // module only reports which Parquet files become orphaned.
    assert!(exclusive.parquet_path.is_file());

    drop(module);
    fs::remove_dir_all(root).unwrap();
}
