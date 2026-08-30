mod auth;
mod backtest;
mod bot_supervisor;
mod component_library;
mod connections;
mod dataset_generation;
mod factor_research;
mod features;
mod forecast_evaluation;
mod forecast_signal_dataset;
mod local_research;
mod market_data_pipeline;
mod market_data_snapshot;
mod operations;
mod paper_feedback;
mod paper_trading;
mod python_research;
mod research_queue;
mod run_engine;
mod strategy_candidate;
mod strategy_qualification;
mod user;
mod validation;
mod watchlist;

use adaq_backtest_core::MarketDataSnapshot;
#[cfg(test)]
use adaq_component_sdk::host::{factor_abi, strategy_abi};
use adaq_component_tooling::{
    BuiltinForecastTarget, ComponentKind, FactorSchema, ForecastTarget, ForecastValueScale,
    ModelScope, PredictionKind, WasmLoader,
};
use adaq_data_core::{
    BarGap, BarInterval, BarSeries, BarStreamEvent, BarSubscription, DataError, HistoricalBarRange,
    InstrumentStatus, Level2StreamEvent, OhlcvBar, OkxClient, SpotInstrument, TickerSnapshot,
    TickerStreamEvent, TradeStreamEvent,
    market::{InstrumentId, PriceBasis, VenueKind},
};
use adaq_trading_crypto::{Exchange, Params};
use rust_decimal::Decimal;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent,
    ipc::Channel,
    menu::{AboutMetadata, MenuBuilder, SubmenuBuilder},
};
use watchlist::{InstrumentRef, WatchlistDb, WatchlistState, validate_provider_venue};

use local_research::LocalResearchState;
use user::validate_user;

const CHECK_FOR_UPDATES_MENU_ID: &str = "check_for_updates";
const CHECK_FOR_UPDATES_EVENT: &str = "adaq-check-for-updates";

fn database_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("adaq.db")
}

struct WorkspaceStates {
    bot_supervisor: std::sync::Arc<bot_supervisor::BotSupervisor>,
    local_research: Arc<LocalResearchState>,
    python_research: Arc<python_research::PythonResearchState>,
    strategy_candidates: Arc<strategy_candidate::StrategyCandidateStore>,
    strategy_qualification: Arc<strategy_qualification::StrategyQualificationStore>,
    watchlist: WatchlistDb,
}

enum WorkspaceInitStatus {
    NotStarted(PathBuf),
    Pending,
    Ready(Option<WorkspaceStates>),
    Failed(String),
    Managed,
}

struct WorkspaceInitialization {
    status: Mutex<WorkspaceInitStatus>,
    ready: Condvar,
}

impl WorkspaceInitialization {
    fn new(app_data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(WorkspaceInitStatus::NotStarted(app_data_dir)),
            ready: Condvar::new(),
        })
    }

    fn install(self: &Arc<Self>, app: &AppHandle) -> Result<(), String> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| "workspace initialization lock poisoned".to_owned())?;
        if let WorkspaceInitStatus::NotStarted(app_data_dir) = &*status {
            let app_data_dir = app_data_dir.clone();
            *status = WorkspaceInitStatus::Pending;
            let worker = Arc::clone(self);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    open_workspace_states(&app_data_dir)
                }))
                .map_err(|_| "workspace initialization panicked".to_owned())
                .and_then(|result| result);
                let status = match result {
                    Ok(states) => WorkspaceInitStatus::Ready(Some(states)),
                    Err(error) => WorkspaceInitStatus::Failed(error),
                };
                if let Ok(mut current) = worker.status.lock() {
                    *current = status;
                    worker.ready.notify_all();
                }
            });
        }

        loop {
            match &mut *status {
                WorkspaceInitStatus::NotStarted(_) => {
                    unreachable!("workspace initialization must start")
                }
                WorkspaceInitStatus::Pending => {
                    status = self
                        .ready
                        .wait(status)
                        .map_err(|_| "workspace initialization wait failed".to_owned())?;
                }
                WorkspaceInitStatus::Ready(states) => {
                    let states = states
                        .take()
                        .ok_or_else(|| "workspace states were already consumed".to_owned())?;
                    app.manage(states.local_research);
                    app.manage(states.bot_supervisor);
                    app.manage(states.python_research);
                    app.manage(states.strategy_candidates);
                    app.manage(states.strategy_qualification);
                    app.manage(states.watchlist);
                    *status = WorkspaceInitStatus::Managed;
                    return Ok(());
                }
                WorkspaceInitStatus::Failed(error) => return Err(error.clone()),
                WorkspaceInitStatus::Managed => return Ok(()),
            }
        }
    }
}

fn open_workspace_states(app_data_dir: &Path) -> Result<WorkspaceStates, String> {
    std::fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let database_path = database_path(app_data_dir);
    let local_research = LocalResearchState::open(app_data_dir)?;
    let python_research = Arc::new(python_research::PythonResearchState::open(app_data_dir));
    python_research.attach_queue(local_research.features.queue());
    local_research
        .features
        .attach_python(python_research.clone())?;
    let strategy_candidate_source = Arc::new(LocalStrategyCandidateSource {
        local_research: local_research.clone(),
        python_research: python_research.clone(),
    });
    let strategy_candidates = Arc::new(strategy_candidate::StrategyCandidateStore::open(
        local_research.database.clone(),
        strategy_candidate_source,
    )?);
    let strategy_qualification_source = Arc::new(LocalStrategyQualificationSource {
        local_research: local_research.clone(),
        strategy_candidates: strategy_candidates.clone(),
    });
    let strategy_qualification =
        Arc::new(strategy_qualification::StrategyQualificationStore::open(
            local_research.database.clone(),
            strategy_qualification_source,
        )?);
    let watchlist = WatchlistDb::open(&database_path)?;
    let bot_supervisor = Arc::new(bot_supervisor::BotSupervisor::new(
        local_research.operations.clone(),
    ));
    bot_supervisor.start_monitor();
    Ok(WorkspaceStates {
        bot_supervisor,
        local_research,
        python_research,
        strategy_candidates,
        strategy_qualification,
        watchlist,
    })
}

struct LocalStrategyCandidateSource {
    local_research: Arc<LocalResearchState>,
    python_research: Arc<python_research::PythonResearchState>,
}

impl strategy_candidate::StrategyCandidateSource for LocalStrategyCandidateSource {
    fn factor_inputs(
        &self,
        user_id: &str,
    ) -> Result<Vec<strategy_candidate::ResolvedFactorInput>, String> {
        self.local_research
            .factor
            .accepted_component_inputs(user_id)
            .map(|inputs| {
                inputs
                    .into_iter()
                    .map(|input| strategy_candidate::ResolvedFactorInput {
                        decision_id: input.decision_id,
                        decision_hash: input.decision_hash,
                        candidate_hash: input.candidate_hash,
                        output_name: input.output_name,
                        package_archive_sha256: input.package_archive_sha256,
                        package_wasm_sha256: input.package_wasm_sha256,
                        component_id: input.component_id,
                        component_version: input.component_version,
                        feature_plan_hash: input.feature_plan_hash,
                        context_hash: input.context_hash,
                        snapshot_id: input.snapshot_id,
                        universe_id: input.universe_id,
                        market: input.market,
                        venue: input.venue,
                    })
                    .collect()
            })
    }

    fn model_inputs(
        &self,
        user_id: &str,
    ) -> Result<Vec<strategy_candidate::ResolvedModelInput>, String> {
        self.python_research
            .accepted_model_inputs(user_id)
            .and_then(|inputs| {
                inputs
                    .into_iter()
                    .map(|input| self.resolve_model_input(user_id, input))
                    .collect()
            })
    }

    fn resolve_factor(
        &self,
        user_id: &str,
        binding: &strategy_candidate::FactorInputBinding,
    ) -> Result<strategy_candidate::ResolvedFactorInput, String> {
        let input = self.local_research.factor.accepted_component_input(
            user_id,
            &binding.decision_id,
            &binding.output_name,
        )?;
        Ok(strategy_candidate::ResolvedFactorInput {
            decision_id: input.decision_id,
            decision_hash: input.decision_hash,
            candidate_hash: input.candidate_hash,
            output_name: input.output_name,
            package_archive_sha256: input.package_archive_sha256,
            package_wasm_sha256: input.package_wasm_sha256,
            component_id: input.component_id,
            component_version: input.component_version,
            feature_plan_hash: input.feature_plan_hash,
            context_hash: input.context_hash,
            snapshot_id: input.snapshot_id,
            universe_id: input.universe_id,
            market: input.market,
            venue: input.venue,
        })
    }

    fn resolve_model(
        &self,
        user_id: &str,
        binding: &strategy_candidate::ModelInputBinding,
    ) -> Result<strategy_candidate::ResolvedModelInput, String> {
        let input = self
            .python_research
            .accepted_model_input(user_id, &binding.qualification_report_id)?;
        if input.decision_id != binding.decision_id
            || input.final_evaluation_report_id != binding.final_evaluation_report_id
            || input.artifact_sha256 != binding.artifact_sha256
            || input.transformation_sha256 != binding.transformation_sha256
            || input.package_archive_sha256 != binding.package_archive_sha256
            || input.package_wasm_sha256 != binding.package_wasm_sha256
            || input.component_id != binding.component_id
            || input.component_version != binding.component_version
            || input.model_profile != binding.model_profile
            || input.exporter_id != binding.exporter_id
            || input.sdk_version != binding.sdk_version
            || input.abi_version != binding.abi_version
            || input.runtime_identity != binding.runtime_identity
            || input.input_slots != binding.input_slots
            || input.output_name != binding.output_name
            || input.target_id != binding.target_id
            || input.target_horizon_bars != binding.target_horizon_bars
            || input.forecast_contract != binding.forecast_contract
        {
            return Err("Model qualification identity does not match the request".into());
        }
        self.resolve_model_input(user_id, input)
    }
}

