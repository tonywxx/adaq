mod m3;
#[allow(dead_code)] // M2 is host-only until Backtest orchestration consumes it.
mod run_engine;
mod watchlist;

use adaq_data_core::{
    BarInterval, BarSeries, BarStreamEvent, BarSubscription, DataError, HistoricalBarRange,
    InstrumentStatus, OkxClient, SpotInstrument, TickerSnapshot, TickerStreamEvent,
};
#[cfg(test)]
use adaq_component_sdk::host::{factor_abi, strategy_abi};
use adaq_component_tooling::{FactorSchema, WasmLoader};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{
    Emitter, Manager, State,
    ipc::Channel,
    menu::{AboutMetadata, MenuBuilder, SubmenuBuilder},
};
use watchlist::{InstrumentRef, WatchlistDb, WatchlistState};

use m3::M3State;

const CHECK_FOR_UPDATES_MENU_ID: &str = "check_for_updates";
const CHECK_FOR_UPDATES_EVENT: &str = "adaq-check-for-updates";

fn database_path(app_data_dir: &Path) -> Result<PathBuf, std::io::Error> {
    let current = app_data_dir.join("adaq.db");
    let legacy = app_data_dir.join("adaq.sqlite3");
    // Remove the legacy branch after two releases containing this migration.
    if !current.exists() && legacy.exists() {
        std::fs::rename(legacy, &current)?;
    }
    Ok(current)
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn load_factor_component(
    path: String,
    loader: State<'_, WasmLoader>,
) -> Result<FactorSchema, String> {
    loader.load(&path)?;
    loader.describe_factor()
}

#[tauri::command]
fn get_factor_schema(loader: State<'_, WasmLoader>) -> Result<FactorSchema, String> {
    loader.describe_factor()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSourceRequest {
    src: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketGetBarSeriesRequest {
    src: String,
    code: String,
    interval: BarInterval,
    start_time_ms: i64,
    end_time_ms: i64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketTickerRequest {
    src: String,
    code: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSubscribeTickersRequest {
    src: String,
    codes: Vec<String>,
    subscription_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketUnsubscribeTickerRequest {
    subscription_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSubscribeBarsRequest {
    src: String,
    subscriptions: Vec<BarSubscription>,
    subscription_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketUnsubscribeBarRequest {
    subscription_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistUserRequest {
    user_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistInstrumentRequest {
    user_id: String,
    instrument: InstrumentRef,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistIntervalRequest {
    user_id: String,
    interval: BarInterval,
}

struct ActiveTickerStream {
    subscription_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    on_event: Channel<TickerStreamEvent>,
}

#[derive(Default)]
struct TickerStreamState(Mutex<Option<ActiveTickerStream>>);

struct ActiveBarStream {
    subscription_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    on_event: Channel<BarStreamEvent>,
}

#[derive(Default)]
struct BarStreamState(Mutex<Option<ActiveBarStream>>);

fn require_okx(src: &str) -> Result<(), DataError> {
    if src == "okx" {
        Ok(())
    } else {
        Err(DataError::new(
            src,
            "unsupported_src",
            format!("unsupported market data source: {src}"),
        ))
    }
}

#[tauri::command]
async fn market_list_spot_instruments(
    request: MarketSourceRequest,
    client: State<'_, OkxClient>,
) -> Result<Vec<SpotInstrument>, DataError> {
    require_okx(&request.src)?;
    client.list_spot_instruments().await
}

#[tauri::command]
async fn market_get_bar_series(
    request: MarketGetBarSeriesRequest,
    client: State<'_, OkxClient>,
) -> Result<BarSeries, DataError> {
    require_okx(&request.src)?;
    client
        .get_bar_series_range(
            &request.code,
            request.interval,
            HistoricalBarRange {
                start_time_ms: request.start_time_ms,
                end_time_ms: request.end_time_ms,
            },
        )
        .await
}

#[tauri::command]
async fn market_get_ticker(
    request: MarketTickerRequest,
    client: State<'_, OkxClient>,
) -> Result<TickerSnapshot, DataError> {
    require_okx(&request.src)?;
    client.get_ticker(&request.code).await
}

#[tauri::command]
fn watchlist_get(
    request: WatchlistUserRequest,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    database.get(&request.user_id)
}

#[tauri::command]
async fn watchlist_add(
    request: WatchlistInstrumentRequest,
    database: State<'_, WatchlistDb>,
    client: State<'_, OkxClient>,
) -> Result<WatchlistState, String> {
    require_okx(&request.instrument.src).map_err(|error| error.to_string())?;
    let instruments = client
        .list_spot_instruments()
        .await
        .map_err(|error| error.to_string())?;
    if !instruments.iter().any(|instrument| {
        instrument.code == request.instrument.code && instrument.status == InstrumentStatus::Live
    }) {
        return Err("only Live OKX Spot Instruments can be added".to_owned());
    }
    database.add(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_remove(
    request: WatchlistInstrumentRequest,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    database.remove(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_set_active(
    request: WatchlistInstrumentRequest,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    database.set_active(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_set_interval(
    request: WatchlistIntervalRequest,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    database.set_interval(&request.user_id, request.interval)
}

#[tauri::command]
fn market_subscribe_tickers(
    request: MarketSubscribeTickersRequest,
    on_event: Channel<TickerStreamEvent>,
    client: State<'_, OkxClient>,
    streams: State<'_, TickerStreamState>,
) -> Result<(), DataError> {
    require_okx(&request.src)?;
    if request.subscription_id.trim().is_empty() || !(1..=32).contains(&request.codes.len()) {
        return Err(DataError::new(
            request.src,
            "invalid_request",
            "subscription ID must be non-empty and ticker codes must contain 1 to 32 items",
        ));
    }

    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    // ponytail: infrequent subscription-set changes restart the one multiplexed socket;
    // move subscribe/unsubscribe messages into a long-lived actor if churn becomes measurable.
    let task_client = client.inner().clone();
    let task_channel = on_event.clone();
    let codes = request.codes;
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = task_client
            .stream_tickers(&codes, |event| task_channel.send(event).is_ok())
            .await
        {
            let _ = task_channel.send(TickerStreamEvent::Error(error));
        }
    });

    if let Some(previous) = active.replace(ActiveTickerStream {
        subscription_id: request.subscription_id,
        task,
        on_event,
    }) {
        let _ = previous.on_event.send(TickerStreamEvent::Closed);
        previous.task.abort();
    }
    Ok(())
}

#[tauri::command]
fn market_unsubscribe_ticker(
    request: MarketUnsubscribeTickerRequest,
    streams: State<'_, TickerStreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    if active
        .as_ref()
        .is_some_and(|stream| stream.subscription_id == request.subscription_id)
    {
        let stream = active.take().expect("active ticker stream disappeared");
        let _ = stream.on_event.send(TickerStreamEvent::Closed);
        stream.task.abort();
    }
    Ok(())
}

#[tauri::command]
fn market_subscribe_bars(
    request: MarketSubscribeBarsRequest,
    on_event: Channel<BarStreamEvent>,
    client: State<'_, OkxClient>,
    streams: State<'_, BarStreamState>,
) -> Result<(), DataError> {
    require_okx(&request.src)?;
    if request.subscription_id.trim().is_empty() || !(1..=32).contains(&request.subscriptions.len())
    {
        return Err(DataError::new(
            request.src,
            "invalid_request",
            "subscription ID must be non-empty and bar subscriptions must contain 1 to 32 items",
        ));
    }

    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    let task_client = client.inner().clone();
    let task_channel = on_event.clone();
    let subscriptions = request.subscriptions;
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = task_client
            .stream_bars(&subscriptions, |event| task_channel.send(event).is_ok())
            .await
        {
            let _ = task_channel.send(BarStreamEvent::Error(error));
        }
    });

    if let Some(previous) = active.replace(ActiveBarStream {
        subscription_id: request.subscription_id,
        task,
        on_event,
    }) {
        let _ = previous.on_event.send(BarStreamEvent::Closed);
        previous.task.abort();
    }
    Ok(())
}

#[tauri::command]
fn market_unsubscribe_bar(
    request: MarketUnsubscribeBarRequest,
    streams: State<'_, BarStreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    if active
        .as_ref()
        .is_some_and(|stream| stream.subscription_id == request.subscription_id)
    {
        let stream = active.take().expect("active bar stream disappeared");
        let _ = stream.on_event.send(BarStreamEvent::Closed);
        stream.task.abort();
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .setup(|app| {
            app.manage(WasmLoader::default());
            app.manage(OkxClient::default());
            app.manage(TickerStreamState::default());
            app.manage(BarStreamState::default());
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = database_path(&app_data_dir)?;
            app.manage(M3State::open(&app_data_dir).map_err(std::io::Error::other)?);
            app.manage(WatchlistDb::open(&database_path).map_err(std::io::Error::other)?);
            let handle = app.handle();
            let app_menu = SubmenuBuilder::new(handle, "adaq")
                .about(Some(AboutMetadata {
                    name: Some("adaq".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    authors: Some(vec!["TONy.W".into()]),
                    comments: Some("AI Quant Trading".into()),
                    ..Default::default()
                }))
                .text(CHECK_FOR_UPDATES_MENU_ID, "Check for Updates...")
                .separator()
                .services()
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;
            let edit_menu = SubmenuBuilder::new(handle, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;
            let window_menu = SubmenuBuilder::new(handle, "Window")
                .minimize()
                .fullscreen()
                .separator()
                .close_window()
                .build()?;
            let menu = MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&edit_menu)
                .item(&window_menu)
                .build()?;

            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                if event.id() == CHECK_FOR_UPDATES_MENU_ID {
                    if let Err(error) = app.emit_to("main", CHECK_FOR_UPDATES_EVENT, ()) {
                        eprintln!("failed to emit update check event: {error}");
                    }
                }
            });

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            load_factor_component,
            get_factor_schema,
            market_list_spot_instruments,
            market_get_bar_series,
            market_get_ticker,
            watchlist_get,
            watchlist_add,
            watchlist_remove,
            watchlist_set_active,
            watchlist_set_interval,
            market_subscribe_tickers,
            market_unsubscribe_ticker,
            market_subscribe_bars,
            market_unsubscribe_bar,
            m3::component_import,
            m3::component_list,
            m3::component_page,
            m3::backtest_compatible_factors,
            m3::component_delete,
            m3::snapshot_create,
            m3::snapshot_download,
            m3::snapshot_list,
            m3::snapshot_list_readable,
            m3::snapshot_cancel,
            m3::backtest_preflight,
            m3::backtest_run,
            m3::backtest_list,
            m3::backtest_get,
            m3::backtest_chart_data,
            m3::backtest_execution_data,
            m3::backtest_delete,
            m3::local_data_summary,
            m3::local_data_reset,
            m3::validation_protocol_create,
            m3::validation_protocol_list,
            m3::validation_report_run,
            m3::validation_report_list,
            m3::validation_report_export
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{WasmLoader, database_path, factor_abi, strategy_abi};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn database_migration_renames_legacy_without_overwriting_current() {
        let root = std::env::temp_dir().join(format!(
            "adaq-database-migration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let legacy = root.join("adaq.sqlite3");
        let current = root.join("adaq.db");
        fs::write(&legacy, b"legacy").unwrap();

        assert_eq!(database_path(&root).unwrap(), current);
        assert_eq!(fs::read(&current).unwrap(), b"legacy");
        assert!(!legacy.exists());

        fs::write(&legacy, b"stale legacy").unwrap();
        fs::write(&current, b"current").unwrap();
        assert_eq!(database_path(&root).unwrap(), current);
        assert_eq!(fs::read(&current).unwrap(), b"current");
        assert_eq!(fs::read(&legacy).unwrap(), b"stale legacy");
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
            .join("target/wasm32-unknown-unknown/debug")
            .join(format!("m1_{name}_fixture.wasm"));
        assert!(
            path.is_file(),
            "build the {name} fixture with cargo component build"
        );
        path.to_string_lossy().into_owned()
    }

    fn bar(open_time_ms: i64, close: &str) -> factor_abi::exports::adaq::factor::api::ClosedBar {
        factor_abi::exports::adaq::factor::api::ClosedBar {
            open_time_ms,
            open: close.to_owned(),
            high: close.to_owned(),
            low: close.to_owned(),
            close: close.to_owned(),
            base_volume: "1".to_owned(),
            quote_volume: close.to_owned(),
        }
    }

    #[test]
    fn factor_loader_starts_empty() {
        let error = WasmLoader::default().describe_factor().err().unwrap();
        assert_eq!(error, "Factor component is not loaded");
    }

    #[test]
    fn factor_fixture_is_stateful_and_chunk_boundary_independent() {
        let path = fixture("factor");
        let bars = vec![
            bar(1, "0.00000303"),
            bar(2, "0.00000304"),
            bar(3, "0.00000302"),
        ];

        let whole = WasmLoader::default();
        whole.load(&path).unwrap();
        assert_eq!(
            whole.describe_factor().unwrap().output_names,
            ["close-change"]
        );
        let one_chunk = whole.process_factor(bars.clone()).unwrap();

        let chunked = WasmLoader::default();
        chunked.load(&path).unwrap();
        let mut two_chunks = chunked.process_factor(bars[..1].to_vec()).unwrap();
        two_chunks.extend(chunked.process_factor(bars[1..].to_vec()).unwrap());

        assert_eq!(one_chunk.len(), two_chunks.len());
        for (whole, chunked) in one_chunk.iter().zip(two_chunks.iter()) {
            match (whole, chunked) {
                (None, None) => {}
                (Some(whole), Some(chunked)) => {
                    assert_eq!(whole.len(), chunked.len());
                    for (whole, chunked) in whole.iter().zip(chunked.iter()) {
                        assert_eq!(whole.name, chunked.name);
                        assert_eq!(whole.value.to_bits(), chunked.value.to_bits());
                    }
                }
                _ => panic!("chunk boundaries changed Factor warmup output"),
            }
        }
        assert!(one_chunk[0].is_none());
        assert_eq!(one_chunk[1].as_ref().unwrap()[0].name, "close-change");
    }

    #[test]
    fn strategy_fixture_returns_complete_target_exposure_per_frame() {
        let loader = WasmLoader::default();
        loader
            .load_strategy(
                &fixture("strategy"),
                ["quote-volume", "close"]
                    .into_iter()
                    .map(
                        |name| strategy_abi::exports::adaq::strategy::api::FeatureSlot {
                            name: name.to_owned(),
                        },
                    )
                    .collect(),
            )
            .unwrap();
        let targets = loader
            .process_strategy(vec![
                strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                    open_time_ms: 1,
                    values: vec![2.0, 1.0],
                },
                strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                    open_time_ms: 2,
                    values: vec![1.0, 2.0],
                },
            ])
            .unwrap();
        assert_eq!(targets, ["0", "1"]);
    }

    #[test]
    fn factor_loader_rejects_strategy_abi() {
        let error = WasmLoader::default()
            .load(&fixture("strategy"))
            .unwrap_err();
        assert!(error.contains("factor"), "unexpected error: {error}");
    }

    #[test]
    fn sdk_and_host_wit_contracts_match() {
        assert_eq!(
            include_str!("../wit/factor/adaq-factor.wit"),
            include_str!("../crates/adaq-component-sdk/wit/factor/adaq-factor.wit")
        );
        assert_eq!(
            include_str!("../wit/strategy/adaq-strategy.wit"),
            include_str!("../crates/adaq-component-sdk/wit/strategy/adaq-strategy.wit")
        );
    }
}
