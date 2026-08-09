mod backtest;
mod component_library;
mod connections;
mod dataset_generation;
mod forecast_evaluation;
mod forecast_signal_dataset;
mod local_research;
mod market_data_pipeline;
mod market_data_snapshot;
mod run_engine;
mod user;
mod validation;
mod watchlist;

use adaq_backtest_core::MarketDataSnapshot;
#[cfg(test)]
use adaq_component_sdk::host::{factor_abi, strategy_abi};
use adaq_component_tooling::{FactorSchema, WasmLoader};
use adaq_data_core::{
    BarInterval, BarSeries, BarStreamEvent, BarSubscription, DataError, HistoricalBarRange,
    InstrumentStatus, Level2StreamEvent, OkxClient, SpotInstrument, TickerSnapshot,
    TickerStreamEvent, TradeStreamEvent,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{
    Emitter, Manager, State,
    ipc::Channel,
    menu::{AboutMetadata, MenuBuilder, SubmenuBuilder},
};
use watchlist::{InstrumentRef, WatchlistDb, WatchlistState};

use local_research::LocalResearchState;
use user::validate_user;

const CHECK_FOR_UPDATES_MENU_ID: &str = "check_for_updates";
const CHECK_FOR_UPDATES_EVENT: &str = "adaq-check-for-updates";

fn database_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("adaq.db")
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
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

/// Tauri Component Library commands are thin adapters: they deserialize
/// the existing contract, delegate to the Tauri-independent Component
/// Library module, and serialize the result. Command names and camelCase
/// shapes are frozen.
#[tauri::command]
fn component_import(
    request: component_library::ComponentImportRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<component_library::LibraryComponent, String> {
    state.components.import(&request.user_id, &request.bytes)
}

#[tauri::command]
async fn component_list(
    request: component_library::ComponentUserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<component_library::LibraryComponent>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .components
            .list(&request.user_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn component_page(
    request: component_library::ComponentPageRequest,
    app: tauri::AppHandle,
) -> Result<component_library::ComponentPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .components
            .page(&request.user_id, request.page)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn component_is_imported(
    request: component_library::ComponentArchiveRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<bool, String> {
    state
        .components
        .is_imported(&request.user_id, &request.archive_sha256)
}

#[tauri::command]
fn backtest_compatible_factors(
    request: component_library::BacktestDependencyRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, String> {
    state
        .components
        .compatible_factors(&request.user_id, &request.strategy_archive_sha256)
}

#[tauri::command]
fn component_delete(
    request: component_library::ComponentDeleteRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state
        .components
        .delete(&request.user_id, &request.archive_sha256)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSourceRequest {
    src: String,
}

/// Tauri Dataset Generation commands are thin adapters: they deserialize the
/// existing contract, delegate to the Tauri-independent Dataset Generation
/// lifecycle module, and serialize the result.
#[tauri::command]
fn dataset_generation_start(
    request: dataset_generation::DatasetGenerationRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<dataset_generation::Attempt, String> {
    state.generation.start(request)
}

#[tauri::command]
fn dataset_generation_retry(
    attempt_id: String,
    user_id: String,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<dataset_generation::Attempt, String> {
    state.generation.retry(&attempt_id, &user_id)
}

#[tauri::command]
async fn dataset_generation_list(
    user_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<dataset_generation::Attempt>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.generation.list(&user_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn dataset_generation_cancel(
    attempt_id: String,
    user_id: String,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state.generation.cancel(&attempt_id, &user_id)
}

/// Tauri Validation commands are thin adapters: they deserialize the
/// existing contract, delegate to the Tauri-independent Validation Studies
/// module, and serialize the result. Command names and camelCase shapes are
/// frozen.
#[tauri::command]
fn validation_protocol_create(
    request: validation::ValidationProtocolCreateRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<validation::ValidationProtocol, String> {
    state.validation.create_protocol(request)
}

#[tauri::command]
async fn validation_protocol_list(
    request: component_library::ComponentUserRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<validation::ValidationProtocol>, String> {
    state.validation.list_protocols(&request.user_id)
}

#[tauri::command]
fn validation_report_run(
    request: validation::ValidationProtocolIdRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<validation::ValidationReport, String> {
    state
        .validation
        .run_report(&request.user_id, &request.protocol_id)
}

#[tauri::command]
async fn validation_report_list(
    request: component_library::ComponentUserRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<validation::ValidationReport>, String> {
    state.validation.list_reports(&request.user_id)
}

#[tauri::command]
fn validation_report_export(
    request: validation::ValidationProtocolIdRequest,
    format: String,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<String, String> {
    state
        .validation
        .export_report(&request.user_id, &request.protocol_id, &format)
}

/// Tauri Market Data Snapshot commands are thin adapters: they deserialize
/// the existing contract, delegate to the Tauri-independent Market Data
/// Snapshot module, and serialize the result. Command names and camelCase
/// shapes are frozen.
#[tauri::command]
async fn snapshot_create(
    request: market_data_snapshot::SnapshotCreateRequest,
    client: State<'_, OkxClient>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<MarketDataSnapshot, String> {
    state.snapshots.create_for_user(&request, &client).await
}

#[tauri::command]
async fn snapshot_download(
    request: market_data_snapshot::SnapshotDownloadRequest,
    on_event: Channel<market_data_snapshot::SnapshotDownloadEvent>,
    client: State<'_, OkxClient>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<MarketDataSnapshot, String> {
    state
        .snapshots
        .download_for_user(&request, &client, |event| {
            let _ = on_event.send(event);
        })
        .await
}

#[tauri::command]
async fn snapshot_list(
    request: market_data_snapshot::SnapshotListRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<market_data_snapshot::SnapshotPage, String> {
    state.snapshots.list(&request)
}

#[tauri::command]
async fn snapshot_list_readable(
    request: market_data_snapshot::ReadableSnapshotListRequest,
    app: tauri::AppHandle,
) -> Result<Vec<MarketDataSnapshot>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .snapshots
            .list_readable(&request.user_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn snapshot_publish_universe(
    request: market_data_snapshot::UniverseSnapshotRequest,
    app: tauri::AppHandle,
) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .snapshots
            .persist_universe_for_user(&request.user_id, request.snapshot)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn snapshot_list_universe(
    request: market_data_snapshot::UniverseSnapshotListRequest,
    app: tauri::AppHandle,
) -> Result<market_data_snapshot::UniverseSnapshotPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .snapshots
            .list_universe_snapshots(&request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn snapshot_read_universe(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .snapshots
            .universe_snapshot_for_user(&request.user_id, &request.evidence_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn snapshot_cancel(
    request: market_data_snapshot::TaskRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state.snapshots.cancel_download(&request.task_id)
}

/// Tauri Data Pipeline commands are thin adapters: provider-neutral typed
/// records enter here, while raw provider payloads stay in the pipeline's
/// immutable Source evidence and never cross into GUI state.
#[tauri::command]
async fn market_data_pipeline_publish(
    request: market_data_pipeline::PublishRequest,
    on_event: Channel<adaq_data_pipeline::PipelineProgress>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::PublicationView, String> {
    let (task_id, user_id, acquisition, canonicalization) = request.into_parts()?;
    app.state::<Arc<LocalResearchState>>()
        .pipeline
        .begin_attempt(&task_id, &user_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state
            .pipeline
            .publish_attempt(&task_id, &user_id, acquisition, canonicalization, |event| {
                let _ = on_event.send(event);
            })
            .map(market_data_pipeline::PublicationView::from)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn market_data_pipeline_cancel(
    task_id: String,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state.pipeline.cancel(&task_id).map_err(string)
}

#[tauri::command]
async fn market_data_pipeline_list(
    user_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::PipelineDatasetSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .list(&user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_derive(
    request: market_data_pipeline::DeriveRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::DerivedMarketDataset, String> {
    let (user_id, canonical_id, derivation, allow_degraded) = request.into_parts();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .derive_for_user(&user_id, &canonical_id, &derivation, allow_degraded)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_derived_list(
    user_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::DerivedMarketDataset>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .list_derived_for_user(&user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_derived(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::DerivedMarketDataset, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .derived_for_user(&request.user_id, &request.evidence_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_quality(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::QualityView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .quality_for_user(&request.user_id, &request.evidence_id)
            .map(market_data_pipeline::QualityView::from)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_failures(
    user_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::PipelineFailure>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .pipeline
            .failures_for_user(&user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_publish_snapshot(
    request: market_data_pipeline::SnapshotRequest,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::SnapshotPublicationView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .publish_pipeline_snapshot_for_user_with_policy(
                &request.user_id,
                &request.canonical_id,
                request.allow_degraded,
            )
            .map(
                |(snapshot, quality)| market_data_pipeline::SnapshotPublicationView {
                    snapshot,
                    quality: market_data_pipeline::QualityView::from(quality),
                },
            )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_publish_derived_snapshot(
    request: market_data_pipeline::DerivedSnapshotRequest,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::SnapshotPublicationView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .publish_pipeline_derived_snapshot_for_user_with_policy(
                &request.user_id,
                &request.derived_id,
                request.allow_degraded,
            )
            .map(
                |(snapshot, quality)| market_data_pipeline::SnapshotPublicationView {
                    snapshot,
                    quality: market_data_pipeline::QualityView::from(quality),
                },
            )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_delete(
    request: market_data_pipeline::DeleteRequest,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        match request.evidence_kind.as_str() {
            "source" => state
                .pipeline
                .delete_source_for_user(&request.user_id, &request.evidence_id),
            "canonical" => state
                .pipeline
                .delete_canonical_for_user(&request.user_id, &request.evidence_id),
            "derived" => state
                .pipeline
                .delete_derived_for_user(&request.user_id, &request.evidence_id),
            _ => Err(adaq_data_pipeline::PipelineError::InvalidRequest(
                "only Source, Canonical, and Derived evidence can be deleted through this command"
                    .into(),
            )),
        }
        .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_instrument_master_acquire(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::okx::InstrumentMasterSnapshot, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        tauri::async_runtime::block_on(state.okx.acquire_instrument_master(&request.user_id))
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_instrument_master_list(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::InstrumentMasterSnapshot>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .okx
            .list_instrument_master_snapshots(&request.user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_universe(
    request: market_data_pipeline::UniverseRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::okx::PointInTimeInstrumentUniverse, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .okx
            .point_in_time_universe(&request.user_id, request.as_of_ms)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_backfill(
    request: adaq_data_pipeline::okx::OkxBackfillRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::PublicationView>, String> {
    validate_user(&request.user_id)?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &request.user_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result =
            tauri::async_runtime::block_on(state.okx.backfill(&request, cancellation, |event| {
                let _ = on_event.send(event);
            }));
        let finish = state.okx.finish_backfill(&task_id);
        match (result, finish) {
            (Ok(publications), Ok(())) => Ok(publications
                .into_iter()
                .map(market_data_pipeline::PublicationView::from)
                .collect()),
            (Err(error), _) | (_, Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn okx_backfill_cancel(
    request: market_data_pipeline::BackfillCancelRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    state
        .okx
        .cancel_backfill(&request.task_id, &request.user_id)
        .map_err(string)
}

#[tauri::command]
async fn okx_acquisition_status(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::OkxAcquisitionStatus>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .okx
            .acquisition_statuses(&request.user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_stream_health(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::OkxStreamHealth>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .okx
            .stream_health(&request.user_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_instrument_master_acquire(
    request: market_data_pipeline::AshareInstrumentMasterRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareInstrumentMasterSnapshotDto, String> {
    validate_user(&request.user_id)?;
    let operation_id = request.operation_id();
    let user_id = request.user_id.clone();
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .ashare
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let connector_cancellation = cancellation.clone();
        let result = tauri::async_runtime::block_on(
            state
                .ashare
                .acquire_instrument_master_with_cancel(&user_id, move || {
                    connector_cancellation.is_cancelled()
                }),
        )
        .map_err(string);
        let finish = state.ashare.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(snapshot), Ok(())) => Ok(snapshot.gui_dto()),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_instrument_master_list(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::a_share::AshareInstrumentMasterSnapshotDto>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .ashare
            .list_instrument_master_snapshots(&request.user_id)
            .map(|snapshots| {
                snapshots
                    .iter()
                    .map(|snapshot| snapshot.gui_dto())
                    .collect()
            })
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_universe(
    request: market_data_pipeline::UniverseRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AsharePointInTimeUniverse, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .ashare
            .point_in_time_membership(&request.user_id, request.as_of_ms)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_calendar_acquire(
    request: market_data_pipeline::AshareCalendarRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::a_share::AshareCalendarSnapshotDto>, String> {
    validate_user(&request.user_id)?;
    let range = request.range();
    let operation_id = request.operation_id();
    let user_id = request.user_id.clone();
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .ashare
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let connector_cancellation = cancellation.clone();
        let result = tauri::async_runtime::block_on(state.ashare.acquire_calendar_with_cancel(
            &user_id,
            range,
            move || connector_cancellation.is_cancelled(),
        ))
        .map_err(string);
        let finish = state.ashare.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(snapshots), Ok(())) => Ok(snapshots
                .iter()
                .map(|snapshot| snapshot.gui_dto())
                .collect()),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_corporate_actions_acquire(
    request: market_data_pipeline::AshareCorporateActionRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareCorporateActionDatasetDto, String> {
    validate_user(&request.user_id)?;
    let operation_id = request.operation_id();
    let user_id = request.user_id.clone();
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .ashare
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let connector_cancellation = cancellation.clone();
        let result =
            tauri::async_runtime::block_on(state.ashare.acquire_corporate_actions_with_cancel(
                &user_id,
                request.instrument,
                move || connector_cancellation.is_cancelled(),
            ))
            .map_err(string);
        let finish = state.ashare.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(dataset), Ok(())) => Ok(dataset.gui_dto()),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn ashare_backfill(
    request: adaq_data_pipeline::a_share::AshareBackfillRequest,
    on_event: Channel<adaq_data_pipeline::a_share::AshareBackfillEvent>,
    app: tauri::AppHandle,
) -> Result<Option<market_data_pipeline::PublicationView>, String> {
    validate_user(&request.user_id)?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .ashare
        .begin_backfill(&request.user_id, &request.task_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(state.ashare.backfill(
            &request,
            cancellation,
            |event| {
                let _ = on_event.send(event);
            },
        ));
        match result {
            Ok(publication) => Ok(publication.map(market_data_pipeline::PublicationView::from)),
            Err(error) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn ashare_backfill_cancel(
    request: market_data_pipeline::BackfillCancelRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    state
        .ashare
        .cancel_backfill(&request.user_id, &request.task_id)
        .map_err(string)
}

#[tauri::command]
fn ashare_acquisition_cancel(
    request: market_data_pipeline::AshareAcquisitionCancelRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    state
        .ashare
        .cancel_acquisition(&request.user_id, &request.operation_id)
        .map_err(string)
}

#[tauri::command]
async fn ashare_workspace(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareMarketWorkspaceDto, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .ashare
            .workspace_dto_for_user(&request.user_id, &request.evidence_id, unix_now_ms())
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_instrument_master_acquire(
    request: market_data_pipeline::UsEquityInstrumentMasterRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityInstrumentMasterSnapshotDto, String> {
    validate_user(&request.user_id)?;
    let operation_id = request.operation_id();
    let user_id = request.user_id;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .us_equity
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let operation_user_id = user_id.clone();
        let cancellation_for_operation = cancellation.clone();
        let operation_state = state.clone();
        let result = state
            .connections
            .with_alpaca_client(&user_id, move |client| {
                tauri::async_runtime::block_on(operation_state.us_equity.acquire_instrument_master(
                    &operation_user_id,
                    &client,
                    &cancellation_for_operation,
                    unix_now_ms(),
                ))
            })
            .and_then(|result| result.map_err(string));
        let finish = state.us_equity.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(snapshot), Ok(())) => Ok(snapshot.gui_dto()),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_instrument_master_list(
    request: market_data_pipeline::UserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::us_equity::UsEquityInstrumentMasterSnapshotDto>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .us_equity
            .list_instrument_master_snapshots(&request.user_id)
            .map(|snapshots| {
                snapshots
                    .iter()
                    .map(|snapshot| snapshot.gui_dto())
                    .collect()
            })
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_universe(
    request: market_data_pipeline::UniverseRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityPointInTimeUniverse, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .us_equity
            .point_in_time_membership(&request.user_id, request.as_of_ms)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_calendar_acquire(
    request: market_data_pipeline::UsEquityCalendarRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityCalendarSnapshotDto, String> {
    validate_user(&request.user_id)?;
    let range = request.range();
    let operation_id = request.operation_id();
    let venue = request.venue;
    let user_id = request.user_id;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .us_equity
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let operation_user_id = user_id.clone();
        let cancellation_for_operation = cancellation.clone();
        let operation_state = state.clone();
        let result = state
            .connections
            .with_alpaca_client(&user_id, move |client| {
                tauri::async_runtime::block_on(operation_state.us_equity.acquire_calendar(
                    &operation_user_id,
                    &client,
                    venue,
                    range,
                    &cancellation_for_operation,
                    unix_now_ms(),
                ))
            })
            .and_then(|result| result.map_err(string));
        let finish = state.us_equity.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(snapshot), Ok(())) => Ok(snapshot.gui_dto()),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_backfill(
    request: adaq_data_pipeline::us_equity::UsEquityBackfillRequest,
    on_event: Channel<adaq_data_pipeline::us_equity::UsEquityBackfillEvent>,
    app: tauri::AppHandle,
) -> Result<Option<market_data_pipeline::PublicationView>, String> {
    validate_user(&request.user_id)?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .us_equity
        .begin_backfill(&user_id, &task_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let request_user_id = request.user_id.clone();
        let cancellation_for_operation = cancellation.clone();
        let operation_state = state.clone();
        let result = state
            .connections
            .with_alpaca_client(&user_id, move |client| {
                tauri::async_runtime::block_on(operation_state.us_equity.backfill(
                    request,
                    &client,
                    cancellation_for_operation,
                    |event| {
                        let _ = on_event.send(event);
                    },
                ))
            })
            .and_then(|result| result.map_err(string));
        let finish = state.us_equity.finish_backfill(&request_user_id, &task_id);
        match (result, finish) {
            (Ok(publication), Ok(())) => {
                Ok(publication.map(market_data_pipeline::PublicationView::from))
            }
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn alpaca_backfill_cancel(
    request: market_data_pipeline::BackfillCancelRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    state
        .us_equity
        .cancel_backfill(&request.user_id, &request.task_id)
        .map_err(string)
}

#[tauri::command]
fn alpaca_acquisition_cancel(
    request: market_data_pipeline::AshareAcquisitionCancelRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    state
        .us_equity
        .cancel_acquisition(&request.user_id, &request.operation_id)
        .map_err(string)
}

#[tauri::command]
async fn alpaca_acquisition_status(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<Option<adaq_data_pipeline::us_equity::UsEquityAcquisitionStatus>, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .us_equity
            .acquisition_status(&request.user_id, &request.evidence_id)
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_snapshot(
    request: market_data_pipeline::UsEquitySnapshotRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityMarketSnapshotDto, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>().inner().clone();
        state
            .connections
            .with_alpaca_client(&request.user_id, |client| {
                tauri::async_runtime::block_on(state.us_equity.snapshot(
                    &client,
                    request.instrument,
                    unix_now_ms(),
                ))
            })
            .and_then(|result| result.map_err(string))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_stream(
    request: market_data_pipeline::UsEquityStreamRequest,
    on_event: Channel<adaq_data_core::alpaca::AlpacaStreamEvent>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    let operation_id = request.operation_id();
    let user_id = request.user_id;
    let subscription = request.subscription;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .us_equity
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let cancellation_for_stream = cancellation.clone();
        let result = state
            .connections
            .with_alpaca_client(&user_id, |client| {
                tauri::async_runtime::block_on(client.stream(subscription, |event| {
                    if cancellation_for_stream.is_cancelled() {
                        return false;
                    }
                    on_event.send(event).is_ok()
                }))
            })
            .map_err(string)
            .and_then(|result| result.map_err(string));
        let finish = state.us_equity.finish_acquisition(&user_id, &operation_id);
        match (result, finish) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn alpaca_workspace(
    request: market_data_pipeline::UserEvidenceRequest,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityMarketWorkspaceDto, String> {
    validate_user(&request.user_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .us_equity
            .workspace_dto_for_user(&request.user_id, &request.evidence_id, unix_now_ms())
            .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Tauri Backtest Run commands are thin adapters: they deserialize the
/// existing contract, delegate to the Tauri-independent Backtest Run
/// module, and serialize the result. Command names and camelCase shapes
/// are frozen.
#[tauri::command]
fn backtest_preflight(
    request: backtest::BacktestRunRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestPreflight, String> {
    state.backtests.preflight(&request)
}

#[tauri::command]
fn backtest_run(
    request: backtest::BacktestRunRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    state.backtests.run(request)
}

#[tauri::command]
async fn backtest_list(
    request: backtest::BacktestListRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunPage, String> {
    state.backtests.list(&request)
}

#[tauri::command]
fn backtest_get(
    request: backtest::BacktestRunIdRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    state.backtests.get(&request.user_id, &request.run_id)
}

#[tauri::command]
fn backtest_chart_data(
    request: backtest::BacktestChartRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    state.backtests.chart_data(&request)
}

#[tauri::command]
fn backtest_execution_data(
    request: backtest::BacktestExecutionRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestExecutionPage, String> {
    state.backtests.execution_data(&request)
}

#[tauri::command]
fn backtest_delete(
    request: backtest::BacktestRunIdRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state.backtests.delete(&request.user_id, &request.run_id)
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
struct MarketSubscribeRealtimeRequest {
    src: String,
    user_id: String,
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

struct ActiveTradeStream {
    subscription_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    on_event: Channel<TradeStreamEvent>,
}

#[derive(Default)]
struct TradeStreamState(Mutex<Option<ActiveTradeStream>>);

struct ActiveLevel2Stream {
    subscription_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    on_event: Channel<Level2StreamEvent>,
}

#[derive(Default)]
struct Level2StreamState(Mutex<Option<ActiveLevel2Stream>>);

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
fn market_subscribe_trades(
    request: MarketSubscribeRealtimeRequest,
    on_event: Channel<TradeStreamEvent>,
    app: tauri::AppHandle,
    streams: State<'_, TradeStreamState>,
) -> Result<(), DataError> {
    validate_realtime_request(
        &request.src,
        &request.user_id,
        &request.codes,
        &request.subscription_id,
    )?;
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    let task_path = app.state::<Arc<LocalResearchState>>().okx.clone();
    let task_channel = on_event.clone();
    let user_id = request.user_id;
    let codes = request.codes;
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = task_path
            .stream_trades(&user_id, &codes, |event| task_channel.send(event).is_ok())
            .await
        {
            let _ = task_channel.send(TradeStreamEvent::Error(error));
        }
    });
    if let Some(previous) = active.replace(ActiveTradeStream {
        subscription_id: request.subscription_id,
        task,
        on_event,
    }) {
        let _ = previous.on_event.send(TradeStreamEvent::Closed);
        previous.task.abort();
    }
    Ok(())
}

#[tauri::command]
fn market_unsubscribe_trades(
    request: MarketUnsubscribeTickerRequest,
    streams: State<'_, TradeStreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    if active
        .as_ref()
        .is_some_and(|stream| stream.subscription_id == request.subscription_id)
    {
        if let Some(previous) = active.take() {
            let _ = previous.on_event.send(TradeStreamEvent::Closed);
            previous.task.abort();
        }
    }
    Ok(())
}

#[tauri::command]
fn market_subscribe_level2(
    request: MarketSubscribeRealtimeRequest,
    on_event: Channel<Level2StreamEvent>,
    app: tauri::AppHandle,
    streams: State<'_, Level2StreamState>,
) -> Result<(), DataError> {
    validate_realtime_request(
        &request.src,
        &request.user_id,
        &request.codes,
        &request.subscription_id,
    )?;
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    let task_path = app.state::<Arc<LocalResearchState>>().okx.clone();
    let task_channel = on_event.clone();
    let user_id = request.user_id;
    let codes = request.codes;
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = task_path
            .stream_level2(&user_id, &codes, |event| task_channel.send(event).is_ok())
            .await
        {
            let _ = task_channel.send(Level2StreamEvent::Error(error));
        }
    });
    if let Some(previous) = active.replace(ActiveLevel2Stream {
        subscription_id: request.subscription_id,
        task,
        on_event,
    }) {
        let _ = previous.on_event.send(Level2StreamEvent::Closed);
        previous.task.abort();
    }
    Ok(())
}

#[tauri::command]
fn market_unsubscribe_level2(
    request: MarketUnsubscribeTickerRequest,
    streams: State<'_, Level2StreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    if active
        .as_ref()
        .is_some_and(|stream| stream.subscription_id == request.subscription_id)
    {
        if let Some(previous) = active.take() {
            let _ = previous.on_event.send(Level2StreamEvent::Closed);
            previous.task.abort();
        }
    }
    Ok(())
}

fn validate_realtime_request(
    src: &str,
    user_id: &str,
    codes: &[String],
    subscription_id: &str,
) -> Result<(), DataError> {
    require_okx(src)?;
    validate_user(user_id).map_err(|message| DataError::new("okx", "invalid_request", message))?;
    if subscription_id.trim().is_empty() || !(1..=32).contains(&codes.len()) {
        return Err(DataError::new(
            "okx",
            "invalid_request",
            "user ID and subscription ID must be non-empty and codes must contain 1 to 32 items",
        ));
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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionUserRequest {
    user_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProfileRequest {
    user_id: String,
    profile_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionSaveRequest {
    user_id: String,
    credentials: connections::ProviderCredentials,
}

/// Tauri Connection commands are thin adapters: they deserialize the
/// existing contract, delegate to the Tauri-independent Connection domain,
/// and serialize the result. Errors are serialized as the typed, redacted
/// ConnectionError contract so the GUI can localize them.
#[tauri::command]
fn connection_profile_list(
    request: ConnectionUserRequest,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<connections::ProfileView>, String> {
    state.connections.list(&request.user_id)
}

#[tauri::command]
async fn connection_profile_save(
    request: ConnectionSaveRequest,
    app: tauri::AppHandle,
) -> Result<connections::ProfileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state
            .connections
            .save(&request.user_id, request.credentials, connections::now_ms())
            .map_err(serialize_connection_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn connection_profile_test(
    request: ConnectionProfileRequest,
    app: tauri::AppHandle,
) -> Result<connections::ProfileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state
            .connections
            .test(&request.user_id, &request.profile_id, connections::now_ms())
            .map_err(serialize_connection_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn connection_profile_delete(
    request: ConnectionProfileRequest,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state
            .connections
            .delete(&request.user_id, &request.profile_id)
            .map_err(serialize_connection_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn serialize_connection_error(error: connections::ConnectionError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| error.message)
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
            app.manage(TradeStreamState::default());
            app.manage(Level2StreamState::default());
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database_path = database_path(&app_data_dir);
            app.manage(LocalResearchState::open(&app_data_dir).map_err(std::io::Error::other)?);
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
            market_subscribe_trades,
            market_unsubscribe_trades,
            market_subscribe_level2,
            market_unsubscribe_level2,
            market_unsubscribe_ticker,
            market_subscribe_bars,
            market_unsubscribe_bar,
            component_import,
            component_list,
            component_page,
            component_is_imported,
            backtest_compatible_factors,
            local_research::backtest_compatible_signals,
            component_delete,
            snapshot_create,
            snapshot_download,
            snapshot_list,
            snapshot_list_readable,
            snapshot_publish_universe,
            snapshot_list_universe,
            snapshot_read_universe,
            snapshot_cancel,
            market_data_pipeline_publish,
            market_data_pipeline_cancel,
            market_data_pipeline_list,
            market_data_pipeline_derive,
            market_data_pipeline_derived_list,
            market_data_pipeline_derived,
            market_data_pipeline_quality,
            market_data_pipeline_failures,
            market_data_pipeline_publish_snapshot,
            market_data_pipeline_publish_derived_snapshot,
            market_data_pipeline_delete,
            okx_instrument_master_acquire,
            okx_instrument_master_list,
            okx_universe,
            okx_backfill,
            okx_backfill_cancel,
            okx_acquisition_status,
            okx_stream_health,
            ashare_instrument_master_acquire,
            ashare_instrument_master_list,
            ashare_universe,
            ashare_calendar_acquire,
            ashare_corporate_actions_acquire,
            ashare_backfill,
            ashare_backfill_cancel,
            ashare_acquisition_cancel,
            ashare_workspace,
            alpaca_instrument_master_acquire,
            alpaca_instrument_master_list,
            alpaca_universe,
            alpaca_calendar_acquire,
            alpaca_backfill,
            alpaca_backfill_cancel,
            alpaca_acquisition_cancel,
            alpaca_acquisition_status,
            alpaca_snapshot,
            alpaca_stream,
            alpaca_workspace,
            backtest_preflight,
            backtest_run,
            backtest_list,
            backtest_get,
            backtest_chart_data,
            backtest_execution_data,
            backtest_delete,
            local_research::local_data_summary,
            local_research::local_data_reset,
            validation_protocol_create,
            validation_protocol_list,
            validation_report_run,
            validation_report_list,
            validation_report_export,
            dataset_generation_start,
            dataset_generation_retry,
            dataset_generation_list,
            dataset_generation_cancel,
            forecast_signal_dataset::signal_dataset_list,
            forecast_signal_dataset::signal_dataset_get,
            forecast_signal_dataset::signal_dataset_rows,
            forecast_signal_dataset::signal_dataset_import,
            forecast_signal_dataset::signal_dataset_export,
            forecast_evaluation::forecast_evaluation_create,
            forecast_evaluation::forecast_evaluation_list,
            forecast_evaluation::forecast_evaluation_export,
            connection_profile_list,
            connection_profile_save,
            connection_profile_test,
            connection_profile_delete
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::{WasmLoader, factor_abi, strategy_abi};
    use std::path::PathBuf;

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