impl LocalStrategyCandidateSource {
    fn resolve_model_input(
        &self,
        user_id: &str,
        input: python_research::AcceptedModelInput,
    ) -> Result<strategy_candidate::ResolvedModelInput, String> {
        let package = self
            .local_research
            .package_for_user(user_id, &input.package_archive_sha256)?;
        if package.archive_sha256 != input.package_archive_sha256
            || package.manifest.kind != ComponentKind::Model
            || package.manifest.wasm_sha256 != input.package_wasm_sha256
            || package.manifest.component_id.to_string() != input.component_id
            || package.manifest.version.to_string() != input.component_version
            || package.manifest.model_scope != Some(ModelScope::SingleInstrument)
            || package.manifest.model_outputs.len() != 1
            || package
                .manifest
                .feature_slots
                .iter()
                .map(|slot| slot.name.as_str())
                .ne(input.input_slots.iter().map(String::as_str))
            || package.manifest.model_outputs[0].name != input.output_name
            || package.manifest.model_outputs[0].horizon_bars != input.target_horizon_bars
            || !matches!(
                &package.manifest.model_outputs[0].prediction_kind,
                PredictionKind::ExpectedValue
            )
            || !matches!(
                &package.manifest.model_outputs[0].forecast_target,
                ForecastTarget::Builtin {
                    target: BuiltinForecastTarget::FutureCloseReturn
                }
            )
            || !matches!(
                &package.manifest.model_outputs[0].value_scale,
                ForecastValueScale::Native
            )
        {
            return Err("Model Component Package does not match qualified evidence".into());
        }
        Ok(strategy_candidate::ResolvedModelInput {
            qualification_report_id: input.qualification_report_id,
            decision_id: input.decision_id,
            final_evaluation_report_id: input.final_evaluation_report_id,
            artifact_sha256: input.artifact_sha256,
            transformation_sha256: input.transformation_sha256,
            package_archive_sha256: input.package_archive_sha256,
            package_wasm_sha256: input.package_wasm_sha256,
            component_id: input.component_id,
            component_version: input.component_version,
            model_profile: input.model_profile,
            exporter_id: input.exporter_id,
            sdk_version: input.sdk_version,
            abi_version: input.abi_version,
            runtime_identity: input.runtime_identity,
            input_slots: input.input_slots,
            output_name: input.output_name,
            target_id: input.target_id,
            target_horizon_bars: input.target_horizon_bars,
            forecast_contract: input.forecast_contract,
            input_evidence_sha256: input.input_evidence_sha256,
        })
    }
}

struct LocalStrategyQualificationSource {
    local_research: Arc<LocalResearchState>,
    strategy_candidates: Arc<strategy_candidate::StrategyCandidateStore>,
}

impl strategy_qualification::StrategyQualificationSource for LocalStrategyQualificationSource {
    fn candidate_revision(
        &self,
        user_id: &str,
        candidate_id: &str,
        revision: u64,
    ) -> Result<(strategy_candidate::StrategyCandidateRevision, bool), String> {
        self.strategy_candidates
            .revision_for_user(user_id, candidate_id, revision)
    }

    fn import_strategy_package(
        &self,
        user_id: &str,
        bytes: &[u8],
    ) -> Result<adaq_component_tooling::ComponentPackage, String> {
        let archive_sha256 = adaq_component_tooling::ComponentPackage::read(bytes)
            .map_err(|error| error.to_string())?
            .archive_sha256;
        self.local_research.components.import(user_id, bytes)?;
        self.local_research
            .package_for_user(user_id, &archive_sha256)
    }

    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<adaq_component_tooling::ComponentPackage, String> {
        self.local_research
            .package_for_user(user_id, archive_sha256)
    }

    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.local_research.snapshot_for_user(user_id, snapshot_id)
    }

    fn universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
        self.local_research
            .snapshots
            .universe_snapshot_for_user(user_id, snapshot_id)
    }

    fn run_backtest(
        &self,
        request: backtest::BacktestRunRequest,
    ) -> Result<backtest::BacktestRunView, String> {
        self.local_research.backtests.run(request)
    }

    fn run_portfolio_backtest(
        &self,
        request: backtest::BacktestRunRequest,
    ) -> Result<backtest::PortfolioBacktestView, String> {
        self.local_research
            .backtests
            .portfolio_run_from_request(request)
    }

    fn load_portfolio_backtest(
        &self,
        user_id: &str,
        run_id: &str,
    ) -> Result<backtest::PortfolioBacktestView, String> {
        self.local_research
            .backtests
            .load_portfolio_run(user_id, run_id)
    }

    fn load_backtest(&self, user_id: &str, run_id: &str) -> Result<backtest::BacktestRun, String> {
        self.local_research.backtests.load_run(user_id, run_id)
    }

    fn create_protocol(
        &self,
        request: validation::ValidationProtocolCreateRequest,
    ) -> Result<validation::ValidationProtocol, String> {
        self.local_research.validation.create_protocol(request)
    }

    fn protocol_for_user(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<validation::ValidationProtocol, String> {
        self.local_research
            .validation
            .protocol_for_user(user_id, protocol_id)
    }

    fn run_report(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<validation::ValidationReport, String> {
        self.local_research
            .validation
            .run_report(user_id, protocol_id)
    }

    fn report_for_user(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<validation::ValidationReport, String> {
        self.local_research
            .validation
            .report_for_user(user_id, report_id)
    }
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
async fn auth_bind_session(
    access_token: String,
    window: WebviewWindow,
    state: State<'_, auth::AuthState>,
) -> Result<auth::AuthContextView, String> {
    let auth_state = state.inner().clone();
    let window_label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        auth_state.bind(&window_label, &access_token, auth::now_ms())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn workspace_ready(
    app: AppHandle,
    state: State<'_, Arc<WorkspaceInitialization>>,
) -> Result<(), String> {
    let initialization = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || initialization.install(&app))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
fn auth_clear_session(window: WebviewWindow, state: State<'_, auth::AuthState>) {
    state.clear(window.label());
}

#[tauri::command]
fn operations_observe(
    mut observation: operations::HealthObservation,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<
    (
        operations::OperationalEvent,
        Option<operations::AlertView>,
        operations::SafetyAction,
    ),
    String,
> {
    observation.user_id = auth.user_id_for_window(window.label())?;
    state.operations.observe(observation)
}

#[tauri::command]
fn operations_health(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<operations::HealthView>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    state.operations.health_for_user(&user_id)
}

#[tauri::command]
fn operations_alerts(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<operations::AlertView>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    state.operations.alerts_for_user(&user_id)
}

#[tauri::command]
fn paper_feedback_snapshot_create(
    mut input: paper_feedback::FeedbackSnapshotInput,
    created_at_ms: i64,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<paper_feedback::FeedbackSnapshot, String> {
    input.user_id = auth.user_id_for_window(window.label())?;
    state.paper_feedback.create_snapshot(input, created_at_ms)
}

#[tauri::command]
fn paper_feedback_report_create(
    mut input: paper_feedback::FeedbackReportInput,
    created_at_ms: i64,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<paper_feedback::FeedbackReport, String> {
    input.user_id = auth.user_id_for_window(window.label())?;
    state.paper_feedback.create_report(input, created_at_ms)
}

#[tauri::command]
fn paper_feedback_review_decide(
    mut input: paper_feedback::ReviewDecisionInput,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<paper_feedback::ReviewDecision, String> {
    input.user_id = auth.user_id_for_window(window.label())?;
    state.paper_feedback.record_review_decision(input)
}

#[tauri::command]
fn operations_alert_transition(
    user_id: String,
    alert_id: String,
    state: operations::AlertState,
    event_id: String,
    occurred_at_ms: i64,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    store: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    store
        .operations
        .transition_alert(&user_id, &alert_id, state, &event_id, occurred_at_ms)
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

#[tauri::command]
fn factor_metric_catalog() -> adaq_factor_research::FactorMetricCatalog {
    adaq_factor_research::FactorMetricCatalog::initial()
}

#[tauri::command]
fn research_context_establish(
    draft: adaq_factor_research::ResearchEvidenceContextDraft,
    stage: adaq_factor_research::ResearchStage,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_factor_research::ResearchEvidenceProjection, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    if stage == adaq_factor_research::ResearchStage::Factors || draft.feature_dataset.is_some() {
        return Err("factor-context-requires-host-dataset-selection".into());
    }
    let mut draft = draft;
    draft.user_id = user_id.clone();
    for evidence in &mut draft.evidence {
        evidence.user_id = user_id.clone();
    }
    let context = adaq_factor_research::ResearchEvidenceContext::establish_for_stage(
        draft,
        stage,
        Default::default(),
    )
    .map_err(|error| error.to_string())?;
    state.store_research_context(context)
}

#[tauri::command]
async fn research_factor_context_establish(
    feature_dataset_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_factor_research::ResearchEvidenceProjection, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .establish_factor_context(&user_id, &feature_dataset_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn research_context_get(
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Option<adaq_factor_research::ResearchEvidenceProjection>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.research_context_for_user(&user_id)
}

#[tauri::command]
fn research_context_freeze(
    operation_id: String,
    stage: adaq_factor_research::ResearchStage,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_factor_research::FrozenResearchEvidence, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    if operation_id.trim().is_empty() {
        return Err("research context operation ID must be non-empty".into());
    }
    state.freeze_research_context(&user_id, operation_id, stage)
}

#[tauri::command]
fn research_context_frozen_get(
    operation_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Option<adaq_factor_research::FrozenResearchEvidence>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    if operation_id.trim().is_empty() {
        return Err("research context operation ID must be non-empty".into());
    }
    state.frozen_research_evidence(&user_id, &operation_id)
}

#[tauri::command]
fn research_context_for_attempt(
    attempt_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Option<adaq_factor_research::FrozenResearchEvidence>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    if attempt_id.trim().is_empty() {
        return Err("research attempt ID must be non-empty".into());
    }
    state.research_context_for_attempt(&user_id, &attempt_id)
}

/// Tauri Component Library commands are thin adapters: they deserialize
/// the existing contract, delegate to the Tauri-independent Component
/// Library module, and serialize the result. Command names and camelCase
/// shapes are frozen.
#[tauri::command]
async fn component_import(
    mut request: component_library::ComponentImportRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<component_library::LibraryComponent, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .components
            .import(&request.user_id, &request.bytes)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn component_qualify(
    mut request: component_library::ComponentImportRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_component_tooling::QualificationAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .components
            .qualify(&request.user_id, &request.bytes)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn component_list(
    mut request: component_library::ComponentUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<component_library::LibraryComponent>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: component_library::ComponentPageRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<component_library::ComponentPage, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: component_library::ComponentArchiveRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<bool, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .components
        .is_imported(&request.user_id, &request.archive_sha256)
}

#[tauri::command]
fn backtest_compatible_factors(
    mut request: component_library::BacktestDependencyRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .components
        .compatible_factors(&request.user_id, &request.strategy_archive_sha256)
}

#[tauri::command]
fn component_delete(
    mut request: component_library::ComponentDeleteRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: dataset_generation::DatasetGenerationRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<dataset_generation::Attempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let user_id = request.user_id.clone();
    let operation_id = format!(
        "model-dataset:{}:{}",
        request.snapshot_id, request.model_archive_sha256
    );
    state.require_frozen_research_evidence(
        &request.user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Models,
    )?;
    let attempt = state.generation.start(request)?;
    state.record_research_attempt_binding(
        &user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Models,
        &attempt.attempt_id,
    )?;
    Ok(attempt)
}

#[tauri::command]
fn dataset_generation_retry(
    attempt_id: String,
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<dataset_generation::Attempt, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    let (operation_id, stage) = state
        .research_attempt_binding(&user_id, &attempt_id)?
        .ok_or("research Context binding is missing for this Attempt")?;
    let attempt = state.generation.retry(&attempt_id, &user_id)?;
    state.record_research_attempt_binding(&user_id, &operation_id, stage, &attempt.attempt_id)?;
    Ok(attempt)
}

#[tauri::command]
async fn dataset_generation_list(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<dataset_generation::Attempt>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    state.generation.cancel(&attempt_id, &user_id)
}

macro_rules! factor_blocking_command {
    ($name:ident, $request:ty, $method:ident, $result:ty) => {
        #[tauri::command]
        async fn $name(
            mut request: $request,
            window: WebviewWindow,
            auth: State<'_, auth::AuthState>,
            app: tauri::AppHandle,
        ) -> Result<$result, String> {
            request.user_id = auth.user_id_for_window(window.label())?;
            tauri::async_runtime::spawn_blocking(move || {
                app.state::<Arc<LocalResearchState>>()
                    .factor
                    .$method(request)
            })
            .await
            .map_err(|error| error.to_string())?
        }
    };
}

#[tauri::command]
async fn factor_candidate_build(
    mut request: factor_research::FactorCandidateBuildRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let user_id = request.user_id.clone();
    let operation_id = request.operation_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.freeze_research_context(
            &user_id,
            operation_id.clone(),
            adaq_factor_research::ResearchStage::Factors,
        )?;
        let attempt = state.factor.build_candidate(request)?;
        state.record_research_attempt_binding(
            &user_id,
            &operation_id,
            adaq_factor_research::ResearchStage::Factors,
            &attempt.attempt_id,
        )?;
        Ok(attempt)
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
async fn factor_candidate_publish(
    mut request: factor_research::FactorCandidatePublishRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorCandidateView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .publish_factor_candidate(request)
    })
    .await
    .map_err(|error| error.to_string())?
}
factor_blocking_command!(
    factor_candidate_list,
    factor_research::FactorPageRequest,
    list_candidates,
    factor_research::FactorPage<factor_research::FactorCandidateView>
);
factor_blocking_command!(
    factor_candidate_get,
    factor_research::FactorEvidenceRequest,
    get_candidate,
    factor_research::FactorCandidateView
);
factor_blocking_command!(
    factor_component_prepare,
    factor_research::FactorComponentPrepareRequest,
    prepare_component,
    factor_research::FactorAttemptView
);
factor_blocking_command!(
    factor_component_candidate_get,
    factor_research::FactorAttemptRequest,
    get_component_candidate,
    factor_research::FactorComponentCandidateView
);
factor_blocking_command!(
    factor_component_qualification_prepare,
    factor_research::FactorComponentQualificationPrepareRequest,
    prepare_component_qualification,
    factor_research::FactorAttemptView
);
factor_blocking_command!(
    factor_component_qualification_get,
    factor_research::FactorAttemptRequest,
    get_component_qualification,
    factor_research::FactorComponentQualificationView
);
#[tauri::command]
async fn factor_materialization_start(
    payload: serde_json::Value,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    let operation_id = payload
        .get("operationId")
        .and_then(serde_json::Value::as_str)
        .ok_or("factor materialization operation ID is required")?
        .to_owned();
    let mut request: factor_research::FactorMaterializationStartRequest =
        serde_json::from_value(payload).map_err(|error| error.to_string())?;
    request.user_id = auth.user_id_for_window(window.label())?;
    let user_id = request.user_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.require_factor_context_for_request(
            &user_id,
            &operation_id,
            &request.protocol.feature_dataset_id,
            &request.protocol.feature_plan_hash,
            &request.protocol.market_data_snapshot_id,
            &request.protocol.point_in_time_universe_id,
            Some((
                request.protocol.observation_range.start_time_ms,
                request.protocol.observation_range.end_time_ms,
            )),
            true,
            &request.protocol.market_context.asset_class,
            &request.protocol.market_context.venue,
            &request.protocol.market_context.point_in_time_universe_id,
        )?;
        let attempt = state.factor.start_materialization(request)?;
        state.record_research_attempt_binding(
            &user_id,
            &operation_id,
            adaq_factor_research::ResearchStage::Factors,
            &attempt.attempt_id,
        )?;
        Ok(attempt)
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
async fn factor_materialization_start_from_context(
    mut request: factor_research::FactorMaterializationContextStartRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .start_factor_materialization_from_context(request)
    })
    .await
    .map_err(|error| error.to_string())?
}
factor_blocking_command!(
    factor_materialization_protocol_freeze,
    factor_research::FactorMaterializationProtocolFreezeRequest,
    freeze_materialization_protocol,
    adaq_factor_research::FactorMaterializationProtocol
);
#[tauri::command]
async fn factor_evaluation_start(
    payload: serde_json::Value,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    let operation_id = payload
        .get("operationId")
        .and_then(serde_json::Value::as_str)
        .ok_or("factor evaluation operation ID is required")?
        .to_owned();
    let mut request: factor_research::FactorEvaluationStartRequest =
        serde_json::from_value(payload).map_err(|error| error.to_string())?;
    request.user_id = auth.user_id_for_window(window.label())?;
    let user_id = request.user_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        let (range_start_ms, range_end_ms) = factor_evaluation_context_range(&request.protocol)?;
        state.require_factor_context_for_request(
            &user_id,
            &operation_id,
            &request.protocol.feature_dataset_id,
            &request.protocol.feature_plan_hash,
            &request.protocol.market_data_snapshot_id,
            &request.protocol.point_in_time_universe_id,
            Some((range_start_ms, range_end_ms)),
            false,
            &request.protocol.market_context.asset_class,
            &request.protocol.market_context.venue,
            &request.protocol.market_context.point_in_time_universe_id,
        )?;
        state.validate_factor_evaluation_inputs_from_host(&request)?;
        let attempt = state.factor.start_evaluation(request)?;
        state.record_research_attempt_binding(
            &user_id,
            &operation_id,
            adaq_factor_research::ResearchStage::Factors,
            &attempt.attempt_id,
        )?;
        Ok(attempt)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn factor_evaluation_start_from_context(
    mut request: factor_research::FactorEvaluationContextStartRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .start_factor_evaluation_from_context(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn factor_evaluation_context_range(
    protocol: &adaq_factor_research::FactorEvaluationProtocol,
) -> Result<(i64, i64), String> {
    let mut start_time_ms = i64::MAX;
    let mut end_time_ms = i64::MIN;
    let mut include = |range: &adaq_factor_research::ObservationRange| {
        start_time_ms = start_time_ms.min(range.start_time_ms);
        end_time_ms = end_time_ms.max(range.end_time_ms);
    };
    for window in &protocol.windows {
        include(&window.selection);
        include(&window.evaluation);
        for range in [
            window.training.as_ref(),
            window.fitting.as_ref(),
            window.normalization.as_ref(),
            window.target_construction.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            include(range);
        }
    }
    if start_time_ms == i64::MAX || end_time_ms == i64::MIN {
        return Err("factor-context-range-mismatch".into());
    }
    Ok((start_time_ms, end_time_ms))
}

factor_blocking_command!(
    factor_evaluation_protocol_freeze,
    factor_research::FactorEvaluationProtocolFreezeRequest,
    freeze_evaluation_protocol,
    adaq_factor_research::FactorEvaluationProtocol
);
factor_blocking_command!(
    factor_attempt_list,
    factor_research::FactorPageRequest,
    list_attempts,
    factor_research::FactorPage<factor_research::FactorAttemptView>
);
factor_blocking_command!(
    factor_attempt_get,
    factor_research::FactorAttemptRequest,
    get_attempt,
    factor_research::FactorAttemptView
);
factor_blocking_command!(
    factor_attempt_cancel,
    factor_research::FactorAttemptRequest,
    cancel,
    ()
);
#[tauri::command]
async fn factor_attempt_retry(
    mut request: factor_research::FactorAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let user_id = request.user_id.clone();
    let attempt_id = request.attempt_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        let (operation_id, stage) = state
            .research_attempt_binding(&user_id, &attempt_id)?
            .ok_or("research Context binding is missing for this Attempt")?;
        state.require_frozen_research_evidence(&user_id, &operation_id, stage)?;
        let attempt = state.factor.retry(request)?;
        state.record_research_attempt_binding(
            &user_id,
            &operation_id,
            stage,
            &attempt.attempt_id,
        )?;
        Ok(attempt)
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
async fn factor_component_retry(
    mut request: factor_research::FactorAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<factor_research::FactorAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>().factor.retry(request)
    })
    .await
    .map_err(|error| error.to_string())?
}
factor_blocking_command!(
    factor_dataset_list,
    factor_research::FactorPageRequest,
    list_datasets,
    factor_research::FactorPage<factor_research::FactorDatasetView>
);
factor_blocking_command!(
    factor_dataset_get,
    factor_research::FactorEvidenceRequest,
    get_dataset,
    factor_research::FactorDatasetView
);
factor_blocking_command!(
    factor_dataset_rows,
    factor_research::FactorDatasetRowsRequest,
    dataset_rows,
    factor_research::FactorDatasetRowsPage
);
factor_blocking_command!(
    factor_dataset_delete,
    factor_research::FactorEvidenceRequest,
    delete_dataset,
    ()
);
factor_blocking_command!(
    factor_report_list,
    factor_research::FactorPageRequest,
    list_reports,
    factor_research::FactorPage<factor_research::FactorReportView>
);
factor_blocking_command!(
    factor_report_get,
    factor_research::FactorEvidenceRequest,
    get_report,
    factor_research::FactorReportView
);
factor_blocking_command!(
    factor_family_register,
    factor_research::FactorFamilyRegisterRequest,
    register_family,
    factor_research::FactorFamilyView
);
factor_blocking_command!(
    factor_family_grid_register,
    factor_research::FactorGridFamilyRegisterRequest,
    register_grid_family,
    factor_research::FactorAttemptView
);
factor_blocking_command!(
    factor_family_list,
    factor_research::FactorPageRequest,
    list_families,
    factor_research::FactorPage<factor_research::FactorFamilyView>
);
factor_blocking_command!(
    factor_family_get,
    factor_research::FactorEvidenceRequest,
    get_family,
    factor_research::FactorFamilyView
);
factor_blocking_command!(
    factor_trial_update,
    factor_research::FactorTrialUpdateRequest,
    update_trial,
    ()
);
factor_blocking_command!(
    factor_lineage_get,
    factor_research::FactorEvidenceRequest,
    lineage,
    factor_research::FactorLineageView
);
factor_blocking_command!(
    factor_policy_save,
    factor_research::FactorPolicySaveRequest,
    save_policy,
    factor_research::FactorPolicyView
);
factor_blocking_command!(
    factor_policy_list,
    factor_research::FactorPageRequest,
    list_policies,
    factor_research::FactorPage<factor_research::FactorPolicyView>
);
factor_blocking_command!(
    factor_promotion_protocol_freeze,
    factor_research::FactorPromotionProtocolFreezeRequest,
    freeze_promotion_protocol,
    adaq_factor_research::PromotionProtocol
);
factor_blocking_command!(
    factor_decision_record,
    factor_research::FactorDecisionRecordRequest,
    record_decision,
    factor_research::FactorDecisionView
);
factor_blocking_command!(
    factor_decision_save,
    factor_research::FactorDecisionSaveRequest,
    save_decision,
    factor_research::FactorDecisionView
);
factor_blocking_command!(
    factor_decision_list,
    factor_research::FactorPageRequest,
    list_decisions,
    factor_research::FactorPage<factor_research::FactorDecisionView>
);
factor_blocking_command!(
    factor_decision_library,
    factor_research::FactorPageRequest,
    list_decision_library,
    factor_research::FactorPage<factor_research::FactorDecisionView>
);
factor_blocking_command!(
    factor_reference_add,
    factor_research::FactorReferenceRequest,
    add_reference,
    ()
);
factor_blocking_command!(
    factor_reference_remove,
    factor_research::FactorReferenceRequest,
    remove_reference,
    ()
);
factor_blocking_command!(
    factor_m12_eligibility,
    factor_research::FactorM12Request,
    m12_eligibility,
    adaq_factor_research::M12Eligibility
);

/// Tauri Feature commands are thin adapters: they deserialize the existing
/// contract, delegate to the Tauri-independent Feature lifecycle module,
/// and serialize the result. Commands validate the User and request and
/// return promptly; heavy Fitting and Materialization work runs in the
/// module's one persistent FIFO background runner.
#[tauri::command]
fn feature_definition_validate(
    mut request: features::DefinitionDraftRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<features::DraftValidationView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.validate_definition_draft(request)
}

#[tauri::command]
fn feature_definition_publish(
    mut request: features::DefinitionPublishRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<features::DefinitionView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.publish_definition(request)
}

#[tauri::command]
async fn feature_definition_list(
    mut request: features::FeatureUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<features::DefinitionView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .list_definitions(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_definition_get(
    mut request: features::DefinitionIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<features::DefinitionView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .get_definition(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_definition_preview(
    mut request: features::FeaturePreviewRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<features::FeaturePreviewView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .preview_definition_draft(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn feature_plan_freeze(
    mut request: features::FeaturePlanDraftRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<features::PlanFreezeView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.freeze_plan_for_user(request)
}

#[tauri::command]
fn feature_fitting_start(
    payload: serde_json::Value,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<features::FittingAttemptView, String> {
    let operation_id = payload
        .get("operationId")
        .and_then(serde_json::Value::as_str)
        .ok_or("feature fitting operation ID is required")?
        .to_owned();
    let mut request: features::FeatureFittingStartRequest =
        serde_json::from_value(payload).map_err(|error| error.to_string())?;
    request.user_id = auth.user_id_for_window(window.label())?;
    let frozen = state.require_frozen_research_evidence(
        &request.user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Features,
    )?;
    let attempt = state
        .features
        .start_fitting_with_evidence(request, &frozen)?;
    state.record_research_attempt_binding(
        &attempt.user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Features,
        &attempt.attempt_id,
    )?;
    Ok(attempt)
}

#[tauri::command]
async fn feature_fitting_list(
    mut request: features::FeatureUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<features::FittingAttemptView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .list_fitting_attempts(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_fitting_get(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<features::FittingAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .get_fitting_attempt(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn feature_fitting_cancel(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.cancel_fitting_attempt(request)
}

#[tauri::command]
fn feature_fitting_retry(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<features::FittingAttemptView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let (operation_id, stage) = state
        .research_attempt_binding(&request.user_id, &request.attempt_id)?
        .ok_or("research Context binding is missing for this Attempt")?;
    state.require_frozen_research_evidence(&request.user_id, &operation_id, stage)?;
    let attempt = state.features.retry_fitting_attempt(request)?;
    state.record_research_attempt_binding(
        &attempt.user_id,
        &operation_id,
        stage,
        &attempt.attempt_id,
    )?;
    Ok(attempt)
}

#[tauri::command]
async fn feature_artifact_list(
    mut request: features::FeatureUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<features::ArtifactView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .list_artifacts(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_artifact_get(
    mut request: features::FeatureArtifactRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<features::ArtifactView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .get_artifact(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn feature_artifact_delete(
    mut request: features::FeatureArtifactRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.delete_artifact(request)
}

#[tauri::command]
fn feature_materialization_start(
    payload: serde_json::Value,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_feature_engine::MaterializationAttempt, String> {
    let operation_id = payload
        .get("operationId")
        .and_then(serde_json::Value::as_str)
        .ok_or("feature materialization operation ID is required")?
        .to_owned();
    let mut request: features::FeatureMaterializationStartRequest =
        serde_json::from_value(payload).map_err(|error| error.to_string())?;
    request.user_id = auth.user_id_for_window(window.label())?;
    let frozen = state.require_frozen_research_evidence(
        &request.user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Features,
    )?;
    let attempt = state
        .features
        .start_materialization_with_evidence(request, &frozen)?;
    state.record_research_attempt_binding(
        &attempt.user_id,
        &operation_id,
        adaq_factor_research::ResearchStage::Features,
        &attempt.attempt_id,
    )?;
    Ok(attempt)
}

#[tauri::command]
async fn feature_materialization_list(
    mut request: features::FeatureUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_feature_engine::MaterializationAttempt>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .list_materialization_attempts(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_materialization_get(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_feature_engine::MaterializationAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .get_materialization_attempt(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn feature_materialization_cancel(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.cancel_materialization_attempt(request)
}

#[tauri::command]
fn feature_materialization_retry(
    mut request: features::FeatureAttemptRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_feature_engine::MaterializationAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let (operation_id, stage) = state
        .research_attempt_binding(&request.user_id, &request.attempt_id)?
        .ok_or("research Context binding is missing for this Attempt")?;
    state.require_frozen_research_evidence(&request.user_id, &operation_id, stage)?;
    let attempt = state.features.retry_materialization_attempt(request)?;
    state.record_research_attempt_binding(
        &attempt.user_id,
        &operation_id,
        stage,
        &attempt.attempt_id,
    )?;
    Ok(attempt)
}

#[tauri::command]
async fn feature_dataset_list(
    mut request: features::FeatureUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<features::FeatureDatasetView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .list_datasets(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_dataset_get(
    mut request: features::FeatureDatasetRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<features::FeatureDatasetView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .get_dataset(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_dataset_summary(
    mut request: features::FeatureDatasetRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_feature_engine::FeatureOutputSummary>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .dataset_summary(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn feature_dataset_rows(
    mut request: features::FeatureDatasetRowsRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_feature_engine::FeatureDatasetPage, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .features
            .dataset_rows(request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn feature_dataset_delete(
    mut request: features::FeatureDatasetRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.features.delete_dataset(request)
}

/// Tauri Validation commands are thin adapters: they deserialize the
/// existing contract, delegate to the Tauri-independent Validation Studies
/// module, and serialize the result. Command names and camelCase shapes are
/// frozen.
#[tauri::command]
fn validation_protocol_create(
    mut request: validation::ValidationProtocolCreateRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<validation::ValidationProtocol, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    request.user_id = user_id.clone();
    request.run.user_id = user_id.clone();
    if let Some(cross_market) = &mut request.cross_market {
        for context in &mut cross_market.contexts {
            if let Some(run_override) = &mut context.run_override {
                run_override.user_id = user_id.clone();
            }
        }
    }
    state.validation.create_protocol(request)
}

#[tauri::command]
async fn validation_protocol_list(
    mut request: component_library::ComponentUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<validation::ValidationProtocol>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.validation.list_protocols(&request.user_id)
}

#[tauri::command]
fn validation_report_run(
    mut request: validation::ValidationProtocolIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<validation::ValidationReport, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .validation
        .run_report(&request.user_id, &request.protocol_id)
}

#[tauri::command]
async fn validation_report_list(
    mut request: component_library::ComponentUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<validation::ValidationReport>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.validation.list_reports(&request.user_id)
}

#[tauri::command]
fn validation_report_export(
    mut request: validation::ValidationProtocolIdRequest,
    format: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<String, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_snapshot::ReadableSnapshotListRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<MarketDataSnapshot>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_snapshot::UniverseSnapshotRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_snapshot::UniverseSnapshotListRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_snapshot::UniverseSnapshotPage, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.snapshots.cancel_download(&user_id, &request.task_id)
}

/// Tauri Data Pipeline commands are thin adapters: provider-neutral typed
/// records enter here, while raw provider payloads stay in the pipeline's
/// immutable Source evidence and never cross into GUI state.
#[tauri::command]
async fn market_data_pipeline_publish(
    mut request: market_data_pipeline::PublishRequest,
    on_event: Channel<adaq_data_pipeline::PipelineProgress>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::PublicationView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.pipeline.cancel(&task_id, &user_id).map_err(string)
}

#[tauri::command]
async fn foundation_acquisition_history(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::FoundationAcquisitionView>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .foundation_acquisition_history(&user_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn market_data_pipeline_list(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::PipelineDatasetSummary>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::DeriveRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::DerivedMarketDataset, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::DerivedMarketDataset>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::DerivedMarketDataset, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::QualityView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::PipelineFailure>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::SnapshotRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::SnapshotPublicationView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .publish_pipeline_snapshot_for_user_with_policy(
                &request.user_id,
                &request.canonical_id,
                request.allow_degraded,
                request.publication_evidence_name.clone(),
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
    mut request: market_data_pipeline::DerivedSnapshotRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::SnapshotPublicationView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::DeleteRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::OkxInstrumentMasterRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::okx::InstrumentMasterSnapshot, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let operation_id = request.operation_id();
    let user_id = request.user_id;
    let ignore_untradable = request.ignore_untradable;
    let minimum_quote_volume_24h = if request.minimum_quote_volume_24h.trim().is_empty() {
        rust_decimal::Decimal::from(5_000_000)
    } else {
        request
            .minimum_quote_volume_24h
            .parse::<rust_decimal::Decimal>()
            .map_err(|_| "minimum quote volume must be a valid decimal".to_owned())?
    };
    let catalog_name = request.catalog_name.clone();
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .okx
        .begin_acquisition(&operation_id, &user_id)
        .map_err(string)?;
    state.foundation_acquisition_start(&user_id, &operation_id, "crypto", "okx")?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(
            state.okx.acquire_instrument_master_filtered_with_cancel(
                &user_id,
                &cancellation,
                ignore_untradable,
                minimum_quote_volume_24h,
                catalog_name,
            ),
        )
        .map_err(string);
        let finish = state.okx.finish_acquisition(&operation_id);
        let (state_name, error) = match &result {
            Ok(_) => ("completed", None),
            Err(error) if error.contains("cancelled") => ("cancelled", Some(error.as_str())),
            Err(error) => ("failed", Some(error.as_str())),
        };
        let _ =
            state.foundation_acquisition_finish(&user_id, &operation_id, state_name, None, error);
        match (result, finish) {
            (Ok(snapshot), Ok(())) => Ok(snapshot),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn okx_instrument_master_cancel(
    mut request: market_data_pipeline::AshareAcquisitionCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .okx
        .cancel_acquisition(&request.operation_id, &request.user_id)
        .map_err(string)
}

#[tauri::command]
async fn okx_instrument_master_list(
    mut request: market_data_pipeline::UserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::InstrumentMasterSnapshot>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::UniverseRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::okx::PointInTimeInstrumentUniverse, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: adaq_data_pipeline::okx::OkxBackfillRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::PublicationView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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

fn auto_validate_okx_sources(
    state: &LocalResearchState,
    request: &adaq_data_pipeline::okx::OkxBackfillRequest,
    sources: Vec<adaq_data_pipeline::SourceMarketDataset>,
    cancellation: &adaq_data_pipeline::CancellationToken,
    on_event: &Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
) -> Result<Vec<adaq_data_pipeline::SourceMarketDataset>, String> {
    if cancellation.is_cancelled() {
        return Err("OKX backfill cancelled".into());
    }
    let (start_time_ms, end_time_ms) = sources
        .first()
        .map(|source| {
            (
                source
                    .identity
                    .request_parameters
                    .get("startTimeMs")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(request.start_time_ms),
                source
                    .identity
                    .request_parameters
                    .get("endTimeMs")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(request.end_time_ms),
            )
        })
        .unwrap_or((request.start_time_ms, request.end_time_ms));
    let validation_request = adaq_data_pipeline::okx::OkxSourcePublicationRequest {
        task_id: request.task_id.clone(),
        user_id: request.user_id.clone(),
        source_ids: sources
            .iter()
            .map(|source| source.source_id.clone())
            .collect(),
        start_time_ms,
        end_time_ms,
        interval: request.interval,
        instrument_codes: request.instrument_codes.clone(),
        publication_evidence_name: request.publication_evidence_name.clone(),
    };
    state
        .okx
        .publish_sources(&validation_request, cancellation.clone(), |event| {
            let _ = on_event.send(event);
        })
        .map(|_| sources)
        .map_err(string)
}

#[tauri::command]
async fn okx_backfill_source(
    mut request: adaq_data_pipeline::okx::OkxBackfillRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::SourceView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &user_id)
        .map_err(string)?;
    state.foundation_okx_backfill_start(&request)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(state.okx.backfill_source_only(
            &request,
            cancellation.clone(),
            |event| {
                let _ = on_event.send(event);
            },
        ))
        .map_err(string)
        .and_then(|sources| {
            auto_validate_okx_sources(&state, &request, sources, &cancellation, &on_event)
        });
        let finish = state.okx.finish_backfill(&task_id);
        let (state_name, revision, error) = match &result {
            Ok(sources) => (
                "completed",
                sources.iter().map(|source| source.revision).max(),
                None,
            ),
            Err(error) if error.contains("cancelled") => ("cancelled", None, Some(error.as_str())),
            Err(error) => ("failed", None, Some(error.as_str())),
        };
        let history =
            state.foundation_acquisition_finish(&user_id, &task_id, state_name, revision, error);
        let sources = result?;
        finish.map_err(string)?;
        history?;
        Ok(sources
            .into_iter()
            .map(market_data_pipeline::SourceView::from)
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_publish_sources(
    mut request: adaq_data_pipeline::okx::OkxSourcePublicationRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::PublicationView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &user_id)
        .map_err(string)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = state
            .okx
            .publish_sources(&request, cancellation.clone(), |event| {
                let _ = on_event.send(event);
            })
            .map_err(string)
            .map(|publications| {
                publications
                    .into_iter()
                    .map(market_data_pipeline::PublicationView::from)
                    .collect::<Vec<_>>()
            });
        let finish = state.okx.finish_backfill(&task_id);
        match (result, finish) {
            (Ok(publications), Ok(())) => Ok(publications),
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(string(error)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_publish_gate_two(
    mut request: adaq_data_pipeline::okx::OkxSourcePublicationRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<market_data_pipeline::GateTwoPublicationView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &user_id)
        .map_err(string)?;
    state.foundation_acquisition_start(&user_id, &task_id, "crypto", "okx")?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = state
            .okx
            .publish_validated_sources(&request, cancellation.clone(), |event| {
                let _ = on_event.send(event);
            })
            .map_err(string)
            .and_then(|publications| {
                if cancellation.is_cancelled() {
                    return Err("OKX research data publication was cancelled".into());
                }
                state
                    .publish_okx_backfill(
                        &request.user_id,
                        request.start_time_ms,
                        request.end_time_ms,
                        request.interval,
                        &request.instrument_codes,
                        &cancellation,
                        &publications,
                        request.publication_evidence_name.clone(),
                    )
                    .map(|universe| market_data_pipeline::GateTwoPublicationView {
                        publications: publications
                            .into_iter()
                            .map(market_data_pipeline::PublicationView::from)
                            .collect(),
                        universe_snapshot_id: universe.snapshot_id,
                        publication_evidence_name: request.publication_evidence_name.clone(),
                    })
            });
        let finish = state.okx.finish_backfill(&task_id);
        let (state_name, revision, error) = match &result {
            Ok(publication) => (
                "completed",
                publication
                    .publications
                    .iter()
                    .map(|item| item.source_revision)
                    .max(),
                None,
            ),
            Err(error) if error.contains("cancelled") => ("cancelled", None, Some(error.as_str())),
            Err(error) => ("failed", None, Some(error.as_str())),
        };
        let history =
            state.foundation_acquisition_finish(&user_id, &task_id, state_name, revision, error);
        let publication = result?;
        finish.map_err(string)?;
        history?;
        Ok(publication)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_backfill_retry(
    mut retry: market_data_pipeline::BackfillRetryRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::SourceView>, String> {
    retry.user_id = auth.user_id_for_window(window.label())?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let mut request = state.foundation_okx_backfill_request(&retry.user_id, &retry.operation_id)?;
    request.user_id = retry.user_id;
    request.task_id = retry.retry_operation_id;
    request.checkpoint_operation_id = Some(retry.operation_id);
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &user_id)
        .map_err(string)?;
    state.foundation_okx_backfill_start(&request)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(state.okx.backfill_source_only(
            &request,
            cancellation.clone(),
            |event| {
                let _ = on_event.send(event);
            },
        ))
        .map_err(string)
        .and_then(|sources| {
            auto_validate_okx_sources(&state, &request, sources, &cancellation, &on_event)
        });
        let finish = state.okx.finish_backfill(&task_id);
        let (state_name, revision, error) = match &result {
            Ok(sources) => (
                "completed",
                sources.iter().map(|source| source.revision).max(),
                None,
            ),
            Err(error) if error.contains("cancelled") => ("cancelled", None, Some(error.as_str())),
            Err(error) => ("failed", None, Some(error.as_str())),
        };
        let history =
            state.foundation_acquisition_finish(&user_id, &task_id, state_name, revision, error);
        let sources = result?;
        finish.map_err(string)?;
        history?;
        Ok(sources
            .into_iter()
            .map(market_data_pipeline::SourceView::from)
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn okx_backfill_publish(
    mut request: adaq_data_pipeline::okx::OkxBackfillRequest,
    on_event: Channel<adaq_data_pipeline::okx::OkxBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<market_data_pipeline::PublicationView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let task_id = request.task_id.clone();
    let user_id = request.user_id.clone();
    let cancellation = state
        .okx
        .begin_backfill(&task_id, &user_id)
        .map_err(string)?;
    state.foundation_acquisition_start(&user_id, &task_id, "crypto", "okx")?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(state.okx.backfill(
            &request,
            cancellation.clone(),
            |event| {
                let _ = on_event.send(event);
            },
        ))
        .map_err(string)
        .and_then(|publications| {
            if cancellation.is_cancelled() {
                return Err("OKX backfill cancelled".into());
            }
            state
                .publish_okx_backfill(
                    &request.user_id,
                    request.start_time_ms,
                    request.end_time_ms,
                    request.interval,
                    &request.instrument_codes,
                    &cancellation,
                    &publications,
                    request.publication_evidence_name.clone(),
                )
                .map(|_| publications)
        });
        let finish = state.okx.finish_backfill(&task_id);
        let (state_name, revision, error) = match &result {
            Ok(publications) => (
                "completed",
                publications.iter().map(|item| item.source.revision).max(),
                None,
            ),
            Err(error) if error.contains("cancelled") => ("cancelled", None, Some(error.as_str())),
            Err(error) => ("failed", None, Some(error.as_str())),
        };
        let history =
            state.foundation_acquisition_finish(&user_id, &task_id, state_name, revision, error);
        let publications = result?;
        finish.map_err(string)?;
        history?;
        Ok(publications
            .into_iter()
            .map(market_data_pipeline::PublicationView::from)
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn okx_backfill_cancel(
    mut request: market_data_pipeline::BackfillCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .okx
        .cancel_backfill(&request.task_id, &request.user_id)
        .map_err(string)
}

#[tauri::command]
async fn okx_acquisition_status(
    mut request: market_data_pipeline::UserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::OkxAcquisitionStatus>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: market_data_pipeline::UserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::okx::OkxStreamHealth>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn ashare_instrument_master_acquire(
    mut request: market_data_pipeline::AshareInstrumentMasterRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareInstrumentMasterSnapshotDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let operation_id = request.operation_id();
    let user_id = request.user_id.clone();
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .ashare
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    state.foundation_acquisition_start(&user_id, &operation_id, "a-shares", "sse/szse")?;
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
        let (state_name, error) = match &result {
            Ok(_) => ("completed", None),
            Err(error) if error.contains("cancelled") => ("cancelled", Some(error.as_str())),
            Err(error) => ("failed", Some(error.as_str())),
        };
        let _ =
            state.foundation_acquisition_finish(&user_id, &operation_id, state_name, None, error);
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
#[cfg(feature = "deferred-equity")]
async fn ashare_instrument_master_list(
    mut request: market_data_pipeline::UserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::a_share::AshareInstrumentMasterSnapshotDto>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn ashare_universe(
    mut request: market_data_pipeline::UniverseRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AsharePointInTimeUniverse, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn ashare_calendar_acquire(
    mut request: market_data_pipeline::AshareCalendarRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::a_share::AshareCalendarSnapshotDto>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn ashare_corporate_actions_acquire(
    mut request: market_data_pipeline::AshareCorporateActionRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareCorporateActionDatasetDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn ashare_backfill(
    mut request: adaq_data_pipeline::a_share::AshareBackfillRequest,
    on_event: Channel<adaq_data_pipeline::a_share::AshareBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Option<market_data_pipeline::PublicationView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
fn ashare_backfill_cancel(
    mut request: market_data_pipeline::BackfillCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .ashare
        .cancel_backfill(&request.user_id, &request.task_id)
        .map_err(string)
}

#[tauri::command]
#[cfg(feature = "deferred-equity")]
fn ashare_acquisition_cancel(
    mut request: market_data_pipeline::AshareAcquisitionCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .ashare
        .cancel_acquisition(&request.user_id, &request.operation_id)
        .map_err(string)
}

#[tauri::command]
#[cfg(feature = "deferred-equity")]
async fn ashare_workspace(
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::a_share::AshareMarketWorkspaceDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_instrument_master_acquire(
    mut request: market_data_pipeline::UsEquityInstrumentMasterRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityInstrumentMasterSnapshotDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let operation_id = request.operation_id();
    let user_id = request.user_id;
    let state = app.state::<Arc<LocalResearchState>>().inner().clone();
    let cancellation = state
        .us_equity
        .begin_acquisition(&user_id, &operation_id)
        .map_err(string)?;
    state.foundation_acquisition_start(
        &user_id,
        &operation_id,
        "us-equities",
        adaq_data_core::stock_us::STOCK_US_SRC,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        let operation_user_id = user_id.clone();
        let cancellation_for_operation = cancellation.clone();
        let operation_state = state.clone();
        let result = match adaq_data_core::stock_us::StockUsClient::new() {
            Ok(client) => {
                tauri::async_runtime::block_on(operation_state.us_equity.acquire_instrument_master(
                    &operation_user_id,
                    &client,
                    &cancellation_for_operation,
                    unix_now_ms(),
                ))
                .map_err(string)
            }
            Err(error) => Err(string(error)),
        };
        let finish = state.us_equity.finish_acquisition(&user_id, &operation_id);
        let (state_name, error) = match &result {
            Ok(_) => ("completed", None),
            Err(error) if error.contains("cancelled") => ("cancelled", Some(error.as_str())),
            Err(error) => ("failed", Some(error.as_str())),
        };
        let _ =
            state.foundation_acquisition_finish(&user_id, &operation_id, state_name, None, error);
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_instrument_master_list(
    mut request: market_data_pipeline::UserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Vec<adaq_data_pipeline::us_equity::UsEquityInstrumentMasterSnapshotDto>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_universe(
    mut request: market_data_pipeline::UniverseRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityPointInTimeUniverse, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_calendar_acquire(
    mut request: market_data_pipeline::UsEquityCalendarRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityCalendarSnapshotDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
        let result = match adaq_data_core::stock_us::StockUsClient::new() {
            Ok(client) => {
                tauri::async_runtime::block_on(operation_state.us_equity.acquire_calendar(
                    &operation_user_id,
                    &client,
                    venue,
                    range,
                    &cancellation_for_operation,
                    unix_now_ms(),
                ))
                .map_err(string)
            }
            Err(error) => Err(string(error)),
        };
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_backfill(
    mut request: adaq_data_pipeline::us_equity::UsEquityBackfillRequest,
    on_event: Channel<adaq_data_pipeline::us_equity::UsEquityBackfillEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Option<market_data_pipeline::PublicationView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
        let result = match adaq_data_core::stock_us::StockUsClient::new() {
            Ok(client) => tauri::async_runtime::block_on(operation_state.us_equity.backfill(
                request,
                &client,
                cancellation_for_operation,
                |event| {
                    let _ = on_event.send(event);
                },
            ))
            .map_err(string),
            Err(error) => Err(string(error)),
        };
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
#[cfg(feature = "deferred-equity")]
fn yahoo_backfill_cancel(
    mut request: market_data_pipeline::BackfillCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .us_equity
        .cancel_backfill(&request.user_id, &request.task_id)
        .map_err(string)
}

#[tauri::command]
#[cfg(feature = "deferred-equity")]
fn yahoo_acquisition_cancel(
    mut request: market_data_pipeline::AshareAcquisitionCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state
        .us_equity
        .cancel_acquisition(&request.user_id, &request.operation_id)
        .map_err(string)
}

#[tauri::command]
#[cfg(feature = "deferred-equity")]
async fn yahoo_acquisition_status(
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<Option<adaq_data_pipeline::us_equity::UsEquityAcquisitionStatus>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_snapshot(
    mut request: market_data_pipeline::UsEquitySnapshotRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityMarketSnapshotDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>().inner().clone();
        let client = adaq_data_core::stock_us::StockUsClient::new().map_err(string)?;
        tauri::async_runtime::block_on(state.us_equity.snapshot(
            &client,
            request.instrument,
            unix_now_ms(),
        ))
        .map_err(string)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
#[cfg(feature = "deferred-equity")]
async fn yahoo_stream(
    mut request: market_data_pipeline::UsEquityStreamRequest,
    on_event: Channel<adaq_data_core::alpaca::AlpacaStreamEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
#[cfg(feature = "deferred-equity")]
async fn yahoo_workspace(
    mut request: market_data_pipeline::UserEvidenceRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<adaq_data_pipeline::us_equity::UsEquityMarketWorkspaceDto, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: backtest::BacktestRunRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestPreflight, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.preflight(&request)
}

#[tauri::command]
fn backtest_run(
    mut request: backtest::BacktestRunRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.run(request)
}

#[tauri::command]
async fn backtest_list(
    mut request: backtest::BacktestListRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunPage, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.list(&request)
}

#[tauri::command]
fn backtest_get(
    mut request: backtest::BacktestRunIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.get(&request.user_id, &request.run_id)
}

#[tauri::command]
fn backtest_chart_data(
    mut request: backtest::BacktestChartRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestRunView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.chart_data(&request)
}

#[tauri::command]
fn backtest_execution_data(
    mut request: backtest::BacktestExecutionRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::BacktestExecutionPage, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.execution_data(&request)
}

#[tauri::command]
fn backtest_delete(
    mut request: backtest::BacktestRunIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.delete(&request.user_id, &request.run_id)
}

#[tauri::command]
fn strategy_project_save(
    mut request: backtest::StrategyProjectRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    request.project.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.save_strategy_project(&request.project)
}

#[tauri::command]
fn strategy_project_list(
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<adaq_backtest_core::StrategyProject>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.backtests.strategy_projects(&user_id)
}

#[tauri::command]
fn strategy_attempt_start(
    request: backtest::StrategyAttemptStartRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .start_strategy_attempt(&user_id, &request.project_id, request.window)
}

#[tauri::command]
fn strategy_attempt_complete(
    request: backtest::StrategyAttemptCompleteRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .complete_strategy_attempt(&user_id, &request.attempt_id, &request.run_id)
}

#[tauri::command]
fn strategy_attempt_begin(
    request: backtest::StrategyAttemptIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .begin_strategy_attempt(&user_id, &request.attempt_id)
}

#[tauri::command]
fn strategy_attempt_fail(
    request: backtest::StrategyAttemptFailureRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .fail_strategy_attempt(&user_id, &request.attempt_id, &request.reason)
}

#[tauri::command]
fn strategy_attempt_cancel(
    request: backtest::StrategyAttemptIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .cancel_strategy_attempt(&user_id, &request.attempt_id)
}

#[tauri::command]
fn strategy_attempt_recover(
    request: backtest::StrategyAttemptIdRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<adaq_backtest_core::StrategyAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .backtests
        .recover_strategy_attempt(&user_id, &request.attempt_id)
}

#[tauri::command]
fn portfolio_backtest_run(
    mut request: backtest::PortfolioBacktestRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<backtest::PortfolioBacktestView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.backtests.portfolio_run(request)
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
struct MarketWorkspaceBarsRequest {
    user_id: String,
    instrument: InstrumentId,
    interval: BarInterval,
    start_time_ms: i64,
    end_time_ms: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MarketWorkspaceBarsView {
    instrument: InstrumentId,
    provider: String,
    actual_upstream: String,
    method: String,
    connector_version: String,
    retrieved_at_ms: i64,
    freshness_ms: Option<i64>,
    price_basis: PriceBasis,
    quality: String,
    bars: Vec<OhlcvBar>,
    gaps: Option<Vec<BarGap>>,
    limitations: Vec<String>,
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
    user_id: String,
    subscription_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    on_event: Channel<TradeStreamEvent>,
}

#[derive(Default)]
struct TradeStreamState(Mutex<Option<ActiveTradeStream>>);

struct ActiveLevel2Stream {
    user_id: String,
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
async fn market_workspace_get_bars(
    mut request: MarketWorkspaceBarsRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<MarketWorkspaceBarsView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    if request.start_time_ms >= request.end_time_ms {
        return Err("Market Bar range must be increasing".to_owned());
    }
    let retrieved_at_ms = unix_now_ms();
    let range = HistoricalBarRange {
        start_time_ms: request.start_time_ms,
        end_time_ms: request.end_time_ms,
    };
    match request.instrument.venue.kind {
        #[cfg(feature = "deferred-equity")]
        VenueKind::ChinaAShareEquity => {
            let client = app
                .state::<Arc<LocalResearchState>>()
                .ashare
                .client()
                .clone();
            let instrument = request.instrument.clone();
            let interval = request.interval;
            let acquisition = tauri::async_runtime::spawn_blocking(move || {
                tauri::async_runtime::block_on(client.acquire_bars(
                    instrument,
                    interval,
                    range,
                    retrieved_at_ms,
                ))
                .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
            Ok(MarketWorkspaceBarsView {
                instrument: request.instrument,
                provider: acquisition.provider,
                actual_upstream: acquisition.actual_upstream,
                method: acquisition.method,
                connector_version: acquisition.connector_version,
                retrieved_at_ms: acquisition.retrieved_at_ms,
                freshness_ms: Some(retrieved_at_ms.saturating_sub(acquisition.retrieved_at_ms)),
                price_basis: acquisition
                    .bars
                    .first()
                    .map(|bar| bar.price_basis)
                    .unwrap_or(PriceBasis::Unknown),
                quality: "unknown".to_owned(),
                bars: ashare_bars(acquisition.bars)?,
                gaps: None,
                limitations: direct_bar_limitations(acquisition.limitations),
            })
        }
        #[cfg(feature = "deferred-equity")]
        VenueKind::UsEquity => {
            let instrument = request.instrument;
            let interval = request.interval;
            tauri::async_runtime::spawn_blocking(move || {
                let client = adaq_data_core::stock_us::StockUsClient::new()
                    .map_err(|error| error.to_string())?;
                let acquisition = tauri::async_runtime::block_on(client.acquire_bars(
                    instrument.clone(),
                    interval,
                    range,
                    retrieved_at_ms,
                    || false,
                ))
                .map_err(|error| error.to_string())?;
                Ok(MarketWorkspaceBarsView {
                    instrument,
                    provider: acquisition.provider,
                    actual_upstream: acquisition.actual_upstream,
                    method: acquisition.method,
                    connector_version: acquisition.connector_version,
                    retrieved_at_ms: acquisition.retrieved_at_ms,
                    freshness_ms: Some(retrieved_at_ms.saturating_sub(acquisition.retrieved_at_ms)),
                    price_basis: PriceBasis::Unadjusted,
                    quality: "unknown".to_owned(),
                    bars: alpaca_bars(acquisition.bars)?,
                    gaps: None,
                    limitations: direct_bar_limitations(acquisition.limitations),
                })
            })
            .await
            .map_err(|error| error.to_string())?
        }
        VenueKind::CryptoSpot => Err("Use the OKX market Bar command for Crypto Spot".to_owned()),
        #[cfg(not(feature = "deferred-equity"))]
        _ => Err("Only OKX Spot market bars are supported in V1".to_owned()),
    }
}

#[cfg(feature = "deferred-equity")]
fn ashare_bars(bars: Vec<adaq_data_core::a_share::AshareBar>) -> Result<Vec<OhlcvBar>, String> {
    bars.into_iter()
        .filter_map(|bar| {
            let values = [
                bar.open.as_deref(),
                bar.high.as_deref(),
                bar.low.as_deref(),
                bar.close.as_deref(),
                bar.base_volume.as_deref(),
                bar.quote_volume.as_deref(),
            ];
            if values.iter().any(Option::is_none) {
                return None;
            }
            Some(decimal_bar(bar.open_time_ms, values))
        })
        .collect()
}

#[cfg(feature = "deferred-equity")]
fn alpaca_bars(bars: Vec<adaq_data_core::alpaca::AlpacaBar>) -> Result<Vec<OhlcvBar>, String> {
    bars.into_iter()
        .filter_map(|bar| {
            let values = [
                bar.open.as_deref(),
                bar.high.as_deref(),
                bar.low.as_deref(),
                bar.close.as_deref(),
                bar.base_volume.as_deref(),
                bar.quote_volume.as_deref(),
            ];
            if values.iter().any(Option::is_none) {
                return None;
            }
            Some(decimal_bar(bar.open_time_ms, values))
        })
        .collect()
}

fn decimal_bar(open_time_ms: i64, values: [Option<&str>; 6]) -> Result<OhlcvBar, String> {
    let [open, high, low, close, base_volume, quote_volume] = values;
    Ok(OhlcvBar {
        open_time_ms,
        open: Decimal::from_str_exact(open.ok_or("missing open")?)
            .map_err(|error| error.to_string())?,
        high: Decimal::from_str_exact(high.ok_or("missing high")?)
            .map_err(|error| error.to_string())?,
        low: Decimal::from_str_exact(low.ok_or("missing low")?)
            .map_err(|error| error.to_string())?,
        close: Decimal::from_str_exact(close.ok_or("missing close")?)
            .map_err(|error| error.to_string())?,
        base_volume: Decimal::from_str_exact(base_volume.ok_or("missing base volume")?)
            .map_err(|error| error.to_string())?,
        quote_volume: Decimal::from_str_exact(quote_volume.ok_or("missing quote volume")?)
            .map_err(|error| error.to_string())?,
    })
}

fn direct_bar_limitations(mut limitations: Vec<String>) -> Vec<String> {
    limitations.push(
        "Direct provider observations are not canonical Data Quality publication evidence."
            .to_owned(),
    );
    limitations.push(
        "Bar coverage and gap evidence are not established for this direct observation.".to_owned(),
    );
    limitations
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
    mut request: WatchlistUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    database.get(&request.user_id)
}

#[tauri::command]
async fn watchlist_add(
    mut request: WatchlistInstrumentRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    database: State<'_, WatchlistDb>,
    client: State<'_, OkxClient>,
) -> Result<WatchlistState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    validate_provider_venue(&request.instrument)?;
    if request.instrument.src == "okx" {
        let instruments = client
            .list_spot_instruments()
            .await
            .map_err(|error| error.to_string())?;
        if !instruments.iter().any(|instrument| {
            instrument.code == request.instrument.code
                && instrument.status == InstrumentStatus::Live
        }) {
            return Err("only Live OKX Spot Instruments can be added".to_owned());
        }
    } else if !matches!(request.instrument.src.as_str(), "akshare-rs" | "alpaca") {
        return Err("unsupported Market Data Provider for Watchlist Instrument".to_owned());
    }
    database.add(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_remove(
    mut request: WatchlistInstrumentRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    validate_provider_venue(&request.instrument)?;
    database.remove(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_set_active(
    mut request: WatchlistInstrumentRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    validate_provider_venue(&request.instrument)?;
    database.set_active(&request.user_id, &request.instrument)
}

#[tauri::command]
fn watchlist_set_interval(
    mut request: WatchlistIntervalRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    database: State<'_, WatchlistDb>,
) -> Result<WatchlistState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: MarketSubscribeRealtimeRequest,
    on_event: Channel<TradeStreamEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
    streams: State<'_, TradeStreamState>,
) -> Result<(), DataError> {
    request.user_id = auth
        .user_id_for_window(window.label())
        .map_err(|message| DataError::new("okx", "unauthenticated", message))?;
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
    let stream_user_id = user_id.clone();
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
        user_id: stream_user_id,
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    streams: State<'_, TradeStreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    let user_id = auth
        .user_id_for_window(window.label())
        .map_err(|message| DataError::new("okx", "unauthenticated", message))?;
    if active.as_ref().is_some_and(|stream| {
        stream.user_id == user_id && stream.subscription_id == request.subscription_id
    }) {
        if let Some(previous) = active.take() {
            let _ = previous.on_event.send(TradeStreamEvent::Closed);
            previous.task.abort();
        }
    }
    Ok(())
}

#[tauri::command]
fn market_subscribe_level2(
    mut request: MarketSubscribeRealtimeRequest,
    on_event: Channel<Level2StreamEvent>,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
    streams: State<'_, Level2StreamState>,
) -> Result<(), DataError> {
    request.user_id = auth
        .user_id_for_window(window.label())
        .map_err(|message| DataError::new("okx", "unauthenticated", message))?;
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
    let stream_user_id = user_id.clone();
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
        user_id: stream_user_id,
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
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    streams: State<'_, Level2StreamState>,
) -> Result<(), DataError> {
    let mut active = streams
        .0
        .lock()
        .map_err(|error| DataError::new("okx", "internal", error.to_string()))?;
    let user_id = auth
        .user_id_for_window(window.label())
        .map_err(|message| DataError::new("okx", "unauthenticated", message))?;
    if active.as_ref().is_some_and(|stream| {
        stream.user_id == user_id && stream.subscription_id == request.subscription_id
    }) {
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
    mut request: ConnectionUserRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<connections::ProfileView>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    state.connections.list(&request.user_id)
}

#[tauri::command]
async fn connection_profile_save(
    mut request: ConnectionSaveRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<connections::ProfileView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: ConnectionProfileRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<connections::ProfileView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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
    mut request: ConnectionProfileRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
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

#[tauri::command]
fn paper_account_view(
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
) -> Result<PaperTradingWorkspaceView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let profile = state
        .connections
        .list(&user_id)?
        .into_iter()
        .find(|profile| profile.provider == connections::Provider::OkxDemo);
    let connection = match profile {
        None => PaperConnectionView {
            state: "disconnected",
            evidence: None,
        },
        Some(profile) if profile.status == connections::ProfileStatus::Usable => {
            PaperConnectionView {
                state: "connected",
                evidence: profile.last_test_evidence,
            }
        }
        Some(profile) => PaperConnectionView {
            state: "degraded",
            evidence: profile.last_test_evidence,
        },
    };
    Ok(PaperTradingWorkspaceView {
        account: state.paper_trading.view_optional(&user_id)?,
        connection,
    })
}

#[tauri::command]
async fn paper_account_reconcile(
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<paper_trading::PaperAccountView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        let profile = state
            .connections
            .list(&user_id)?
            .into_iter()
            .find(|profile| profile.provider == connections::Provider::OkxDemo)
            .ok_or_else(|| "No OKX Demo connection is configured.".to_owned())?;
        let account_id = profile.account_id.ok_or_else(|| {
            "The OKX Demo connection has no validated account identity.".to_owned()
        })?;
        state.connections.with_okx_demo_client(&user_id, |client| {
            state
                .paper_trading
                .provider_balance(&user_id, account_id, &client, unix_now_ms())
        })?
    })
    .await
    .map_err(|error| serialize_paper_account_error(error.to_string()))?;
    result.map_err(serialize_paper_account_error)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PaperTradingWorkspaceView {
    account: Option<paper_trading::PaperAccountView>,
    connection: PaperConnectionView,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PaperConnectionView {
    state: &'static str,
    evidence: Option<connections::tester::ConnectionEvidence>,
}

fn serialize_paper_account_error(error: String) -> String {
    let code = if error.contains("No OKX Demo connection") {
        "connectionMissing"
    } else if error.contains("account identity") {
        "accountIdentityInvalid"
    } else if error.contains("unusable") || error.contains("credential") {
        "connectionUnavailable"
    } else {
        "providerUnavailable"
    };
    serde_json::json!({ "code": code, "message": error }).to_string()
}

#[tauri::command]
async fn paper_order_submit(
    mut request: paper_trading::PaperOrderRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<paper_trading::PaperAccountView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.paper_trading.begin_order(&request, unix_now_ms())?;
        let side = request.side.to_lowercase();
        let result = state
            .connections
            .with_okx_demo_client(&request.user_id, |client| {
                tauri::async_runtime::block_on(client.create_order(
                    &request.instrument,
                    "limit",
                    &side,
                    &request.quantity.to_string(),
                    Some(&request.limit_price.to_string()),
                    Params::new(),
                ))
            });
        match result {
            Ok(Ok(order)) => state.paper_trading.record_order_result(
                &request.user_id,
                &request.operation_id,
                order.id,
                order.status.as_deref().unwrap_or("accepted"),
                None,
                unix_now_ms(),
            ),
            Ok(Err(_)) | Err(_) => state.paper_trading.mark_uncertain(
                &request.user_id,
                &request.operation_id,
                unix_now_ms(),
            ),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn paper_order_cancel(
    mut request: paper_trading::PaperCancelRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<paper_trading::PaperAccountView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        let provider_order_id = state
            .paper_trading
            .provider_order_id(&request.user_id, &request.operation_id)?;
        let result = state
            .connections
            .with_okx_demo_client(&request.user_id, |client| {
                tauri::async_runtime::block_on(client.cancel_order(
                    &provider_order_id,
                    &request.instrument,
                    Params::new(),
                ))
            });
        match result {
            Ok(Ok(order)) => {
                state.paper_trading.record_order_result(
                    &request.user_id,
                    &request.operation_id,
                    order.id,
                    order.status.as_deref().unwrap_or("canceled"),
                    None,
                    unix_now_ms(),
                )?;
                state.paper_trading.cancel_local_order(
                    &request.user_id,
                    &request.operation_id,
                    unix_now_ms(),
                )
            }
            Ok(Err(_)) | Err(_) => state.paper_trading.mark_uncertain(
                &request.user_id,
                &request.operation_id,
                unix_now_ms(),
            ),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn paper_order_sync(
    mut request: paper_trading::PaperSyncRequest,
    window: WebviewWindow,
    auth: State<'_, auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<paper_trading::PaperAccountView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        let provider_order_id = state
            .paper_trading
            .provider_order_id(&request.user_id, &request.operation_id)?;
        let result = state
            .connections
            .with_okx_demo_client(&request.user_id, |client| {
                tauri::async_runtime::block_on(client.fetch_orders(
                    Some(&request.instrument),
                    None,
                    None,
                    Params::new(),
                ))
            })?;
        let remote = result
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|order| order.id.as_deref() == Some(provider_order_id.as_str()))
            .ok_or_else(|| "OKX did not return the requested order.".to_owned())?;
        state.paper_trading.sync_provider_order(
            &request.user_id,
            &request.operation_id,
            &remote,
            unix_now_ms(),
        )
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
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
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                window
                    .app_handle()
                    .state::<auth::AuthState>()
                    .clear(window.label());
            }
        })
        .setup(|app| {
            app.manage(WasmLoader::default());
            app.manage(OkxClient::default());
            app.manage(TickerStreamState::default());
            app.manage(BarStreamState::default());
            app.manage(TradeStreamState::default());
            app.manage(Level2StreamState::default());
            let app_data_dir = app.path().app_data_dir()?;
            app.manage(auth::AuthState::from_environment());
            app.manage(WorkspaceInitialization::new(app_data_dir));
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
            auth_bind_session,
            auth_clear_session,
            workspace_ready,
            operations_observe,
            operations_health,
            operations_alerts,
            operations_alert_transition,
            paper_feedback_snapshot_create,
            paper_feedback_report_create,
            paper_feedback_review_decide,
            paper_account_view,
            paper_account_reconcile,
            paper_order_submit,
            paper_order_cancel,
            paper_order_sync,
            python_research::project_list,
            python_research::project_create,
            python_research::project_import,
            python_research::project_validate,
            python_research::project_freeze,
            python_research::project_export,
            python_research::research_reset,
            python_research::trust_revision,
            python_research::attempt_list,
            python_research::attempt_preview,
            python_research::attempt_start,
            python_research::attempt_cancel,
            python_research::attempt_retry,
            python_research::python_factor_demo,
            python_research::python_factor_trial_select,
            python_research::python_factor_promote,
            python_research::model_demo_run,
            python_research::model_experiment_register,
            python_research::model_experiment_list,
            python_research::model_lab_state,
            python_research::model_trial_complete,
            python_research::model_trial_fail,
            python_research::model_trial_retry,
            python_research::model_selection_record,
            python_research::model_final_evaluate,
            python_research::model_qualify_deployment,
            python_research::runtime_profile,
            python_research::runtime_prepare,
            python_research::runtime_prepare_managed,
            python_research::runtime_prepare_cancel,
            python_research::environment_sync,
            python_research::environment_sync_managed,
            python_research::environment_prepare,
            python_research::environment_prepare_managed,
            python_research::environment_for_project,
            python_research::cache_evict,
            load_factor_component,
            get_factor_schema,
            factor_metric_catalog,
            research_context_establish,
            research_factor_context_establish,
            research_context_get,
            research_context_freeze,
            research_context_frozen_get,
            research_context_for_attempt,
            market_list_spot_instruments,
            market_get_bar_series,
            market_workspace_get_bars,
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
            component_qualify,
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
            foundation_acquisition_history,
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
            okx_instrument_master_cancel,
            okx_instrument_master_list,
            okx_universe,
            okx_backfill,
            okx_backfill_source,
            okx_publish_sources,
            okx_publish_gate_two,
            okx_backfill_retry,
            okx_backfill_publish,
            okx_backfill_cancel,
            okx_acquisition_status,
            okx_stream_health,
            #[cfg(feature = "deferred-equity")]
            ashare_instrument_master_acquire,
            #[cfg(feature = "deferred-equity")]
            ashare_instrument_master_list,
            #[cfg(feature = "deferred-equity")]
            ashare_universe,
            #[cfg(feature = "deferred-equity")]
            ashare_calendar_acquire,
            #[cfg(feature = "deferred-equity")]
            ashare_corporate_actions_acquire,
            #[cfg(feature = "deferred-equity")]
            ashare_backfill,
            #[cfg(feature = "deferred-equity")]
            ashare_backfill_cancel,
            #[cfg(feature = "deferred-equity")]
            ashare_acquisition_cancel,
            #[cfg(feature = "deferred-equity")]
            ashare_workspace,
            #[cfg(feature = "deferred-equity")]
            yahoo_instrument_master_acquire,
            #[cfg(feature = "deferred-equity")]
            yahoo_instrument_master_list,
            #[cfg(feature = "deferred-equity")]
            yahoo_universe,
            #[cfg(feature = "deferred-equity")]
            yahoo_calendar_acquire,
            #[cfg(feature = "deferred-equity")]
            yahoo_backfill,
            #[cfg(feature = "deferred-equity")]
            yahoo_backfill_cancel,
            #[cfg(feature = "deferred-equity")]
            yahoo_acquisition_cancel,
            #[cfg(feature = "deferred-equity")]
            yahoo_acquisition_status,
            #[cfg(feature = "deferred-equity")]
            yahoo_snapshot,
            #[cfg(feature = "deferred-equity")]
            yahoo_stream,
            #[cfg(feature = "deferred-equity")]
            yahoo_workspace,
            backtest_preflight,
            backtest_run,
            backtest_list,
            backtest_get,
            backtest_chart_data,
            backtest_execution_data,
            backtest_delete,
            strategy_project_save,
            strategy_project_list,
            strategy_attempt_start,
            strategy_attempt_begin,
            strategy_attempt_complete,
            strategy_attempt_fail,
            strategy_attempt_cancel,
            strategy_attempt_recover,
            portfolio_backtest_run,
            local_research::local_data_summary,
            local_research::local_data_reset,
            local_research::factor_research_device_reset,
            validation_protocol_create,
            validation_protocol_list,
            validation_report_run,
            validation_report_list,
            validation_report_export,
            dataset_generation_start,
            dataset_generation_retry,
            dataset_generation_list,
            dataset_generation_cancel,
            factor_candidate_build,
            factor_candidate_publish,
            factor_candidate_list,
            factor_candidate_get,
            factor_component_prepare,
            factor_component_candidate_get,
            factor_component_qualification_prepare,
            factor_component_qualification_get,
            strategy_candidate::strategy_candidate_catalog,
            strategy_candidate::strategy_candidate_preflight,
            strategy_candidate::strategy_candidate_create,
            strategy_candidate::strategy_candidate_retry,
            strategy_candidate::strategy_candidate_list,
            strategy_candidate::strategy_candidate_get,
            strategy_qualification::strategy_qualification_run,
            strategy_qualification::strategy_qualification_qualify,
            strategy_qualification::strategy_qualification_attempt_list,
            strategy_qualification::strategy_qualification_attempt_get,
            strategy_qualification::strategy_qualification_list,
            strategy_qualification::strategy_qualification_get,
            factor_materialization_start,
            factor_materialization_start_from_context,
            factor_materialization_protocol_freeze,
            factor_evaluation_start,
            factor_evaluation_start_from_context,
            factor_evaluation_protocol_freeze,
            factor_attempt_list,
            factor_attempt_get,
            factor_attempt_cancel,
            factor_attempt_retry,
            factor_component_retry,
            factor_dataset_list,
            factor_dataset_get,
            factor_dataset_rows,
            factor_dataset_delete,
            factor_report_list,
            factor_report_get,
            factor_family_register,
            factor_family_grid_register,
            factor_family_list,
            factor_family_get,
            factor_trial_update,
            factor_lineage_get,
            factor_policy_save,
            factor_policy_list,
            factor_promotion_protocol_freeze,
            factor_decision_record,
            factor_decision_save,
            factor_decision_list,
            factor_decision_library,
            factor_reference_add,
            factor_reference_remove,
            factor_m12_eligibility,
            feature_definition_validate,
            feature_definition_publish,
            feature_definition_list,
            feature_definition_get,
            feature_definition_preview,
            feature_plan_freeze,
            feature_fitting_start,
            feature_fitting_list,
            feature_fitting_get,
            feature_fitting_cancel,
            feature_fitting_retry,
            feature_artifact_list,
            feature_artifact_get,
            feature_artifact_delete,
            feature_materialization_start,
            feature_materialization_list,
            feature_materialization_get,
            feature_materialization_cancel,
            feature_materialization_retry,
            feature_dataset_list,
            feature_dataset_get,
            feature_dataset_summary,
            feature_dataset_rows,
            feature_dataset_delete,
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

    fn bar(
        open_time_ms: i64,
        close: &str,
    ) -> factor_abi::exports::adaq::factor::time_series_api::TimeSeriesRow {
        factor_abi::exports::adaq::factor::time_series_api::TimeSeriesRow {
            instrument_id: "component-run".into(),
            observation_time_ms: open_time_ms,
            slots: vec![
                factor_abi::exports::adaq::factor::time_series_api::FeatureValue {
                    value: close.parse().unwrap(),
                    available_at_ms: open_time_ms,
                },
                factor_abi::exports::adaq::factor::time_series_api::FeatureValue {
                    value: 1.0,
                    available_at_ms: open_time_ms,
                },
            ],
        }
    }

    #[test]
    fn factor_loader_starts_empty() {
        let error = WasmLoader::default().describe_factor().err().unwrap();
        assert_eq!(error, "Factor component is not loaded");
    }

    #[test]
    fn factor_loader_derives_v2_slots_for_schema_only_loading() {
        let loader = WasmLoader::default();
        loader.load(&fixture("factor")).unwrap();
        assert_eq!(
            loader.describe_factor().unwrap().feature_slots,
            ["close", "base-volume"]
        );
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
        whole
            .load_factor_time_series_bytes(
                &std::fs::read(&path).unwrap(),
                vec![
                    factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: "close".into(),
                    },
                    factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: "base-volume".into(),
                    },
                ],
                &[],
            )
            .unwrap();
        assert_eq!(
            whole.describe_factor().unwrap().output_names,
            ["close-change"]
        );
        let one_chunk = whole.process_factor(bars.clone()).unwrap();

        let chunked = WasmLoader::default();
        chunked
            .load_factor_time_series_bytes(
                &std::fs::read(&path).unwrap(),
                vec![
                    factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: "close".into(),
                    },
                    factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: "base-volume".into(),
                    },
                ],
                &[],
            )
            .unwrap();
        let mut two_chunks = chunked.process_factor(bars[..1].to_vec()).unwrap();
        two_chunks.extend(chunked.process_factor(bars[1..].to_vec()).unwrap());

        assert_eq!(one_chunk.len(), two_chunks.len());
        for (whole, chunked) in one_chunk.iter().zip(two_chunks.iter()) {
            assert_eq!(whole.instrument_id, chunked.instrument_id);
            assert_eq!(whole.observation_time_ms, chunked.observation_time_ms);
            match (&whole.values, &chunked.values) {
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
        assert!(one_chunk[0].values.is_none());
        assert_eq!(
            one_chunk[1].values.as_ref().unwrap()[0].name,
            "close-change"
        );
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
