//! The Local Research composition root.
//!
//! Owns only dependency wiring (the shared SQLite connection, the data
//! root layout, and the Source implementations each deep module composes),
//! the Local Data summary and reset orchestration that fans out to the
//! modules' hooks, and the one cross-domain compatibility command that
//! joins Component, Snapshot, and Signal Dataset reads. The deep modules —
//! Component Library, Dataset Generation, Market Data Snapshot, Validation
//! Studies, and Backtest Run — own their domains' schema, SQL, and
//! lifecycle rules; this file issues no SQL against their tables (the
//! not-yet-extracted Watchlist, Signal Dataset, and Forecast Evaluation
//! domains still live here).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use adaq_backtest_core::{
    MarketDataSnapshot, MarketDataUniverseSnapshot, SnapshotDatasetBinding, SnapshotProvenance,
    SnapshotStore, SnapshotUniverseBinding, UniverseSnapshotComponent,
};
use adaq_component_tooling::{
    ComponentKind, ComponentManifest, ComponentPackage, FeatureSlotSource, verify_package,
};
#[cfg(feature = "deferred-equity")]
use adaq_data_core::a_share::AshareClient;
use adaq_data_core::{
    BarGap, BarInterval, OhlcvBar, OkxClient,
    market::{Venue, VenueKind},
    next_bar_open_time_ms,
};
use adaq_data_pipeline::{
    CancellationToken, DataPipeline, DataQualityReport, DataQualityState,
    okx::{OkxSpotDataPath, PointInTimeInstrumentUniverse},
};
#[cfg(feature = "deferred-equity")]
use adaq_data_pipeline::{a_share::AshareDataPath, us_equity::UsEquityDataPath};
use adaq_factor_research::{
    CorporateActionEvidence, EconomicAssumptions, EvaluationWindow, FactorEvaluationProtocol,
    FactorEvaluationProtocolDraft, FactorLens, FactorMarketSeries, FactorOrientation, FactorTarget,
    ResearchEvidenceContext,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{
    backtest::{BacktestSource, Backtests, ComponentPackageSource, SnapshotReadSource},
    component_library::{
        ComponentLibrary, ComponentLockSource, ComponentSource, finish_staged_files, stage_files,
    },
    dataset_generation::{DatasetGeneration, GenerationSource},
    factor_research::{
        FactorAttemptView, FactorCandidatePredecessor, FactorCandidatePublishRequest,
        FactorCandidateView, FactorEvaluationContextStartRequest, FactorEvaluationStartRequest,
        FactorEvidenceRequest, FactorMaterializationContextBinding,
        FactorMaterializationContextStartRequest, FactorMaterializationStartRequest,
        FactorResearch, FactorResearchSource, user_uuid,
    },
    features::{FeatureDatasetRequest, FeatureSource, Features},
    forecast_signal_dataset::{BacktestSignalDataset, backtest_signal_datasets},
    market_data_snapshot::{LocalSnapshotSource, MarketDataSnapshots},
    operations::OperationsStore,
    paper_feedback::PaperFeedbackStore,
    research_queue::ResearchQueue,
    user::validate_user,
    validation::{ValidationRunOutcome, ValidationSource, ValidationStudies},
    watchlist::insert_default_watchlist,
};

pub struct LocalResearchState {
    pub(crate) root: PathBuf,
    pub(crate) database: Arc<Mutex<Connection>>,
    pub(crate) pipeline: DataPipeline,
    pub(crate) okx: OkxSpotDataPath,
    #[cfg(feature = "deferred-equity")]
    pub(crate) ashare: AshareDataPath,
    #[cfg(feature = "deferred-equity")]
    pub(crate) us_equity: UsEquityDataPath,
    pub(crate) snapshots: MarketDataSnapshots,
    pub(crate) components: ComponentLibrary,
    source: Arc<LocalGenerationSource>,
    pub(crate) generation: DatasetGeneration,
    pub(crate) features: Features,
    pub(crate) factor: FactorResearch,
    pub(crate) validation: ValidationStudies,
    pub(crate) backtests: Backtests,
    pub(crate) connections: crate::connections::ConnectionManager,
    pub(crate) operations: OperationsStore,
    pub(crate) paper_feedback: PaperFeedbackStore,
    pub(crate) paper_trading: crate::paper_trading::PaperTradingStore,
    pub(crate) research_contexts: Mutex<HashMap<String, ResearchEvidenceContext>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataSummary {
    data_directory: String,
    database_bytes: u64,
    component_bytes: u64,
    market_data_bytes: u64,
    watchlist_count: u64,
    component_count: u64,
    snapshot_count: u64,
    run_count: u64,
    protocol_count: u64,
    report_count: u64,
    generation_attempt_count: u64,
    model_artifact_count: u64,
    signal_dataset_count: u64,
    component_blocking_run_count: u64,
    market_data_blocking_record_count: u64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalDataResetKind {
    Watchlist,
    Components,
    MarketData,
    All,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "User identity is retained for IPC compatibility and ignored at the Host boundary"
)]
pub struct LocalDataRequest {
    pub user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "User identity is retained for IPC compatibility and ignored at the Host boundary"
)]
pub struct LocalDataResetRequest {
    pub user_id: String,
    pub kind: LocalDataResetKind,
}

/// The concrete local dependencies composed into the Dataset Generation
/// lifecycle module. Only database access, Component Package access
/// through the Component Library module, Market Data Snapshot access,
/// runtime Component materialization, and the Signal Dataset directory
/// are shared; the complete Local Research state is not.
pub(crate) struct LocalGenerationSource {
    database: Arc<Mutex<Connection>>,
    snapshots: MarketDataSnapshots,
    root: PathBuf,
    components: ComponentLibrary,
}

impl SnapshotReadSource for LocalGenerationSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }
}

impl ComponentPackageSource for LocalGenerationSource {
    fn package_for_user(&self, user_id: &str, hash: &str) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, hash)
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        let directory = self.root.join("runtime");
        fs::create_dir_all(&directory).map_err(string)?;
        let path = directory.join(format!("{}.wasm", package.manifest.wasm_sha256));
        if !path.is_file() {
            fs::write(&path, &package.wasm).map_err(string)?;
        }
        Ok(path)
    }
}

impl GenerationSource for LocalGenerationSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn dataset_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("signal-datasets");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }
}

/// The concrete local dependencies composed into the Feature lifecycle
/// module. Only database access, the database file path for the engine's
/// own Materialization connection, the Feature Dataset directory, and
/// User-scoped Snapshot/Universe evidence reads are shared; the complete
/// Local Research state is not.
pub(crate) struct LocalFeatureSource {
    database: Arc<Mutex<Connection>>,
    database_path: PathBuf,
    snapshots: MarketDataSnapshots,
    root: PathBuf,
}

impl SnapshotReadSource for LocalFeatureSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }
}

impl FeatureSource for LocalFeatureSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn database_path(&self) -> Result<PathBuf, String> {
        Ok(self.database_path.clone())
    }

    fn feature_dataset_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("feature-datasets");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }

    fn universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
        self.snapshots
            .universe_snapshot_for_user(user_id, snapshot_id)
    }
}

pub(crate) struct LocalFactorSource {
    database: Arc<Mutex<Connection>>,
    root: PathBuf,
    feature_materialization: adaq_feature_engine::FeatureMaterializationStore,
    snapshots: MarketDataSnapshots,
    components: ComponentLibrary,
}

impl FactorResearchSource for LocalFactorSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn dataset_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("factor-datasets");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }

    fn feature_dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<adaq_factor_research::CompletedFeatureDataset, String> {
        let mut dataset = Features::completed_dataset_from_store(
            &self.feature_materialization,
            user_id,
            dataset_id,
        )?;
        dataset.user_id = user_uuid(user_id).to_string();
        dataset.validate().map_err(|error| error.to_string())?;
        Ok(dataset)
    }

    fn point_in_time_universe(
        &self,
        user_id: &str,
        universe_id: &str,
    ) -> Result<Vec<String>, String> {
        Ok(self
            .snapshots
            .universe_snapshot_for_user(user_id, universe_id)?
            .universe
            .instruments
            .into_iter()
            .map(|instrument| format!("{}:{}", instrument.venue.id, instrument.code))
            .collect())
    }

    fn component_package(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, archive_sha256)
    }

    fn candidate_package(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        let database = self.database.lock().map_err(string)?;
        let bytes: Vec<u8> = database
            .query_row(
                "SELECT package_bytes FROM factor_candidate_packages
                 WHERE user_id = ?1 AND package_sha256 = ?2",
                params![user_id, archive_sha256],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Candidate Package was not found".to_owned())?;
        let package = ComponentPackage::read(&bytes).map_err(string)?;
        verify_package(&package)?;
        if package.archive_sha256 != archive_sha256 {
            return Err("Factor Candidate Package archive hash mismatch".into());
        }
        Ok(package)
    }

    fn reference_feature_dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
        reference_id: &str,
    ) -> Result<(), String> {
        self.feature_materialization
            .reference_dataset(user_id, dataset_id, user_id, reference_id)
            .map_err(|error| error.to_string())
    }

    fn unreference_feature_dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
        reference_id: &str,
    ) -> Result<(), String> {
        self.feature_materialization
            .unreference_dataset(user_id, dataset_id, reference_id)
            .map_err(|error| error.to_string())
    }

    fn validate_materialization_context(
        &self,
        user_id: &str,
        context: &FactorMaterializationContextBinding,
    ) -> Result<(), String> {
        let database = self.database.lock().map_err(string)?;
        let frozen_json = database
            .query_row(
                "SELECT frozen_json FROM research_frozen_evidence
                 WHERE user_id = ?1 AND operation_id = ?2",
                params![user_id, context.operation_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "factor-context-stale".to_owned())?;
        let frozen: adaq_factor_research::FrozenResearchEvidence =
            serde_json::from_str(&frozen_json).map_err(string)?;
        let current_json = database
            .query_row(
                "SELECT context_json FROM research_evidence_contexts
                 WHERE user_id = ?1 AND revision = ?2 AND context_hash = ?3",
                params![
                    user_id,
                    context.context_revision as i64,
                    context.context_hash
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "factor-context-stale".to_owned())?;
        drop(database);
        let current: ResearchEvidenceContext =
            serde_json::from_str(&current_json).map_err(string)?;
        if frozen.operation_id != context.operation_id
            || frozen.stage != adaq_factor_research::ResearchStage::Factors
            || frozen.context_revision != context.context_revision
            || frozen.context_hash != context.context_hash
            || current.revision != context.context_revision
            || current.context_hash != context.context_hash
        {
            return Err("factor-context-stale".into());
        }
        current
            .revalidate_with_policy(
                &current,
                adaq_factor_research::ResearchStage::Factors,
                Default::default(),
            )
            .map_err(|_| "factor-context-stale".to_owned())
    }

    fn record_component_qualification(
        &self,
        user_id: &str,
        attempt: &adaq_component_tooling::QualificationAttempt,
        archive_sha256: &str,
        evidence_json: &str,
    ) -> Result<(), String> {
        self.components
            .record_qualification(user_id, attempt, archive_sha256, evidence_json)
    }

    fn publish_qualified_component_in_transaction(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
        package_bytes: &[u8],
        attempt: &adaq_component_tooling::QualificationAttempt,
        evidence_json: &str,
    ) -> Result<(), String> {
        self.components.publish_qualified_in_transaction(
            transaction,
            user_id,
            package_bytes,
            attempt,
            evidence_json,
        )
    }

    fn component_qualification_for_user(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<Option<crate::component_library::ComponentQualificationRecord>, String> {
        self.components.qualification_for_user(user_id, attempt_id)
    }

    fn component_qualifications_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<crate::component_library::ComponentQualificationRecord>, String> {
        self.components.qualifications_for_user(user_id)
    }
}

/// The concrete local dependencies composed into the Component Library
/// module. Only database access, the archive directory, and the Backtest
/// Run module's component-lock queries are shared; the complete Local
/// Research state is not. The Backtest module is constructed first, so it
/// is bound directly.
pub(crate) struct LocalComponentSource {
    database: Arc<Mutex<Connection>>,
    root: PathBuf,
    backtests: Backtests,
}

impl ComponentLockSource for LocalComponentSource {
    fn runs_locking_components(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
        self.backtests.runs_locking_components(database, user_id)
    }

    fn component_hashes_locked_by_runs(
        &self,
        database: &Connection,
        excluding_user: Option<&str>,
    ) -> Result<std::collections::HashSet<String>, String> {
        self.backtests
            .component_hashes_locked_by_runs(database, excluding_user)
    }
}

impl ComponentSource for LocalComponentSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn archive_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("components");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }

    fn locks(&self) -> &dyn ComponentLockSource {
        self
    }
}

/// The concrete local dependencies composed into the Validation Studies
/// module. Only database access, Component Package access, Market Data
/// Snapshot access and persistence, and Backtest Run execution are shared;
/// the complete Local Research state is not. The state reference is bound
/// after the composition root finishes constructing itself. The database
/// handle is held directly because the module initializes its schema
/// before the self-reference is bound.
pub(crate) struct LocalValidationSource {
    database: Arc<Mutex<Connection>>,
    state: Mutex<Weak<LocalResearchState>>,
}

impl ValidationSource for LocalValidationSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.state()?.package_for_user(user_id, archive_sha256)
    }

    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.state()?.snapshot_for_user(user_id, snapshot_id)
    }

    fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.state()?.persist_snapshot_for_user(user_id, series)
    }

    fn run_backtest(
        &self,
        request: crate::backtest::BacktestRunRequest,
    ) -> Result<ValidationRunOutcome, String> {
        let state = self.state()?;
        let view = state.backtests.run(request)?;
        Ok(ValidationRunOutcome {
            run_id: view.run_id,
            metrics: view.result.metrics,
            pauses: view.pauses,
        })
    }
}

impl LocalValidationSource {
    fn state(&self) -> Result<Arc<LocalResearchState>, String> {
        self.state
            .lock()
            .map_err(string)?
            .upgrade()
            .ok_or_else(|| "Local Research state is not available".to_owned())
    }
}

/// The concrete local dependencies composed into the Backtest Run module.
/// Only database access, Market Data Snapshot reads, Component Package
/// access, Signal Dataset reads through the forecast_signal_dataset-owned path, and the
/// Validation Report reference check are shared; the complete Local
/// Research state is not. The state reference is bound after the
/// composition root finishes constructing itself. The database handle is
/// held directly because the module initializes its schema before the
/// self-reference is bound.
pub(crate) struct LocalBacktestSource {
    database: Arc<Mutex<Connection>>,
    state: Mutex<Weak<LocalResearchState>>,
}

impl LocalBacktestSource {
    fn state(&self) -> Result<Arc<LocalResearchState>, String> {
        self.state
            .lock()
            .map_err(string)?
            .upgrade()
            .ok_or_else(|| "Local Research state is not available".to_owned())
    }
}

impl SnapshotReadSource for LocalBacktestSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.state()?.snapshot_for_user(user_id, snapshot_id)
    }

    fn portfolio_universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<adaq_backtest_core::MarketDataUniverseSnapshot, String> {
        self.state()?
            .snapshots
            .universe_snapshot_for_user(user_id, snapshot_id)
    }
}

impl ComponentPackageSource for LocalBacktestSource {
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.state()?.package_for_user(user_id, archive_sha256)
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        self.state()?.runtime_component(package)
    }
}

impl BacktestSource for LocalBacktestSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn signal_datasets(
        &self,
        user_id: &str,
        include_rows: bool,
        dataset_ids: Option<&[String]>,
    ) -> Result<Vec<BacktestSignalDataset>, String> {
        let state = self.state()?;
        backtest_signal_datasets(&state, user_id, include_rows, dataset_ids)
    }

    fn validation_report_references_run(
        &self,
        user_id: &str,
        run_id: &str,
    ) -> Result<bool, String> {
        self.state()?.validation.references_run(user_id, run_id)
    }
}

impl LocalResearchState {
    pub fn open(app_data: &Path) -> Result<Arc<Self>, String> {
        let root = app_data.join("m3");
        fs::create_dir_all(root.join("components")).map_err(string)?;
        let snapshot_store = SnapshotStore::new(root.join("market-data")).map_err(string)?;
        let database = Connection::open(app_data.join("adaq.db")).map_err(string)?;
        database
            .busy_timeout(Duration::from_secs(5))
            .map_err(string)?;
        // WAL lets the materialization store's separate connection write while the
        // main connection reads. Rollback journal blocks this on Windows ("database is locked").
        database
            .execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(string)?;
        database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS signal_dataset_content (
                dataset_id TEXT PRIMARY KEY,
                metadata_json TEXT NOT NULL,
                parquet_path TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS signal_dataset_access (
                user_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                PRIMARY KEY(user_id, dataset_id),
                FOREIGN KEY(dataset_id) REFERENCES signal_dataset_content(dataset_id)
             );
             CREATE TABLE IF NOT EXISTS forecast_evaluation_content (
                report_id TEXT PRIMARY KEY,
                report_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS forecast_evaluation_access (
                user_id TEXT NOT NULL,
                report_id TEXT NOT NULL,
                PRIMARY KEY(user_id, report_id),
                FOREIGN KEY(report_id) REFERENCES forecast_evaluation_content(report_id)
             );
             CREATE TABLE IF NOT EXISTS research_evidence_contexts (
                user_id TEXT NOT NULL,
                market TEXT NOT NULL,
                revision INTEGER NOT NULL,
                context_hash TEXT NOT NULL,
                context_json TEXT NOT NULL,
                PRIMARY KEY(user_id, market)
             );
             CREATE TABLE IF NOT EXISTS research_frozen_evidence (
                user_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                frozen_json TEXT NOT NULL,
                PRIMARY KEY(user_id, operation_id)
             );
             CREATE TABLE IF NOT EXISTS research_attempt_context_bindings (
                user_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                PRIMARY KEY(user_id, operation_id)
             );
             CREATE TABLE IF NOT EXISTS research_attempt_context_bindings_v2 (
                user_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                PRIMARY KEY(user_id, attempt_id)
             );
             CREATE TABLE IF NOT EXISTS foundation_acquisitions (
                user_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                market TEXT NOT NULL,
                venue TEXT NOT NULL,
                state TEXT NOT NULL,
                revision INTEGER,
                error TEXT,
                request_json TEXT,
                started_at_ms INTEGER NOT NULL,
                finished_at_ms INTEGER,
                PRIMARY KEY(user_id, operation_id)
             );",
            )
            .map_err(string)?;
        let has_request_json = database
            .prepare("SELECT 1 FROM pragma_table_info('foundation_acquisitions') WHERE name = 'request_json'")
            .map_err(string)?
            .exists([])
            .map_err(string)?;
        if !has_request_json {
            database
                .execute(
                    "ALTER TABLE foundation_acquisitions ADD COLUMN request_json TEXT",
                    [],
                )
                .map_err(string)?;
        }
        database
            .execute(
                "UPDATE foundation_acquisitions
                 SET state = 'failed', error = 'operation interrupted by host restart', finished_at_ms = ?1
                 WHERE state = 'running'",
                [now_ms()],
            )
            .map_err(string)?;
        let database = Arc::new(Mutex::new(database));
        let pipeline = DataPipeline::open(root.join("market-data-pipeline"), database.clone())
            .map_err(string)?;
        let okx = OkxSpotDataPath::open(pipeline.clone(), OkxClient::default()).map_err(string)?;
        #[cfg(feature = "deferred-equity")]
        let ashare =
            AshareDataPath::open(pipeline.clone(), AshareClient::default()).map_err(string)?;
        #[cfg(feature = "deferred-equity")]
        let us_equity = UsEquityDataPath::open(pipeline.clone()).map_err(string)?;
        let connections = crate::connections::ConnectionManager::open_production(database.clone())?;
        let operations = OperationsStore::open(database.clone())?;
        let paper_feedback = PaperFeedbackStore::open(database.clone())?;
        let paper_trading = crate::paper_trading::PaperTradingStore::open(database.clone())?;
        let snapshot_source = Arc::new(LocalSnapshotSource::new(
            database.clone(),
            Arc::new(snapshot_store),
        ));
        let snapshots = MarketDataSnapshots::open(snapshot_source)?;
        let backtest_source = Arc::new(LocalBacktestSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let backtests = Backtests::open(backtest_source.clone())?;
        let component_source = Arc::new(LocalComponentSource {
            database: database.clone(),
            root: root.clone(),
            backtests: backtests.clone(),
        });
        let components = ComponentLibrary::open(component_source)?;
        let source = Arc::new(LocalGenerationSource {
            database: database.clone(),
            snapshots: snapshots.clone(),
            root: root.clone(),
            components: components.clone(),
        });
        let generation = DatasetGeneration::open(source.clone())?;
        let feature_source = Arc::new(LocalFeatureSource {
            database: database.clone(),
            database_path: app_data.join("adaq.db"),
            snapshots: snapshots.clone(),
            root: root.clone(),
        });
        let queue = ResearchQueue::open(database.clone())?;
        let features = Features::open(feature_source, queue.clone())?;
        let factor_source = Arc::new(LocalFactorSource {
            database: database.clone(),
            root: root.clone(),
            feature_materialization: features.materialization_store(),
            snapshots: snapshots.clone(),
            components: components.clone(),
        });
        let factor = FactorResearch::open(factor_source, queue.admitter())?;
        features.attach_factor(Arc::new(factor.clone()))?;
        let validation_source = Arc::new(LocalValidationSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let validation = ValidationStudies::open(validation_source.clone())?;
        Ok(Arc::new_cyclic(|weak| {
            *validation_source
                .state
                .lock()
                .expect("validation source state binding is uncontended") = weak.clone();
            *backtest_source
                .state
                .lock()
                .expect("backtest source state binding is uncontended") = weak.clone();
            Self {
                root,
                database,
                pipeline,
                okx,
                #[cfg(feature = "deferred-equity")]
                ashare,
                #[cfg(feature = "deferred-equity")]
                us_equity,
                snapshots,
                components,
                source,
                generation,
                features,
                factor,
                validation,
                backtests,
                connections,
                operations,
                paper_feedback,
                paper_trading,
                research_contexts: Mutex::new(HashMap::new()),
            }
        }))
    }

    pub fn store_research_context(
        &self,
        context: ResearchEvidenceContext,
    ) -> Result<adaq_factor_research::ResearchEvidenceProjection, String> {
        let user_id = context.draft.user_id.clone();
        let projection = context.projection();
        let context_json = serde_json::to_string(&context).map_err(string)?;
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "INSERT INTO research_evidence_contexts (user_id, market, revision, context_hash, context_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(user_id, market) DO UPDATE SET
                   revision = excluded.revision,
                   context_hash = excluded.context_hash,
                   context_json = excluded.context_json",
                params![
                    user_id,
                    projection.market,
                    projection.context_revision as i64,
                    projection.context_hash,
                    context_json,
                ],
            )
            .map_err(string)?;
        self.research_contexts
            .lock()
            .map_err(|_| "research context store lock failed".to_string())?
            .insert(format!("{}:{}", user_id, projection.market), context);
        Ok(projection)
    }

    pub(crate) fn establish_factor_context(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<adaq_factor_research::ResearchEvidenceProjection, String> {
        validate_user(user_id)?;
        if dataset_id.trim().is_empty() {
            return Err("factor-context-feature-dataset-required".into());
        }
        let dataset = self
            .features
            .get_dataset(FeatureDatasetRequest {
                user_id: user_id.to_owned(),
                dataset_id: dataset_id.to_owned(),
            })
            .map_err(|error| factor_context_dataset_error(&error))?;
        self.validate_factor_dataset(user_id, dataset_id)?;
        let request = &dataset.manifest.request;
        if dataset.user_id != user_id || request.user_id != user_id {
            return Err("factor-context-user-mismatch".into());
        }
        let (snapshot, _) = self
            .snapshots
            .snapshot_for_user(user_id, &request.snapshot_id)
            .map_err(|_| "factor-context-snapshot-inaccessible".to_owned())?;
        let universe = self
            .snapshots
            .universe_snapshot_for_user(user_id, &request.point_in_time_universe_id)
            .map_err(|_| "factor-context-universe-inaccessible".to_owned())?;
        let (market, venue) = factor_context_market_venue(&snapshot)?;
        if universe.venue.id != venue
            || snapshot
                .provenance
                .as_ref()
                .is_some_and(|provenance| provenance.venue != universe.venue)
            || !universe
                .components
                .iter()
                .any(|component| component.snapshot_id == snapshot.snapshot_id)
        {
            return Err("factor-context-market-venue-mismatch".into());
        }
        if universe.interval != snapshot.interval {
            return Err("factor-context-interval-mismatch".into());
        }
        let range = &request.observation_range;
        Self::validate_factor_context_range(
            range.start_time_ms,
            range.end_time_ms,
            &snapshot,
            &universe,
        )?;
        if universe.universe.evidence_state == "unknown" {
            return Err("factor-context-universe-incomplete".into());
        }
        let feature_dataset = adaq_factor_research::FeatureDatasetBinding {
            dataset_id: dataset.dataset_id.clone(),
            request_hash: dataset.request_hash.clone(),
            feature_plan_hash: request.feature_plan_hash.clone(),
            content_sha256: dataset.manifest.content_sha256.clone(),
            output_names: dataset
                .manifest
                .outputs
                .iter()
                .map(|output| output.output_name.clone())
                .collect(),
        };
        let draft = adaq_factor_research::ResearchEvidenceContextDraft {
            user_id: user_id.to_owned(),
            market: market.clone(),
            venue: venue.clone(),
            range_start_ms: range.start_time_ms,
            range_end_ms: range.end_time_ms,
            snapshot_id: request.snapshot_id.clone(),
            universe_id: Some(request.point_in_time_universe_id.clone()),
            evidence: vec![adaq_factor_research::EvidenceBinding {
                id: dataset.dataset_id.clone(),
                lineage_hash: dataset.dataset_id.clone(),
                user_id: user_id.to_owned(),
                market: market.clone(),
                venue: venue.clone(),
                snapshot_id: request.snapshot_id.clone(),
                universe_id: Some(request.point_in_time_universe_id.clone()),
                feature_id: Some(dataset.dataset_id.clone()),
                factor_id: None,
                model_id: None,
                grade: adaq_factor_research::EvidenceGrade::ProviderGraded,
                accessible: true,
                complete: true,
                fresh: true,
            }],
            feature_dataset: Some(feature_dataset.clone()),
        };
        let context = match self.context_for_user(user_id)? {
            Some(current) if current.draft == draft => {
                current
                    .revalidate_with_policy(
                        &current,
                        adaq_factor_research::ResearchStage::Factors,
                        Default::default(),
                    )
                    .map_err(|error| error.to_string())?;
                current
            }
            Some(current) => current
                .revise_for_stage(
                    draft,
                    adaq_factor_research::ResearchStage::Factors,
                    Default::default(),
                )
                .map_err(|error| error.to_string())?,
            None => adaq_factor_research::ResearchEvidenceContext::establish_for_stage(
                draft,
                adaq_factor_research::ResearchStage::Factors,
                Default::default(),
            )
            .map_err(|error| error.to_string())?,
        };
        self.store_research_context(context)
    }

    pub(crate) fn publish_factor_candidate(
        &self,
        request: FactorCandidatePublishRequest,
    ) -> Result<FactorCandidateView, String> {
        let user_id = request.user_id.clone();
        let context = self
            .context_for_user(&user_id)?
            .ok_or_else(|| "factor-context-required".to_owned())?;
        let dataset_id = context
            .draft
            .feature_dataset
            .as_ref()
            .ok_or_else(|| "factor-context-feature-dataset-required".to_owned())?
            .dataset_id
            .clone();
        let projection = self.establish_factor_context(&user_id, &dataset_id)?;
        let predecessor = FactorCandidatePredecessor::from_projection(user_id, projection)?;
        self.factor
            .publish_candidate_with_predecessor(request, predecessor)
    }

    pub(crate) fn start_factor_materialization_from_context(
        &self,
        request: FactorMaterializationContextStartRequest,
    ) -> Result<FactorAttemptView, String> {
        validate_user(&request.user_id)?;
        if request.operation_id.trim().is_empty() || request.candidate_hash.trim().is_empty() {
            return Err("factor-context-required".into());
        }
        let current_context = self
            .context_for_user(&request.user_id)?
            .ok_or_else(|| "factor-context-required".to_owned())?;
        let dataset_id = current_context
            .draft
            .feature_dataset
            .as_ref()
            .ok_or_else(|| "factor-context-feature-dataset-required".to_owned())?
            .dataset_id
            .clone();
        let projection = self.establish_factor_context(&request.user_id, &dataset_id)?;
        let candidate = self.factor.get_candidate(FactorEvidenceRequest {
            user_id: request.user_id.clone(),
            evidence_id: request.candidate_hash.clone(),
        })?;
        let expected_predecessor = FactorCandidatePredecessor::from_projection(
            request.user_id.clone(),
            projection.clone(),
        )?;
        if candidate.predecessor.as_ref() != Some(&expected_predecessor) {
            return Err("factor-context-mismatch".into());
        }
        let feature_dataset = self.features.get_dataset(FeatureDatasetRequest {
            user_id: request.user_id.clone(),
            dataset_id: dataset_id.clone(),
        })?;
        let (snapshot, _) = self
            .snapshots
            .snapshot_for_user(&request.user_id, &projection.snapshot_id)?;
        let universe_id = projection
            .universe_id
            .clone()
            .ok_or_else(|| "factor-context-universe-inaccessible".to_owned())?;
        let valuation_currency = factor_valuation_currency(
            &projection.market,
            &snapshot.code,
            &feature_dataset.manifest.request.valuation_currency,
        )?;
        let candidate_hash = request.candidate_hash.clone();
        let protocol = adaq_factor_research::FactorMaterializationProtocol::freeze(
            adaq_factor_research::FactorMaterializationProtocolDraft {
                protocol_id: user_uuid(&format!(
                    "factor-materialization:{}:{}:{}",
                    request.candidate_hash, projection.context_hash, request.seed
                )),
                user_id: user_uuid(&request.user_id),
                candidate_hash,
                feature_dataset_id: dataset_id,
                feature_plan_hash: projection
                    .feature_dataset
                    .as_ref()
                    .ok_or("factor-context-feature-dataset-required")?
                    .feature_plan_hash
                    .clone(),
                parameters: factor_parameter_defaults(&candidate.candidate.parameters)?,
                market_data_snapshot_id: projection.snapshot_id.clone(),
                point_in_time_universe_id: universe_id.clone(),
                observation_range: adaq_factor_research::ObservationRange {
                    start_time_ms: projection.range_start_ms,
                    end_time_ms: projection.range_end_ms,
                },
                market_context: adaq_factor_research::FactorMarketContext {
                    venue: projection.venue.clone(),
                    asset_class: projection.market.clone(),
                    bar_interval: snapshot.interval.as_str().into(),
                    price_basis: "unadjusted".into(),
                    valuation_currency,
                    point_in_time_universe_id: universe_id,
                },
                engine_identity: factor_native_engine_identity(
                    &candidate.candidate,
                    &request.candidate_hash,
                    &projection,
                ),
                seed: request.seed,
            },
        )
        .map_err(string)?;
        let frozen = self.freeze_research_context(
            &request.user_id,
            request.operation_id.clone(),
            adaq_factor_research::ResearchStage::Factors,
        )?;
        let attempt = self
            .factor
            .start_materialization(FactorMaterializationStartRequest {
                user_id: request.user_id.clone(),
                protocol,
                dataset: None,
                context: Some(FactorMaterializationContextBinding {
                    operation_id: request.operation_id.clone(),
                    context_revision: frozen.context_revision,
                    context_hash: frozen.context_hash,
                }),
            })?;
        self.record_research_attempt_binding(
            &request.user_id,
            &request.operation_id,
            adaq_factor_research::ResearchStage::Factors,
            &attempt.attempt_id,
        )?;
        Ok(attempt)
    }

    pub(crate) fn start_factor_evaluation_from_context(
        &self,
        request: FactorEvaluationContextStartRequest,
    ) -> Result<FactorAttemptView, String> {
        validate_user(&request.user_id)?;
        if request.operation_id.trim().is_empty()
            || request.candidate_hash.trim().is_empty()
            || request.dataset_id.trim().is_empty()
            || request.output_name.trim().is_empty()
        {
            return Err("factor-context-required".into());
        }
        let current_context = self
            .context_for_user(&request.user_id)?
            .ok_or_else(|| "factor-context-required".to_owned())?;
        let context_feature_dataset = current_context
            .draft
            .feature_dataset
            .as_ref()
            .ok_or_else(|| "factor-context-feature-dataset-required".to_owned())?;
        let dataset = self.factor.get_dataset(FactorEvidenceRequest {
            user_id: request.user_id.clone(),
            evidence_id: request.dataset_id.clone(),
        })?;
        let manifest = dataset.manifest.clone();
        if manifest.feature_dataset_id != context_feature_dataset.dataset_id {
            return Err("factor-context-mismatch".into());
        }
        let projection =
            self.establish_factor_context(&request.user_id, &context_feature_dataset.dataset_id)?;
        let candidate = self.factor.get_candidate(FactorEvidenceRequest {
            user_id: request.user_id.clone(),
            evidence_id: request.candidate_hash.clone(),
        })?;
        let expected_predecessor = FactorCandidatePredecessor::from_projection(
            request.user_id.clone(),
            projection.clone(),
        )?;
        if candidate.predecessor.as_ref() != Some(&expected_predecessor) {
            return Err(if candidate.predecessor.is_none() {
                "factor-context-candidate-predecessor-missing".into()
            } else {
                "factor-context-mismatch".into()
            });
        }
        if manifest.candidate_hash != request.candidate_hash
            || manifest.scope != candidate.candidate.scope
            || !candidate
                .candidate
                .outputs
                .iter()
                .any(|output| output.name == request.output_name)
            || !manifest
                .output_names
                .iter()
                .any(|output| output == &request.output_name)
            || manifest.feature_plan_hash != context_feature_dataset.feature_plan_hash
            || manifest.market_data_snapshot_id != projection.snapshot_id
            || manifest.point_in_time_universe_id
                != projection.universe_id.clone().unwrap_or_default()
            || manifest.market_context.asset_class != projection.market
            || manifest.market_context.venue != projection.venue
        {
            return Err("factor-context-mismatch".into());
        }
        let range = manifest
            .observation_range
            .clone()
            .ok_or_else(|| "factor-context-range-mismatch".to_owned())?;
        if range.start_time_ms != projection.range_start_ms
            || range.end_time_ms != projection.range_end_ms
            || manifest.market_context.point_in_time_universe_id
                != projection.universe_id.clone().unwrap_or_default()
        {
            return Err("factor-context-range-mismatch".into());
        }
        let (snapshot, _) = self
            .snapshots
            .snapshot_for_user(&request.user_id, &projection.snapshot_id)
            .map_err(|_| "factor-context-snapshot-inaccessible".to_owned())?;
        let universe_id = projection
            .universe_id
            .clone()
            .ok_or_else(|| "factor-context-universe-inaccessible".to_owned())?;
        let universe = self
            .snapshots
            .universe_snapshot_for_user(&request.user_id, &universe_id)
            .map_err(|_| "factor-context-universe-inaccessible".to_owned())?;
        let mut point_in_time_universe = universe
            .universe
            .instruments
            .iter()
            .map(|instrument| format!("{}:{}", instrument.venue.id, instrument.code))
            .collect::<Vec<_>>();
        point_in_time_universe.sort_unstable();
        point_in_time_universe.dedup();
        if point_in_time_universe.is_empty() {
            return Err("factor-context-universe-incomplete".into());
        }
        if manifest.market_context.bar_interval != snapshot.interval.as_str() {
            return Err("factor-context-interval-mismatch".into());
        }
        let midpoint = range
            .end_time_ms
            .checked_sub(range.start_time_ms)
            .and_then(|width| range.start_time_ms.checked_add(width / 2))
            .ok_or_else(|| "factor-context-range-mismatch".to_owned())?;
        if midpoint <= range.start_time_ms || midpoint >= range.end_time_ms {
            return Err("factor-context-range-mismatch".into());
        }
        let selection = adaq_factor_research::ObservationRange {
            start_time_ms: range.start_time_ms,
            end_time_ms: midpoint,
        };
        let evaluation = adaq_factor_research::ObservationRange {
            start_time_ms: midpoint,
            end_time_ms: range.end_time_ms,
        };
        let family_id = user_uuid(&format!(
            "factor-evaluation-family:{}:{}:{}:{}:{}",
            request.user_id,
            request.candidate_hash,
            request.dataset_id,
            request.output_name,
            request.seed
        ));
        let trial_id = user_uuid(&format!(
            "factor-evaluation-trial:{}:{}:{}:{}:{}",
            request.user_id,
            request.candidate_hash,
            request.dataset_id,
            request.output_name,
            request.seed
        ));
        let protocol = FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
            protocol_id: user_uuid(&format!(
                "factor-evaluation-protocol:{}:{}:{}:{}:{}",
                request.user_id,
                request.candidate_hash,
                request.dataset_id,
                request.output_name,
                request.seed
            )),
            user_id: user_uuid(&request.user_id),
            factor_dataset_id: request.dataset_id.clone(),
            feature_dataset_id: manifest.feature_dataset_id.clone(),
            feature_plan_hash: manifest.feature_plan_hash.clone(),
            market_data_snapshot_id: manifest.market_data_snapshot_id.clone(),
            point_in_time_universe_id: universe_id.clone(),
            point_in_time_universe,
            output_name: request.output_name,
            scope: candidate.candidate.scope,
            target: FactorTarget::FutureCloseReturn,
            horizon_bars: vec![1],
            market_context: manifest.market_context.clone(),
            engine_identity: manifest.engine_identity.clone(),
            orientation: FactorOrientation::Positive,
            windows: vec![EvaluationWindow {
                fold_id: "host-selection-evaluation-1".into(),
                selection: selection.clone(),
                evaluation,
                training: Some(selection.clone()),
                fitting: Some(selection.clone()),
                normalization: Some(selection.clone()),
                target_construction: Some(selection),
            }],
            purge_bars: 0,
            embargo_bars: 0,
            lenses: FactorLens::required(candidate.candidate.scope).to_vec(),
            nuisance_feature_names: Vec::new(),
            regime: None,
            economic: EconomicAssumptions {
                rebalance_every_bars: 1,
                fee_bps: 0.0,
                slippage_bps: 0.0,
                long_short: true,
            },
            family_id,
            trial_id,
            seed: request.seed,
        })
        .map_err(string)?;
        let market_series =
            self.factor_market_series_for_context(&request.user_id, &protocol, &universe)?;
        let frozen = self.freeze_research_context(
            &request.user_id,
            request.operation_id.clone(),
            adaq_factor_research::ResearchStage::Factors,
        )?;
        let attempt = self
            .factor
            .start_evaluation_host_owned(FactorEvaluationStartRequest {
                user_id: request.user_id.clone(),
                protocol,
                dataset: None,
                market_series,
                feature_evidence: None,
            })?;
        self.record_research_attempt_binding(
            &request.user_id,
            &request.operation_id,
            adaq_factor_research::ResearchStage::Factors,
            &attempt.attempt_id,
        )?;
        let _ = frozen;
        Ok(attempt)
    }

    pub(crate) fn validate_factor_evaluation_inputs_from_host(
        &self,
        request: &FactorEvaluationStartRequest,
    ) -> Result<(), String> {
        if request.feature_evidence.is_some() {
            return Err("factor-context-feature-evidence-unavailable".into());
        }
        let universe = self
            .snapshots
            .universe_snapshot_for_user(
                &request.user_id,
                &request.protocol.point_in_time_universe_id,
            )
            .map_err(|_| "factor-context-universe-inaccessible".to_owned())?;
        let expected =
            self.factor_market_series_for_context(&request.user_id, &request.protocol, &universe)?;
        if expected != request.market_series {
            return Err("factor-context-market-evidence-mismatch".into());
        }
        Ok(())
    }

    fn factor_market_series_for_context(
        &self,
        user_id: &str,
        protocol: &FactorEvaluationProtocol,
        universe: &MarketDataUniverseSnapshot,
    ) -> Result<Vec<FactorMarketSeries>, String> {
        if universe.universe.evidence_state == "unknown" {
            return Err("factor-context-universe-incomplete".into());
        }
        universe
            .components
            .iter()
            .map(|component| {
                let instrument = &component.dataset.instrument;
                if instrument.venue != universe.venue
                    || !universe.universe.instruments.contains(instrument)
                {
                    return Err("factor-context-universe-incomplete".into());
                }
                let (snapshot, bars) = self
                    .snapshots
                    .snapshot_for_user(user_id, &component.snapshot_id)
                    .map_err(|_| "factor-context-snapshot-inaccessible".to_owned())?;
                if snapshot.src != instrument.venue.id
                    || snapshot.code != instrument.code
                    || snapshot.interval != universe.interval
                {
                    return Err("factor-context-market-venue-mismatch".into());
                }
                let bars = bars
                    .into_iter()
                    .map(|mut bar| {
                        bar.open_time_ms =
                            next_bar_open_time_ms(bar.open_time_ms, universe.interval)
                                .map_err(string)?;
                        Ok(bar)
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if bars.is_empty() {
                    return Err("factor-context-market-venue-unavailable".into());
                }
                let gaps = snapshot
                    .gaps
                    .into_iter()
                    .map(|gap| BarGap {
                        start_time_ms: gap.start_time_ms,
                        end_time_ms: gap.end_time_ms,
                    })
                    .collect();
                let corporate_action_evidence = match universe.venue.kind {
                    VenueKind::CryptoSpot => CorporateActionEvidence::Verified,
                    VenueKind::ChinaAShareEquity | VenueKind::UsEquity => {
                        CorporateActionEvidence::Unavailable {
                            reason: "Host corporate-action evidence is not available".into(),
                        }
                    }
                };
                Ok(FactorMarketSeries {
                    instrument_id: format!("{}:{}", instrument.venue.id, instrument.code),
                    snapshot_id: protocol.market_data_snapshot_id.clone(),
                    market_context: protocol.market_context.clone(),
                    bars,
                    gaps,
                    corporate_action_evidence,
                })
            })
            .collect()
    }

    fn validate_factor_dataset(&self, user_id: &str, dataset_id: &str) -> Result<(), String> {
        let store = self.features.materialization_store();
        Features::completed_dataset_from_store(&store, user_id, dataset_id)
            .map(|_| ())
            .map_err(|error| factor_context_dataset_error(&error))
    }

    pub(crate) fn require_factor_context_for_request(
        &self,
        user_id: &str,
        operation_id: &str,
        feature_dataset_id: &str,
        feature_plan_hash: &str,
        snapshot_id: &str,
        universe_id: &str,
        request_range: Option<(i64, i64)>,
        require_exact_range: bool,
        market: &str,
        venue: &str,
        market_context_universe_id: &str,
    ) -> Result<adaq_factor_research::FrozenResearchEvidence, String> {
        let frozen = self.require_frozen_research_evidence(
            user_id,
            operation_id,
            adaq_factor_research::ResearchStage::Factors,
        )?;
        let binding = frozen
            .feature_dataset
            .as_ref()
            .ok_or("factor-context-feature-dataset-required")?;
        let context = self
            .context_for_user(user_id)?
            .ok_or("Research Evidence Context is not established")?;
        let range_matches = request_range.is_none_or(|(start, end)| {
            if require_exact_range {
                context.draft.range_start_ms == start && context.draft.range_end_ms == end
            } else {
                context.draft.range_start_ms <= start && context.draft.range_end_ms >= end
            }
        });
        if binding.dataset_id != feature_dataset_id
            || binding.feature_plan_hash != feature_plan_hash
            || frozen.snapshot_id != snapshot_id
            || frozen.universe_id.as_deref() != Some(universe_id)
            || context.draft.market != market
            || context.draft.venue != venue
            || context.draft.universe_id.as_deref() != Some(market_context_universe_id)
            || !range_matches
        {
            return Err("factor-context-mismatch".into());
        }
        self.validate_factor_dataset(user_id, feature_dataset_id)?;
        Ok(frozen)
    }

    fn validate_factor_context_range(
        start_time_ms: i64,
        end_time_ms: i64,
        snapshot: &MarketDataSnapshot,
        universe: &MarketDataUniverseSnapshot,
    ) -> Result<(), String> {
        let snapshot_coverage_end =
            adaq_data_core::next_bar_open_time_ms(snapshot.end_time_ms, snapshot.interval)
                .map_err(|_| "factor-context-range-mismatch".to_owned())?;
        let universe_coverage_start = universe
            .universe
            .coverage_start_ms
            .map_or(universe.start_time_ms, |start| {
                universe.start_time_ms.max(start)
            });
        let universe_coverage_end_open = universe
            .universe
            .coverage_end_ms
            .map_or(universe.end_time_ms, |end| universe.end_time_ms.min(end));
        let universe_coverage_end =
            adaq_data_core::next_bar_open_time_ms(universe_coverage_end_open, universe.interval)
                .map_err(|_| "factor-context-range-mismatch".to_owned())?;
        if start_time_ms >= end_time_ms
            || start_time_ms < snapshot.start_time_ms
            || end_time_ms > snapshot_coverage_end
            || start_time_ms < universe_coverage_start
            || end_time_ms > universe_coverage_end
        {
            return Err("factor-context-range-mismatch".into());
        }
        Ok(())
    }

    fn context_for_user(&self, user_id: &str) -> Result<Option<ResearchEvidenceContext>, String> {
        validate_user(user_id)?;
        if let Some(context) = self
            .research_contexts
            .lock()
            .map_err(|_| "research context store lock failed".to_string())?
            .values()
            .find(|context| context.draft.user_id == user_id)
            .cloned()
        {
            return Ok(Some(context));
        }
        let database = self
            .database
            .lock()
            .map_err(|_| "database lock failed".to_string())?;
        let mut statement = database
            .prepare("SELECT context_json FROM research_evidence_contexts WHERE user_id = ?1 ORDER BY market LIMIT 1")
            .map_err(string)?;
        let context = statement
            .query_row([user_id], |row| row.get::<_, String>(0))
            .optional()
            .map_err(string)?
            .map(|json| serde_json::from_str::<ResearchEvidenceContext>(&json).map_err(string))
            .transpose()?;
        drop(statement);
        drop(database);
        if let Some(context) = context {
            let market = context.draft.market.clone();
            self.research_contexts
                .lock()
                .map_err(|_| "research context store lock failed".to_string())?
                .insert(format!("{}:{}", user_id, market), context.clone());
            return Ok(Some(context));
        }
        Ok(None)
    }

    pub fn research_context_for_user(
        &self,
        user_id: &str,
    ) -> Result<Option<adaq_factor_research::ResearchEvidenceProjection>, String> {
        let context = self.context_for_user(user_id)?;
        if let Some(context) = context {
            if let Some(binding) = context.draft.feature_dataset.as_ref() {
                self.validate_factor_dataset(user_id, &binding.dataset_id)?;
            }
            return Ok(Some(context.projection()));
        }
        Ok(None)
    }

    pub fn freeze_research_context(
        &self,
        user_id: &str,
        operation_id: String,
        stage: adaq_factor_research::ResearchStage,
    ) -> Result<adaq_factor_research::FrozenResearchEvidence, String> {
        let context = self.context_for_user(user_id)?.ok_or_else(|| {
            if stage == adaq_factor_research::ResearchStage::Factors {
                "factor-context-required".to_owned()
            } else {
                "Research Evidence Context is not established".to_owned()
            }
        })?;
        if stage == adaq_factor_research::ResearchStage::Factors {
            let dataset_id = context
                .draft
                .feature_dataset
                .as_ref()
                .ok_or("factor-context-feature-dataset-required")?
                .dataset_id
                .clone();
            self.validate_factor_dataset(user_id, &dataset_id)?;
        }
        let frozen = context
            .freeze(operation_id, stage)
            .map_err(|error| error.to_string())?;
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "INSERT OR REPLACE INTO research_frozen_evidence (user_id, operation_id, frozen_json) VALUES (?1, ?2, ?3)",
                params![
                    user_id,
                    frozen.operation_id,
                    serde_json::to_string(&frozen).map_err(string)?,
                ],
            )
            .map_err(string)?;
        Ok(frozen)
    }

    pub fn frozen_research_evidence(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<Option<adaq_factor_research::FrozenResearchEvidence>, String> {
        validate_user(user_id)?;
        let database = self
            .database
            .lock()
            .map_err(|_| "database lock failed".to_string())?;
        database
            .query_row(
                "SELECT frozen_json FROM research_frozen_evidence WHERE user_id = ?1 AND operation_id = ?2",
                params![user_id, operation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(string)?
            .map(|json| serde_json::from_str(&json).map_err(string))
            .transpose()
    }

    pub fn require_frozen_research_evidence(
        &self,
        user_id: &str,
        operation_id: &str,
        stage: adaq_factor_research::ResearchStage,
    ) -> Result<adaq_factor_research::FrozenResearchEvidence, String> {
        let frozen = self
            .frozen_research_evidence(user_id, operation_id)?
            .ok_or_else(|| {
                "Research Evidence Context is not frozen for this operation".to_string()
            })?;
        if frozen.stage != stage {
            return Err(
                "Research Evidence Context stage is incompatible with this operation".into(),
            );
        }
        let current = self.context_for_user(user_id)?.ok_or_else(|| {
            if stage == adaq_factor_research::ResearchStage::Factors {
                "factor-context-required".to_owned()
            } else {
                "Research Evidence Context is not established".to_owned()
            }
        })?;
        current
            .revalidate(&current, stage)
            .map_err(|error| error.to_string())?;
        if current.revision != frozen.context_revision
            || current.context_hash != frozen.context_hash
            || current.draft.snapshot_id != frozen.snapshot_id
            || current.draft.universe_id != frozen.universe_id
        {
            return Err(if stage == adaq_factor_research::ResearchStage::Factors {
                "factor-context-stale".into()
            } else {
                "Research Evidence Context is stale for this operation".into()
            });
        }
        if stage == adaq_factor_research::ResearchStage::Factors {
            let dataset_id = frozen
                .feature_dataset
                .as_ref()
                .ok_or("factor-context-feature-dataset-required")?
                .dataset_id
                .clone();
            self.validate_factor_dataset(user_id, &dataset_id)?;
        }
        Ok(frozen)
    }

    pub fn record_research_attempt_binding(
        &self,
        user_id: &str,
        operation_id: &str,
        stage: adaq_factor_research::ResearchStage,
        attempt_id: &str,
    ) -> Result<(), String> {
        self.require_frozen_research_evidence(user_id, operation_id, stage)?;
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "INSERT OR REPLACE INTO research_attempt_context_bindings_v2 (user_id, operation_id, stage, attempt_id) VALUES (?1, ?2, ?3, ?4)",
                params![user_id, operation_id, format!("{stage:?}"), attempt_id],
            )
            .map_err(string)?;
        Ok(())
    }

    pub fn research_attempt_binding(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<Option<(String, adaq_factor_research::ResearchStage)>, String> {
        validate_user(user_id)?;
        let database = self
            .database
            .lock()
            .map_err(|_| "database lock failed".to_string())?;
        let binding = database
            .query_row(
                "SELECT operation_id, stage FROM research_attempt_context_bindings_v2 WHERE user_id = ?1 AND attempt_id = ?2",
                params![user_id, attempt_id],
                |row| {
                    let operation_id: String = row.get(0)?;
                    let stage: String = row.get(1)?;
                    Ok((operation_id, stage))
                },
            )
            .optional()
            .map_err(string)?;
        let binding = match binding {
            Some(binding) => Some(binding),
            None => database
                .query_row(
                    "SELECT operation_id, stage FROM research_attempt_context_bindings WHERE user_id = ?1 AND attempt_id = ?2",
                    params![user_id, attempt_id],
                    |row| {
                        let operation_id: String = row.get(0)?;
                        let stage: String = row.get(1)?;
                        Ok((operation_id, stage))
                    },
                )
                .optional()
                .map_err(string)?,
        };
        binding
            .map(|(operation_id, stage)| {
                let stage = match stage.as_str() {
                    "Features" => adaq_factor_research::ResearchStage::Features,
                    "Factors" => adaq_factor_research::ResearchStage::Factors,
                    "Models" => adaq_factor_research::ResearchStage::Models,
                    _ => return Err("stored research attempt stage is invalid".to_string()),
                };
                Ok((operation_id, stage))
            })
            .transpose()
    }

    pub fn research_context_for_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<Option<adaq_factor_research::FrozenResearchEvidence>, String> {
        let Some((operation_id, _stage)) = self.research_attempt_binding(user_id, attempt_id)?
        else {
            return Ok(None);
        };
        self.frozen_research_evidence(user_id, &operation_id)
    }

    pub fn foundation_acquisition_start(
        &self,
        user_id: &str,
        operation_id: &str,
        market: &str,
        venue: &str,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        if operation_id.trim().is_empty() {
            return Err("foundation acquisition operation ID must be non-empty".into());
        }
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "INSERT OR REPLACE INTO foundation_acquisitions
                 (user_id, operation_id, market, venue, state, revision, error, started_at_ms, finished_at_ms)
                 VALUES (?1, ?2, ?3, ?4, 'running', NULL, NULL, ?5, NULL)",
                params![user_id, operation_id, market, venue, now_ms()],
            )
            .map_err(string)?;
        Ok(())
    }

    pub fn foundation_acquisition_finish(
        &self,
        user_id: &str,
        operation_id: &str,
        state: &str,
        revision: Option<u64>,
        error: Option<&str>,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "UPDATE foundation_acquisitions
                 SET state = ?1, revision = ?2, error = ?3, finished_at_ms = ?4
                 WHERE user_id = ?5 AND operation_id = ?6",
                params![
                    state,
                    revision.map(|value| value as i64),
                    error,
                    now_ms(),
                    user_id,
                    operation_id
                ],
            )
            .map_err(string)?;
        Ok(())
    }

    pub fn foundation_okx_backfill_start(
        &self,
        request: &adaq_data_pipeline::okx::OkxBackfillRequest,
    ) -> Result<(), String> {
        self.foundation_acquisition_start(&request.user_id, &request.task_id, "crypto", "okx")?;
        self.database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .execute(
                "UPDATE foundation_acquisitions SET request_json = ?1
                 WHERE user_id = ?2 AND operation_id = ?3",
                params![
                    serde_json::to_string(request).map_err(string)?,
                    request.user_id,
                    request.task_id
                ],
            )
            .map_err(string)?;
        Ok(())
    }

    pub fn foundation_okx_backfill_request(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<adaq_data_pipeline::okx::OkxBackfillRequest, String> {
        validate_user(user_id)?;
        let (state, request_json) = self
            .database
            .lock()
            .map_err(|_| "database lock failed".to_string())?
            .query_row(
                "SELECT state, request_json FROM foundation_acquisitions
                 WHERE user_id = ?1 AND operation_id = ?2 AND market = 'crypto' AND venue = 'okx'",
                params![user_id, operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(string)?
            .ok_or_else(|| "retained OKX backfill request was not found".to_string())?;
        if !matches!(state.as_str(), "failed" | "cancelled") {
            return Err("only failed or cancelled OKX backfills can be retried".into());
        }
        let request_json = request_json
            .ok_or_else(|| "retained OKX backfill request was not found".to_string())?;
        serde_json::from_str(&request_json).map_err(string)
    }

    pub fn foundation_acquisition_history(
        &self,
        user_id: &str,
    ) -> Result<Vec<crate::market_data_pipeline::FoundationAcquisitionView>, String> {
        validate_user(user_id)?;
        let database = self
            .database
            .lock()
            .map_err(|_| "database lock failed".to_string())?;
        let mut statement = database
            .prepare(
                "SELECT operation_id, market, venue, state, revision, error, started_at_ms, finished_at_ms
                 FROM foundation_acquisitions WHERE user_id = ?1 ORDER BY started_at_ms DESC",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok(crate::market_data_pipeline::FoundationAcquisitionView {
                    operation_id: row.get(0)?,
                    market: row.get(1)?,
                    venue: row.get(2)?,
                    state: row.get(3)?,
                    revision: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                    error: row.get(5)?,
                    started_at_ms: row.get(6)?,
                    finished_at_ms: row.get(7)?,
                })
            })
            .map_err(string)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(string)
    }

    pub fn local_data_summary(&self, user_id: &str) -> Result<LocalDataSummary, String> {
        validate_user(user_id)?;
        let generation_attempt_count = self.generation.list(user_id)?.len() as u64;
        let validation = self.validation.summary_for_user(user_id)?;
        // Query the Snapshot, Backtest, and Component modules before
        // locking the database mutex so the hooks never re-enter a held
        // lock.
        let snapshots = self.snapshots.summary_for_user(user_id)?;
        let pipeline_market_data_bytes = self
            .pipeline
            .storage_footprint_for_user(user_id)
            .map_err(string)?;
        let backtests = self.backtests.summary_for_user(user_id)?;
        let components = self.components.summary_for_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let count = |sql: &str| -> Result<u64, String> {
            database
                .query_row(sql, [user_id], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(string)
        };
        let database_path = self.root.parent().unwrap_or(&self.root).join("adaq.db");
        let data_directory = database_path
            .parent()
            .unwrap_or(&self.root)
            .to_string_lossy()
            .into_owned();

        Ok(LocalDataSummary {
            data_directory,
            database_bytes: file_bytes(&database_path),
            component_bytes: components.component_bytes,
            market_data_bytes: snapshots
                .market_data_bytes
                .saturating_add(pipeline_market_data_bytes),
            watchlist_count: count("SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1")?,
            component_count: components.component_count,
            snapshot_count: snapshots.snapshot_count,
            run_count: backtests.run_count,
            protocol_count: validation.protocol_count,
            report_count: validation.report_count,
            generation_attempt_count,
            model_artifact_count: components.model_artifact_count,
            signal_dataset_count: count(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            )?,
            component_blocking_run_count: components.component_blocking_run_count,
            market_data_blocking_record_count: backtests
                .run_count
                .saturating_add(count(
                    "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
                )?)
                .saturating_add(validation.report_count),
        })
    }

    #[cfg(feature = "deferred-equity")]
    pub fn reset_local_data(&self, user_id: &str, kind: LocalDataResetKind) -> Result<(), String> {
        validate_user(user_id)?;
        let _reset_block = if matches!(kind, LocalDataResetKind::All) {
            Some(self.generation.stop_all_for_user(user_id)?)
        } else {
            None
        };
        let _feature_reset_block = if matches!(kind, LocalDataResetKind::All) {
            Some(self.features.stop_all_for_user(user_id)?)
        } else {
            None
        };
        // Factor evidence owns references into Feature Datasets, so release
        // those references before the Feature store prunes its content.
        if matches!(kind, LocalDataResetKind::All) {
            self.factor.reset_for_user(user_id)?;
            self.features.reset_materialization_for_user(user_id)?;
        }
        let resets_ashare = matches!(
            kind,
            LocalDataResetKind::MarketData | LocalDataResetKind::All
        );
        let resets_us_equity = resets_ashare;
        // Query the Validation module before locking the database mutex so
        // the hook never re-enters a held lock. The Snapshot orphan query
        // and the Backtest and Component hooks run inside the reset flows
        // under the held lock instead, so they stay serialized with
        // Snapshot persistence and Run writes.
        let validation_report_count = if matches!(kind, LocalDataResetKind::MarketData) {
            self.validation.summary_for_user(user_id)?.report_count
        } else {
            0
        };
        if resets_ashare {
            if let Err(error) = self.pipeline.begin_user_reset(user_id) {
                return Err(string(error));
            }
        }
        let pipeline_snapshot_blocker_count = if matches!(
            kind,
            LocalDataResetKind::MarketData | LocalDataResetKind::All
        ) {
            match self.pipeline.snapshot_deletion_blockers_for_user(user_id) {
                Ok(blockers) => blockers.len() as u64,
                Err(error) => {
                    if resets_ashare {
                        let _ = self.pipeline.finish_user_reset(user_id);
                    }
                    return Err(string(error));
                }
            }
        } else {
            0
        };
        if resets_ashare {
            if let Err(error) = self.ashare.begin_user_reset(user_id) {
                let _ = self.pipeline.finish_user_reset(user_id);
                return Err(string(error));
            }
        }
        if resets_us_equity {
            if let Err(error) = self.us_equity.begin_user_reset(user_id) {
                let _ = self.ashare.finish_user_reset(user_id);
                let _ = self.pipeline.finish_user_reset(user_id);
                return Err(string(error));
            }
        }
        let mut database = match self.database.lock() {
            Ok(database) => database,
            Err(error) => {
                if resets_ashare {
                    let _ = self.us_equity.finish_user_reset(user_id);
                    let _ = self.ashare.finish_user_reset(user_id);
                    let _ = self.pipeline.finish_user_reset(user_id);
                }
                return Err(string(error));
            }
        };
        let paths = if resets_ashare {
            self.pipeline
                .reset_paths_for_user_with_connection(&database, user_id)
                .and_then(|pipeline_paths| {
                    self.ashare
                        .reset_paths_for_user_with_connection(&database, user_id)
                        .and_then(|ashare_paths| {
                            self.us_equity
                                .reset_paths_for_user_with_connection(&database, user_id)
                                .map(|us_equity_paths| {
                                    (pipeline_paths, ashare_paths, us_equity_paths)
                                })
                        })
                })
                .map_err(string)
        } else {
            Ok((Vec::new(), Vec::new(), Vec::new()))
        };
        let (pipeline_paths, ashare_paths, us_equity_paths) = match paths {
            Ok(paths) => paths,
            Err(error) => {
                if resets_ashare {
                    let _ = self.us_equity.finish_user_reset(user_id);
                    let _ = self.ashare.finish_user_reset(user_id);
                    let _ = self.pipeline.finish_user_reset(user_id);
                }
                return Err(error);
            }
        };
        let result = match kind {
            LocalDataResetKind::Watchlist => reset_watchlist(&mut database, user_id),
            LocalDataResetKind::Components => {
                self.components.reset_for_user(&mut database, user_id)
            }
            LocalDataResetKind::MarketData => reset_market_data(
                &mut database,
                user_id,
                &self.root,
                validation_report_count,
                pipeline_snapshot_blocker_count,
                &self.snapshots,
                &self.backtests,
                &self.ashare,
                &self.us_equity,
                &self.pipeline,
                pipeline_paths.clone(),
                ashare_paths.clone(),
                us_equity_paths.clone(),
            ),
            LocalDataResetKind::All => reset_all(
                &mut database,
                user_id,
                &self.root,
                _reset_block.as_ref().unwrap(),
                _feature_reset_block.as_ref().unwrap(),
                &self.components,
                &self.validation,
                &self.snapshots,
                &self.backtests,
                pipeline_snapshot_blocker_count,
                &self.ashare,
                &self.us_equity,
                &self.pipeline,
                pipeline_paths,
                ashare_paths,
                us_equity_paths,
            ),
        };
        if resets_ashare {
            let us_equity_finish = self.us_equity.finish_user_reset(user_id).map_err(string);
            let finish = self.ashare.finish_user_reset(user_id).map_err(string);
            let pipeline_finish = self.pipeline.finish_user_reset(user_id).map_err(string);
            match (result, us_equity_finish, finish, pipeline_finish) {
                (Err(error), _, _, _) => Err(error),
                (Ok(()), Err(error), _, _) => Err(error),
                (Ok(()), Ok(()), Err(error), _) => Err(error),
                (Ok(()), Ok(()), Ok(()), Err(error)) => Err(error),
                (Ok(()), Ok(()), Ok(()), Ok(())) => Ok(()),
            }
        } else {
            result
        }
    }

    #[cfg(not(feature = "deferred-equity"))]
    pub fn reset_local_data(&self, user_id: &str, kind: LocalDataResetKind) -> Result<(), String> {
        validate_user(user_id)?;
        let reset_block = if matches!(kind, LocalDataResetKind::All) {
            Some(self.generation.stop_all_for_user(user_id)?)
        } else {
            None
        };
        let feature_reset_block = if matches!(kind, LocalDataResetKind::All) {
            Some(self.features.stop_all_for_user(user_id)?)
        } else {
            None
        };
        if matches!(kind, LocalDataResetKind::All) {
            self.factor.reset_for_user(user_id)?;
            self.features.reset_materialization_for_user(user_id)?;
        }
        let resets_market_data = matches!(
            kind,
            LocalDataResetKind::MarketData | LocalDataResetKind::All
        );
        if resets_market_data {
            self.pipeline.begin_user_reset(user_id).map_err(string)?;
        }
        let blocker_count = if resets_market_data {
            self.pipeline
                .snapshot_deletion_blockers_for_user(user_id)
                .map_err(string)?
                .len() as u64
        } else {
            0
        };
        let mut database = self.database.lock().map_err(string)?;
        let pipeline_paths = if resets_market_data {
            self.pipeline
                .reset_paths_for_user_with_connection(&database, user_id)
                .map_err(string)?
        } else {
            Vec::new()
        };
        let result = match kind {
            LocalDataResetKind::Watchlist => reset_watchlist(&mut database, user_id),
            LocalDataResetKind::Components => {
                self.components.reset_for_user(&mut database, user_id)
            }
            LocalDataResetKind::MarketData => reset_market_data_okx(
                &mut database,
                user_id,
                &self.root,
                &self.snapshots,
                &self.backtests,
                blocker_count,
                &self.pipeline,
                pipeline_paths,
            ),
            LocalDataResetKind::All => reset_all_okx(
                &mut database,
                user_id,
                &self.root,
                reset_block.as_ref().unwrap(),
                feature_reset_block.as_ref().unwrap(),
                &self.components,
                &self.validation,
                &self.snapshots,
                &self.backtests,
                blocker_count,
                &self.pipeline,
                pipeline_paths,
            ),
        };
        if resets_market_data {
            self.pipeline.finish_user_reset(user_id).map_err(string)?;
        }
        result
    }

    pub(crate) fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.snapshots.persist_for_user(user_id, series)
    }

    #[cfg(test)]
    pub(crate) fn grant_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(), String> {
        self.snapshots.grant_for_user(user_id, snapshot_id)
    }

    pub(crate) fn package_for_user(
        &self,
        user_id: &str,
        hash: &str,
    ) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, hash)
    }

    pub(crate) fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }

    pub(crate) fn publish_pipeline_snapshot_for_user_with_policy(
        &self,
        user_id: &str,
        canonical_id: &str,
        allow_degraded: bool,
        publication_evidence_name: Option<String>,
    ) -> Result<(MarketDataSnapshot, DataQualityReport), String> {
        let cancellation = CancellationToken::new();
        self.publish_pipeline_snapshot_for_user_with_policy_and_cancellation(
            user_id,
            canonical_id,
            allow_degraded,
            publication_evidence_name,
            &cancellation,
        )
    }

    fn publish_pipeline_snapshot_for_user_with_policy_and_cancellation(
        &self,
        user_id: &str,
        canonical_id: &str,
        allow_degraded: bool,
        publication_evidence_name: Option<String>,
        cancellation: &CancellationToken,
    ) -> Result<(MarketDataSnapshot, DataQualityReport), String> {
        let _operation = self
            .pipeline
            .begin_user_operation(user_id, format!("snapshot:{canonical_id}"), cancellation)
            .map_err(string)?;
        let canonical = self
            .pipeline
            .canonical_for_user(user_id, canonical_id)
            .map_err(string)?;
        if cancellation.is_cancelled() {
            return Err("pipeline snapshot publication was cancelled".into());
        }
        let quality = self
            .pipeline
            .quality_for_user(user_id, &canonical.quality_report_id)
            .map_err(string)?;
        if quality.state == DataQualityState::Rejected {
            return Err(format!(
                "Market Data Snapshot publication is blocked by rejected quality report {}",
                quality.report_id
            ));
        }
        if quality.state == DataQualityState::Degraded && !allow_degraded {
            return Err(format!(
                "Market Data Snapshot publication requires explicit acceptance of degraded quality report {}",
                quality.report_id
            ));
        }
        let source = self
            .pipeline
            .source_for_user(user_id, &canonical.source_id)
            .map_err(string)?;
        let provenance = SnapshotProvenance {
            venue: canonical.instrument.venue.clone(),
            datasets: vec![SnapshotDatasetBinding {
                instrument: canonical.instrument.clone(),
                source_id: canonical.source_id.clone(),
                source_revision: source.revision,
                canonical_id: Some(canonical.canonical_id.clone()),
                derived_id: None,
                quality_report_id: quality.report_id.clone(),
                content_sha256: canonical.content_sha256.clone(),
            }],
            quality_report_ids: vec![quality.report_id.clone()],
            calendar_snapshot_ids: vec![canonical.calendar.identity()],
            provider_capability_snapshots: vec![
                serde_json::to_value(&source.identity.capability_snapshot).map_err(string)?,
            ],
            universe: None,
            derivation_algorithm_version: None,
        };
        let publication_evidence_name = self
            .pipeline
            .set_publication_evidence_name(user_id, &canonical.source_id, publication_evidence_name)
            .map_err(string)?;
        let snapshot = self.snapshots.persist_for_user_with_provenance_and_name(
            user_id,
            &canonical.to_bar_series(),
            Some(provenance),
            publication_evidence_name,
        )?;
        if cancellation.is_cancelled() {
            self.snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id)?;
            return Err("pipeline snapshot publication was cancelled".into());
        }
        if let Err(error) =
            self.pipeline
                .record_snapshot_reference(user_id, canonical_id, &snapshot.snapshot_id)
        {
            let _ = self.pipeline.remove_snapshot_reference(
                user_id,
                canonical_id,
                &snapshot.snapshot_id,
            );
            let cleanup = self
                .snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id);
            return match cleanup {
                Ok(()) => Err(string(error)),
                Err(cleanup_error) => Err(format!(
                    "{error}; failed to clean up persisted snapshot: {cleanup_error}"
                )),
            };
        }
        if cancellation.is_cancelled() {
            let reference_cleanup = self.pipeline.remove_snapshot_reference(
                user_id,
                canonical_id,
                &snapshot.snapshot_id,
            );
            let snapshot_cleanup = self
                .snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id);
            return match (reference_cleanup, snapshot_cleanup) {
                (Ok(()), Ok(())) => Err("pipeline snapshot publication was cancelled".into()),
                (reference, snapshot) => Err(format!(
                    "pipeline snapshot publication was cancelled; cleanup failed: reference={reference:?}, snapshot={snapshot:?}"
                )),
            };
        }
        Ok((snapshot, quality))
    }

    pub(crate) fn publish_okx_backfill(
        &self,
        user_id: &str,
        start_time_ms: i64,
        end_time_ms: i64,
        interval: BarInterval,
        instrument_codes: &[String],
        cancellation: &CancellationToken,
        publications: &[adaq_data_pipeline::PipelinePublication],
        publication_evidence_name: Option<String>,
    ) -> Result<MarketDataUniverseSnapshot, String> {
        let universe = self
            .okx
            .point_in_time_universe(user_id, end_time_ms)
            .map_err(string)?;
        let venue = Venue::crypto_spot("okx").map_err(string)?;
        let expected_instruments = universe
            .instruments
            .iter()
            .filter(|instrument| {
                instrument_codes.is_empty() || instrument_codes.contains(&instrument.code)
            })
            .map(|instrument| format!("{}:{}", venue.id, instrument.code))
            .collect::<HashSet<_>>();
        let mut published_instruments = HashSet::new();
        let canonical_publications = publications
            .iter()
            .map(|publication| {
                publication
                    .canonical
                    .as_ref()
                    .ok_or_else(|| "OKX backfill did not produce Canonical evidence".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        for canonical in &canonical_publications {
            let instrument_key = format!(
                "{}:{}",
                canonical.instrument.venue.id, canonical.instrument.code
            );
            if !published_instruments.insert(instrument_key) {
                return Err("OKX backfill produced duplicate instrument evidence".into());
            }
        }
        if published_instruments != expected_instruments {
            return Err(
                "OKX backfill did not produce complete Point-in-Time Universe coverage".into(),
            );
        }
        let mut components = Vec::with_capacity(canonical_publications.len());
        let mut quality_report_ids = Vec::new();
        let mut calendar_snapshot_ids = Vec::new();
        let mut provider_capability_snapshots = Vec::new();
        let mut coverage = None;

        for canonical in canonical_publications {
            if cancellation.is_cancelled() {
                return Err("OKX research data publication was cancelled".into());
            }
            let (snapshot, _) = self
                .publish_pipeline_snapshot_for_user_with_policy_and_cancellation(
                    user_id,
                    &canonical.canonical_id,
                    false,
                    publication_evidence_name.clone(),
                    cancellation,
                )?;
            let provenance = snapshot
                .provenance
                .as_ref()
                .ok_or_else(|| "OKX Snapshot provenance is missing".to_owned())?;
            let dataset = provenance
                .datasets
                .first()
                .cloned()
                .ok_or_else(|| "OKX Snapshot dataset provenance is missing".to_owned())?;
            if provenance.venue != venue
                || snapshot.interval != interval
                || snapshot.start_time_ms < start_time_ms
                || snapshot.end_time_ms > end_time_ms
            {
                return Err("OKX Snapshot coverage or venue does not match the backfill".into());
            }
            if let Some((expected_start, expected_end)) = coverage {
                if snapshot.start_time_ms != expected_start || snapshot.end_time_ms != expected_end
                {
                    return Err("OKX Snapshot coverage is not aligned across instruments".into());
                }
            } else {
                coverage = Some((snapshot.start_time_ms, snapshot.end_time_ms));
            }
            quality_report_ids.extend(provenance.quality_report_ids.iter().cloned());
            calendar_snapshot_ids.extend(provenance.calendar_snapshot_ids.iter().cloned());
            provider_capability_snapshots
                .extend(provenance.provider_capability_snapshots.iter().cloned());
            components.push(UniverseSnapshotComponent {
                snapshot_id: snapshot.snapshot_id,
                dataset,
            });
        }

        let (snapshot_start, snapshot_end) =
            coverage.ok_or_else(|| "OKX backfill produced no Snapshot evidence".to_owned())?;
        let instruments = components
            .iter()
            .map(|component| component.dataset.instrument.clone())
            .collect::<Vec<_>>();
        quality_report_ids.sort_unstable();
        quality_report_ids.dedup();
        calendar_snapshot_ids.sort_unstable();
        calendar_snapshot_ids.dedup();
        let evidence_state = universe_evidence_state(&universe);
        let universe_snapshot = MarketDataUniverseSnapshot {
            snapshot_id: String::new(),
            venue: venue.clone(),
            interval,
            start_time_ms: snapshot_start,
            end_time_ms: snapshot_end,
            universe: SnapshotUniverseBinding {
                universe_id: universe.universe_id,
                as_of_ms: universe.as_of_ms,
                evidence_state,
                evidence_reasons: universe.evidence_reasons,
                coverage_start_ms: universe.coverage_start_ms,
                coverage_end_ms: universe.coverage_end_ms,
                instruments,
            },
            components,
            quality_report_ids,
            calendar_snapshot_ids,
            provider_capability_snapshots,
            content_sha256: String::new(),
        };
        if cancellation.is_cancelled() {
            return Err("OKX research data publication was cancelled".into());
        }
        self.snapshots
            .persist_universe_for_user(user_id, universe_snapshot)
    }

    pub(crate) fn publish_pipeline_derived_snapshot_for_user_with_policy(
        &self,
        user_id: &str,
        derived_id: &str,
        allow_degraded: bool,
    ) -> Result<(MarketDataSnapshot, DataQualityReport), String> {
        let derived = self
            .pipeline
            .derived_for_user(user_id, derived_id)
            .map_err(string)?;
        let canonical = self
            .pipeline
            .canonical_for_user(user_id, &derived.canonical_id)
            .map_err(string)?;
        let quality = self
            .pipeline
            .quality_for_user(user_id, &canonical.quality_report_id)
            .map_err(string)?;
        if quality.state == DataQualityState::Rejected {
            return Err(format!(
                "Market Data Snapshot publication is blocked by rejected quality report {}",
                quality.report_id
            ));
        }
        if quality.state == DataQualityState::Degraded && !allow_degraded {
            return Err(format!(
                "Market Data Snapshot publication requires explicit acceptance of degraded quality report {}",
                quality.report_id
            ));
        }
        let source = self
            .pipeline
            .source_for_user(user_id, &derived.source_id)
            .map_err(string)?;
        let provenance = SnapshotProvenance {
            venue: derived.instrument.venue.clone(),
            datasets: vec![SnapshotDatasetBinding {
                instrument: derived.instrument.clone(),
                source_id: derived.source_id.clone(),
                source_revision: source.revision,
                canonical_id: Some(derived.canonical_id.clone()),
                derived_id: Some(derived.derived_id.clone()),
                quality_report_id: quality.report_id.clone(),
                content_sha256: derived.content_sha256.clone(),
            }],
            quality_report_ids: vec![quality.report_id.clone()],
            calendar_snapshot_ids: vec![derived.calendar.identity()],
            provider_capability_snapshots: vec![
                serde_json::to_value(&source.identity.capability_snapshot).map_err(string)?,
            ],
            universe: None,
            derivation_algorithm_version: Some(derived.algorithm_version.clone()),
        };
        let snapshot = self.snapshots.persist_for_user_with_provenance(
            user_id,
            &derived.to_bar_series(),
            Some(provenance),
        )?;
        if let Err(error) = self.pipeline.record_derived_snapshot_reference(
            user_id,
            derived_id,
            &snapshot.snapshot_id,
        ) {
            let cleanup = self
                .snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id);
            return match cleanup {
                Ok(()) => Err(string(error)),
                Err(cleanup_error) => Err(format!(
                    "{error}; failed to clean up persisted snapshot: {cleanup_error}"
                )),
            };
        }
        Ok((snapshot, quality))
    }

    pub(crate) fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        self.source.runtime_component(package)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCompatibilityRequest {
    user_id: String,
    strategy_archive_sha256: String,
    snapshot_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCandidate {
    slot: String,
    dataset_id: String,
    signal_name: String,
    evidence_state: String,
}

/// The one cross-domain compatibility command: it joins Component Package
/// reads through the Component Library module, Snapshot reads through the
/// Snapshot module, and Signal Dataset reads through the forecast_signal_dataset-owned path.
#[tauri::command]
pub fn backtest_compatible_signals(
    mut request: BacktestSignalCompatibilityRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<BacktestSignalCandidate>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let strategy = state.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if strategy.manifest.kind != ComponentKind::Strategy {
        return Err("Compatible Signals require a Strategy Component".into());
    }
    let (snapshot, _) = state.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    let datasets = backtest_signal_datasets(&state, &request.user_id, false, None)?;
    Ok(compatible_signal_candidates(
        &strategy.manifest,
        &snapshot,
        &datasets,
    ))
}

fn compatible_signal_candidates(
    strategy: &ComponentManifest,
    snapshot: &MarketDataSnapshot,
    datasets: &[crate::forecast_signal_dataset::BacktestSignalDataset],
) -> Vec<BacktestSignalCandidate> {
    let mut candidates = Vec::new();
    for slot in &strategy.feature_slots {
        let FeatureSlotSource::Signal {
            prediction_kind,
            forecast_target,
            value_scale,
            horizon_bars,
        } = &slot.source
        else {
            continue;
        };
        for dataset in datasets.iter().filter(|dataset| {
            dataset.snapshot_id == snapshot.snapshot_id
                && dataset.src == snapshot.src
                && dataset.code == snapshot.code
                && dataset.interval == snapshot.interval.as_str()
        }) {
            for output in dataset.outputs.iter().filter(|output| {
                output.prediction_kind == *prediction_kind
                    && output.forecast_target == *forecast_target
                    && output.value_scale == *value_scale
                    && output.horizon_bars == *horizon_bars
            }) {
                candidates.push(BacktestSignalCandidate {
                    slot: slot.name.clone(),
                    dataset_id: dataset.dataset_id.clone(),
                    signal_name: output.name.clone(),
                    evidence_state: dataset.evidence_state.clone(),
                });
            }
        }
    }
    candidates
}

#[tauri::command]
pub fn local_data_summary(
    request: LocalDataRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<LocalDataSummary, String> {
    let _ = request;
    let user_id = auth.user_id_for_window(window.label())?;
    state.local_data_summary(&user_id)
}

#[tauri::command]
pub async fn local_data_reset(
    request: LocalDataResetRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let kind = request.kind;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.reset_local_data(&user_id, kind)?;
        if matches!(kind, LocalDataResetKind::All) {
            app.state::<Arc<crate::strategy_candidate::StrategyCandidateStore>>()
                .reset_user(&user_id)?;
        }
        Ok(())
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub async fn factor_research_device_reset(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .factor
            .reset_for_device()
    })
    .await
    .map_err(string)?
}

fn reset_watchlist(database: &mut Connection, user_id: &str) -> Result<(), String> {
    let transaction = database.transaction().map_err(string)?;
    transaction
        .execute(
            "DELETE FROM watchlist_settings WHERE user_id = ?1",
            [user_id],
        )
        .map_err(string)?;
    insert_default_watchlist(&transaction, user_id)?;
    transaction.commit().map_err(string)
}

#[cfg(not(feature = "deferred-equity"))]
fn reset_market_data_okx(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    blocker_count: u64,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
) -> Result<(), String> {
    if blocker_count > 0 {
        return Err(format!(
            "Market Data reset is blocked by {blocker_count} immutable research record(s)"
        ));
    }
    let blocking_datasets: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    if backtests.run_count(database, user_id)? + blocking_datasets.max(0) as u64 > 0 {
        return Err("Market Data reset is blocked by immutable research records".into());
    }
    let staged = stage_files(
        snapshots
            .orphaned_parquet_paths(database, user_id)?
            .into_iter()
            .chain(pipeline_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

#[cfg(not(feature = "deferred-equity"))]
fn reset_all_okx(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    reset_block: &crate::dataset_generation::UserResetBlock<'_>,
    feature_reset_block: &crate::features::UserFeatureResetBlock<'_>,
    components: &ComponentLibrary,
    validation: &ValidationStudies,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    blocker_count: u64,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
) -> Result<(), String> {
    if blocker_count > 0 {
        return Err(format!(
            "All local data reset is blocked by {blocker_count} pipeline snapshot reference(s)"
        ));
    }
    let component_paths = components.orphan_archive_paths(database, user_id, Some(user_id))?;
    let dataset_paths = strings(
        database,
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND NOT EXISTS(SELECT 1 FROM signal_dataset_access other WHERE other.dataset_id = c.dataset_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let staged = stage_files(
        component_paths
            .into_iter()
            .chain(snapshots.orphaned_parquet_paths(database, user_id)?)
            .chain(dataset_paths.into_iter().map(PathBuf::from))
            .chain(pipeline_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        validation.reset_for_user(&transaction, user_id)?;
        reset_block.delete_attempt_evidence(&transaction)?;
        feature_reset_block.delete_attempt_evidence(&transaction)?;
        backtests.reset_for_user(&transaction, user_id)?;
        transaction
            .execute(
                "DELETE FROM signal_dataset_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute("DELETE FROM signal_dataset_content WHERE NOT EXISTS(SELECT 1 FROM signal_dataset_access a WHERE a.dataset_id = signal_dataset_content.dataset_id)", [])
            .map_err(string)?;
        components.reset_access_for_user(&transaction, user_id)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        components.delete_orphan_content(&transaction, Some(user_id))?;
        transaction
            .execute(
                "DELETE FROM watchlist_settings WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        insert_default_watchlist(&transaction, user_id)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

#[cfg(feature = "deferred-equity")]
fn reset_market_data(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    validation_report_count: u64,
    pipeline_snapshot_blocker_count: u64,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    ashare: &AshareDataPath,
    us_equity: &UsEquityDataPath,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
    ashare_paths: Vec<PathBuf>,
    us_equity_paths: Vec<PathBuf>,
) -> Result<(), String> {
    let blocking_datasets: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    let blocking = backtests.run_count(database, user_id)?
        + blocking_datasets.max(0) as u64
        + validation_report_count
        + pipeline_snapshot_blocker_count;
    if blocking > 0 {
        return Err(format!(
            "Market Data reset is blocked by {blocking} immutable research record(s)"
        ));
    }
    let staged = stage_files(
        snapshots
            .orphaned_parquet_paths(database, user_id)?
            .into_iter()
            .chain(pipeline_paths)
            .chain(ashare_paths)
            .chain(us_equity_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        ashare
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        us_equity
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

#[cfg(feature = "deferred-equity")]
fn reset_all(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    reset_block: &crate::dataset_generation::UserResetBlock<'_>,
    feature_reset_block: &crate::features::UserFeatureResetBlock<'_>,
    components: &ComponentLibrary,
    validation: &ValidationStudies,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    pipeline_snapshot_blocker_count: u64,
    ashare: &AshareDataPath,
    us_equity: &UsEquityDataPath,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
    ashare_paths: Vec<PathBuf>,
    us_equity_paths: Vec<PathBuf>,
) -> Result<(), String> {
    // The reset User's Runs are deleted inside the transaction below, so
    // only other Users' Runs keep locking Component content; the Component
    // module derives that guard from the Backtest module's lock query, and
    // it stays stable under the held database lock for both the staged
    // file selection and the transaction's orphan cleanup.
    if pipeline_snapshot_blocker_count > 0 {
        return Err(format!(
            "All local data reset is blocked by {pipeline_snapshot_blocker_count} pipeline snapshot reference(s)"
        ));
    }
    let component_paths = components.orphan_archive_paths(database, user_id, Some(user_id))?;
    let dataset_paths = strings(
        database,
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND NOT EXISTS(SELECT 1 FROM signal_dataset_access other WHERE other.dataset_id = c.dataset_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let staged = stage_files(
        component_paths
            .into_iter()
            .chain(snapshots.orphaned_parquet_paths(database, user_id)?)
            .chain(dataset_paths.into_iter().map(PathBuf::from))
            .chain(pipeline_paths)
            .chain(ashare_paths)
            .chain(us_equity_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        validation.reset_for_user(&transaction, user_id)?;
        reset_block.delete_attempt_evidence(&transaction)?;
        feature_reset_block.delete_attempt_evidence(&transaction)?;
        transaction
            .execute(
                "DELETE FROM forecast_evaluation_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM forecast_evaluation_content WHERE NOT EXISTS(SELECT 1 FROM forecast_evaluation_access a WHERE a.report_id = forecast_evaluation_content.report_id)", []).map_err(string)?;
        backtests.reset_for_user(&transaction, user_id)?;
        transaction
            .execute(
                "DELETE FROM signal_dataset_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM signal_dataset_content WHERE NOT EXISTS(SELECT 1 FROM signal_dataset_access a WHERE a.dataset_id = signal_dataset_content.dataset_id)", []).map_err(string)?;
        components.reset_access_for_user(&transaction, user_id)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        ashare
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        us_equity
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        components.delete_orphan_content(&transaction, Some(user_id))?;
        transaction
            .execute(
                "DELETE FROM watchlist_settings WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        insert_default_watchlist(&transaction, user_id)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn strings(database: &Connection, sql: &str, user_id: &str) -> Result<Vec<String>, String> {
    database
        .prepare(sql)
        .map_err(string)?
        .query_map([user_id], |row| row.get(0))
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

fn file_bytes(path: impl AsRef<Path>) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

fn universe_evidence_state(universe: &PointInTimeInstrumentUniverse) -> String {
    match universe.evidence_state {
        adaq_data_pipeline::okx::UniverseEvidenceState::Observed => "observed".into(),
        adaq_data_pipeline::okx::UniverseEvidenceState::Reconstructed => "reconstructed".into(),
        adaq_data_pipeline::okx::UniverseEvidenceState::Unknown => "unknown".into(),
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn factor_context_market_venue(snapshot: &MarketDataSnapshot) -> Result<(String, String), String> {
    let Some(provenance) = snapshot.provenance.as_ref() else {
        return Err("factor-context-market-venue-unavailable".into());
    };
    let venue = provenance.venue.clone();
    let market = match venue.kind {
        VenueKind::CryptoSpot => "crypto",
        VenueKind::ChinaAShareEquity => "a-share",
        VenueKind::UsEquity => "us-equity",
    };
    Ok((market.into(), venue.id))
}

fn factor_valuation_currency(
    market: &str,
    instrument_code: &str,
    explicit: &str,
) -> Result<String, String> {
    if !explicit.trim().is_empty() {
        return Ok(explicit.trim().into());
    }
    match market {
        "crypto" => instrument_code
            .rsplit_once('-')
            .map(|(_, quote)| quote.to_owned())
            .filter(|quote| !quote.trim().is_empty())
            .ok_or_else(|| "factor-context-valuation-currency-required".into()),
        "a-share" => Ok("CNY".into()),
        "us-equity" => Ok("USD".into()),
        _ => Err("factor-context-valuation-currency-required".into()),
    }
}

fn factor_parameter_defaults(
    parameters: &[adaq_factor_research::FactorParameter],
) -> Result<Vec<adaq_factor_research::FactorParameterValue>, String> {
    parameters
        .iter()
        .map(|parameter| match parameter.parameter_type {
            adaq_factor_research::FactorParameterType::Decimal => {
                if !parameter
                    .default_value
                    .parse::<f64>()
                    .ok()
                    .is_some_and(f64::is_finite)
                {
                    return Err("factor-candidate-invalid".into());
                }
                Ok(adaq_factor_research::FactorParameterValue::Decimal(
                    parameter.default_value.clone(),
                ))
            }
            adaq_factor_research::FactorParameterType::Integer => parameter
                .default_value
                .parse::<i64>()
                .map(adaq_factor_research::FactorParameterValue::Integer)
                .map_err(|_| "factor-candidate-invalid".into()),
            adaq_factor_research::FactorParameterType::Boolean => parameter
                .default_value
                .parse::<bool>()
                .map(adaq_factor_research::FactorParameterValue::Boolean)
                .map_err(|_| "factor-candidate-invalid".into()),
            adaq_factor_research::FactorParameterType::Text => Ok(
                adaq_factor_research::FactorParameterValue::Text(parameter.default_value.clone()),
            ),
        })
        .collect()
}

fn factor_native_engine_identity(
    candidate: &adaq_factor_research::FactorCandidate,
    candidate_hash: &str,
    context: &adaq_factor_research::ResearchEvidenceProjection,
) -> adaq_factor_research::ResearchEngineProvenance {
    adaq_factor_research::ResearchEngineProvenance {
        engine_id: "adaq-native-factor".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        adapter: "native-factor-materializer".into(),
        target_triple: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        build_id: env!("CARGO_PKG_VERSION").into(),
        environment: BTreeMap::new(),
        parameters: BTreeMap::from([
            (
                "contextRevision".into(),
                context.context_revision.to_string(),
            ),
            ("scope".into(), candidate.scope.world().into()),
        ]),
        input_identities: vec![
            candidate_hash.into(),
            context.context_hash.clone(),
            context
                .feature_dataset
                .as_ref()
                .map(|binding| binding.dataset_id.clone())
                .unwrap_or_default(),
            context.snapshot_id.clone(),
            context.universe_id.clone().unwrap_or_default(),
        ],
    }
}

fn factor_context_dataset_error(error: &str) -> String {
    match error.split(':').next().unwrap_or_default() {
        "feature-dataset-not-found" | "feature-dataset-not-authorized" => {
            "factor-context-feature-dataset-inaccessible".into()
        }
        "invalid-feature-dataset-schema"
        | "incomplete-feature-dataset-rows"
        | "duplicate-feature-observation"
        | "invalid-feature-observation"
        | "feature-dataset-content-collision"
        | "incompatible-feature-schema" => "factor-context-feature-dataset-incomplete".into(),
        _ => "factor-context-feature-dataset-unavailable".into(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backtest::{BacktestRunRequest, FactorInstanceRequest},
        watchlist::WatchlistDb,
    };
    use adaq_backtest_core::ExecutionProfile;
    use adaq_data_core::{
        BarGap, BarInterval, HistoricalBarRange, InstrumentMasterAcquisition, InstrumentStatus,
        OkxRequestDiagnostics, SpotInstrument,
        market::{InstrumentId, Venue},
    };
    use adaq_data_pipeline::{
        AcquisitionDiagnostics, CalendarEvidence, CanonicalizationRequest,
        ProviderCapabilitySnapshot, SourceAcquisition, SourceMarketRecord,
    };
    use rust_decimal::Decimal;
    use std::{
        collections::HashMap,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn local_data_state(name: &str) -> (PathBuf, Arc<LocalResearchState>, WatchlistDb) {
        let root = std::env::temp_dir().join(format!(
            "adaq-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let watchlist = WatchlistDb::open(&root.join("adaq.db")).unwrap();
        (root, state, watchlist)
    }

    #[test]
    fn factor_context_range_uses_half_open_bar_coverage() {
        const HOUR: i64 = 3_600_000;
        let venue = Venue::crypto_spot("okx").unwrap();
        let snapshot = MarketDataSnapshot {
            snapshot_id: "snapshot".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            start_time_ms: 0,
            end_time_ms: HOUR,
            bar_count: 2,
            gaps: vec![],
            parquet_path: PathBuf::new(),
            provenance: None,
            publication_evidence_name: None,
        };
        let universe = MarketDataUniverseSnapshot {
            snapshot_id: "universe".into(),
            venue,
            interval: BarInterval::OneHour,
            start_time_ms: 0,
            end_time_ms: HOUR,
            universe: SnapshotUniverseBinding {
                universe_id: "pit".into(),
                as_of_ms: 0,
                evidence_state: "observed".into(),
                evidence_reasons: vec!["test".into()],
                coverage_start_ms: Some(0),
                coverage_end_ms: Some(HOUR),
                instruments: vec![],
            },
            components: vec![],
            quality_report_ids: vec![],
            calendar_snapshot_ids: vec![],
            provider_capability_snapshots: vec![],
            content_sha256: "content".into(),
        };

        assert!(
            LocalResearchState::validate_factor_context_range(0, 2 * HOUR, &snapshot, &universe,)
                .is_ok()
        );
        assert_eq!(
            LocalResearchState::validate_factor_context_range(0, 3 * HOUR, &snapshot, &universe,)
                .unwrap_err(),
            "factor-context-range-mismatch"
        );
    }

    fn public_example_package(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/components")
            .join(name)
            .join("dist")
            .join(format!("{name}-0.1.0.adaq"));
        assert!(
            path.is_file(),
            "build the {name} example with adaq-component build"
        );
        fs::read(path).unwrap()
    }

    #[test]
    fn compatible_signal_selection_requires_exact_market_and_semantic_identity() {
        let strategy: ComponentManifest = serde_json::from_value(serde_json::json!({
            "manifestSchemaVersion": "1.0.0",
            "componentId": "00000000-0000-4000-8000-000000000001",
            "version": "1.0.0",
            "name": "Signal Strategy",
            "kind": "strategy",
            "sdkVersion": "0.1.0",
            "abiVersion": "1.0.0",
            "featureSlots": [{
                "name": "forecast-up",
                "source": {
                    "kind": "signal",
                    "predictionKind": {"kind": "probability"},
                    "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                    "valueScale": {"kind": "probability"},
                    "horizonBars": 1
                }
            }]
        }))
        .unwrap();
        let snapshot = MarketDataSnapshot {
            snapshot_id: "snapshot".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            start_time_ms: 0,
            end_time_ms: 3_600_000,
            bar_count: 1,
            gaps: vec![],
            parquet_path: PathBuf::new(),
            provenance: None,
            publication_evidence_name: None,
        };
        let dataset = crate::forecast_signal_dataset::BacktestSignalDataset {
            dataset_id: "a".repeat(64),
            snapshot_id: snapshot.snapshot_id.clone(),
            src: snapshot.src.clone(),
            code: snapshot.code.clone(),
            interval: snapshot.interval.as_str().into(),
            outputs: vec![adaq_component_tooling::ModelOutput {
                name: "up".into(),
                prediction_kind: adaq_component_tooling::PredictionKind::Probability,
                forecast_target: adaq_component_tooling::ForecastTarget::Builtin {
                    target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
                },
                value_scale: adaq_component_tooling::ForecastValueScale::Probability,
                horizon_bars: 1,
            }],
            producer_segments: vec![serde_json::json!({"segment": 1})],
            artifact_provenance: serde_json::json!({"sha256": "artifact"}),
            evidence_state: "unknown".into(),
            component_lock: vec![],
            rows: vec![],
        };
        let mut wrong_snapshot = dataset.clone();
        wrong_snapshot.dataset_id = "b".repeat(64);
        wrong_snapshot.snapshot_id = "other".into();
        let mut wrong_horizon = dataset.clone();
        wrong_horizon.dataset_id = "c".repeat(64);
        wrong_horizon.outputs[0].horizon_bars = 2;

        let candidates = compatible_signal_candidates(
            &strategy,
            &snapshot,
            &[dataset, wrong_snapshot, wrong_horizon],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].slot, "forecast-up");
        assert_eq!(candidates[0].signal_name, "up");
    }

    #[test]
    fn reset_all_is_user_scoped_and_restores_watchlist_defaults() {
        let (root, state, watchlist) = local_data_state("reset-all");
        watchlist.get("alice").unwrap();
        watchlist.get("bob").unwrap();
        let package = root.join("m3/components/shared.adaq");
        fs::write(&package, b"package").unwrap();
        let database = state.database.lock().unwrap();
        database.execute(
			"INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES ('hash', 'component', '1.0.0', 'Shared', 'factor', 'wasm', ?1)",
			[package.to_string_lossy().as_ref()],
		).unwrap();
        for user in ["alice", "bob"] {
            database
                .execute("INSERT INTO component_access VALUES (?1, 'hash')", [user])
                .unwrap();
        }
        let dataset = root.join("m3/signal-datasets/shared.parquet");
        fs::create_dir_all(dataset.parent().unwrap()).unwrap();
        fs::write(&dataset, b"parquet").unwrap();
        database
            .execute(
                "INSERT INTO signal_dataset_content VALUES ('dataset', '{}', ?1)",
                [dataset.to_string_lossy().as_ref()],
            )
            .unwrap();
        for user in ["alice", "bob"] {
            database
                .execute(
                    "INSERT INTO signal_dataset_access VALUES (?1, 'dataset')",
                    [user],
                )
                .unwrap();
        }
        database
            .execute(
                "INSERT INTO dataset_generation_attempts
                 (attempt_id, request_hash, user_id, status, request_json)
                 VALUES ('attempt', 'request', 'alice', 'pending', '{}')",
                [],
            )
            .unwrap();
        drop(database);

        state
            .reset_local_data("alice", LocalDataResetKind::All)
            .unwrap();

        let alice = state.local_data_summary("alice").unwrap();
        let bob = state.local_data_summary("bob").unwrap();
        assert_eq!(alice.watchlist_count, 16);
        assert_eq!(alice.component_count, 0);
        assert_eq!(alice.generation_attempt_count, 0);
        assert_eq!(alice.signal_dataset_count, 0);
        assert_eq!(bob.watchlist_count, 16);
        assert_eq!(bob.component_count, 1);
        assert_eq!(bob.signal_dataset_count, 1);
        assert!(package.is_file());
        assert!(dataset.is_file());
    }

    #[test]
    fn component_reset_refuses_to_break_a_run_lock() {
        let (root, state, watchlist) = local_data_state("reset-components");
        watchlist.get("alice").unwrap();
        let package = root.join("m3/components/locked.adaq");
        fs::write(&package, b"package").unwrap();
        let database = state.database.lock().unwrap();
        database.execute(
			"INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES ('hash', 'component', '1.0.0', 'Locked', 'factor', 'wasm', ?1)",
			[package.to_string_lossy().as_ref()],
		).unwrap();
        database
            .execute("INSERT INTO component_access VALUES ('alice', 'hash')", [])
            .unwrap();
        database.execute("INSERT INTO backtest_runs(run_id, user_id, result_json) VALUES ('run', 'alice', '{}')", []).unwrap();
        database
            .execute(
                "INSERT INTO backtest_run_components VALUES ('run', 'hash')",
                [],
            )
            .unwrap();
        drop(database);

        let error = state
            .reset_local_data("alice", LocalDataResetKind::Components)
            .unwrap_err();
        assert!(error.contains("blocked by 1 immutable Backtest Run"));
        assert_eq!(
            state.local_data_summary("alice").unwrap().component_count,
            1
        );
        assert!(package.is_file());
    }

    #[test]
    fn public_examples_import_and_execute_a_deterministic_backtest() {
        let root = std::env::temp_dir().join(format!(
            "adaq-m6-examples-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let factor_bytes = public_example_package("factor-close-momentum-5");
        let strategy_bytes = public_example_package("strategy-momentum-trend");
        state.components.import("alice", &factor_bytes).unwrap();
        state.components.import("alice", &strategy_bytes).unwrap();
        let factor_hash = ComponentPackage::read(&factor_bytes)
            .unwrap()
            .archive_sha256;
        let strategy_hash = ComponentPackage::read(&strategy_bytes)
            .unwrap()
            .archive_sha256;
        assert_eq!(
            state
                .components
                .compatible_factors("alice", &strategy_hash)
                .unwrap()["momentum"],
            [factor_hash.clone()]
        );
        let bars = (0..50)
            .map(|index| {
                let close = rust_decimal::Decimal::from(100 + index);
                let time_index = if index < 25 { index } else { index + 5 };
                OhlcvBar {
                    open_time_ms: time_index * 3_600_000,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    base_volume: rust_decimal::Decimal::ONE,
                    quote_volume: close,
                }
            })
            .collect();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &adaq_data_core::BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars,
                    gaps: vec![BarGap {
                        start_time_ms: 25 * 3_600_000,
                        end_time_ms: 30 * 3_600_000,
                    }],
                },
            )
            .unwrap();
        let request = || BacktestRunRequest {
            user_id: "alice".into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            run_start_time_ms: None,
            run_end_time_ms: None,
            factor_instances: vec![FactorInstanceRequest {
                alias: "momentum".into(),
                archive_sha256: factor_hash.clone(),
                parameters: HashMap::new(),
            }],
            signal_instances: vec![],
            strategy_archive_sha256: strategy_hash.clone(),
            strategy_parameters: HashMap::new(),
            initial_quote_allocation: 10_000.into(),
            execution_profile: ExecutionProfile {
                maker_fee_rate: rust_decimal::Decimal::new(8, 4),
                taker_fee_rate: rust_decimal::Decimal::new(1, 3),
                adverse_slippage_rate: rust_decimal::Decimal::ZERO,
                rebalance_threshold: rust_decimal::Decimal::ZERO,
                price_increment: rust_decimal::Decimal::ONE,
                quantity_increment: rust_decimal::Decimal::new(1, 4),
                minimum_quantity: rust_decimal::Decimal::new(1, 4),
                risk_free_rate: rust_decimal::Decimal::ZERO,
                fill_policy: adaq_backtest_core::FillPolicy::Taker,
            },
            seed: 0,
        };

        let first = state.backtests.run(request()).unwrap();
        let replay = state.backtests.run(request()).unwrap();

        assert_eq!(first.run_id, replay.run_id);
        let provenance = first.provenance.as_ref().unwrap();
        assert_eq!(
            provenance.normalized_request.snapshot_id,
            snapshot.snapshot_id
        );
        assert_eq!(provenance.feature_plan_hash, first.plan_hash);
        assert_eq!(provenance.component_lock, first.component_lock);
        assert_eq!(
            state
                .backtests
                .get("alice", &first.run_id)
                .unwrap()
                .provenance,
            first.provenance
        );
        assert!(provenance.feature_plan_json.contains("\"slots\""));
        assert_eq!(
            provenance.normalized_request.initial_quote_allocation,
            rust_decimal::Decimal::from(10_000),
        );
        assert_eq!(first.component_lock.len(), 2);
        assert_eq!(first.pauses.len(), 38);
        assert!(!first.result.orders.is_empty());
        assert!(!first.result.fills.is_empty());
        let execution_page = state
            .backtests
            .execution_data(&crate::backtest::BacktestExecutionRequest {
                user_id: "alice".into(),
                run_id: first.run_id.clone(),
                offset: 0,
                limit: 1,
            })
            .unwrap();
        assert_eq!(execution_page.orders.len(), 1);
        assert_eq!(execution_page.fills.len(), 1);
        assert_eq!(execution_page.total_orders, first.result.orders.len());
        assert_eq!(execution_page.total_fills, first.result.fills.len());
        let run_count = || {
            state
                .backtests
                .list(&crate::backtest::BacktestListRequest {
                    user_id: "alice".into(),
                    src: None,
                    code: None,
                    page: 1,
                })
                .unwrap()
                .total
        };
        assert_eq!(run_count(), 1);
        let mut changed_request = request();
        changed_request.seed = 1;
        let changed = state.backtests.run(changed_request).unwrap();
        assert_ne!(first.run_id, changed.run_id);
        assert_eq!(run_count(), 2);

        // Validation Studies is a deep module: Protocols and Reports flow
        // through its interface, and an immutable Report locks the Backtest
        // Runs it references.
        let validation = crate::validation::ValidationProtocolCreateRequest {
            user_id: "alice".into(),
            run: request(),
            windows: vec![crate::validation::ValidationWindowRequest {
                snapshot_id: snapshot.snapshot_id.clone(),
                sample_out_start_time_ms: 25 * 3_600_000,
                sample_out_end_time_ms: None,
            }],
            walk_forward: None,
            cross_market: None,
            method_version: "chronological-holdout@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        assert!(
            state
                .validation
                .create_protocol(crate::validation::ValidationProtocolCreateRequest {
                    windows: vec![crate::validation::ValidationWindowRequest {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        sample_out_start_time_ms: 0,
                        sample_out_end_time_ms: None,
                    }],
                    ..validation.clone()
                })
                .is_err()
        );
        let protocol = state.validation.create_protocol(validation).unwrap();
        let report = state
            .validation
            .run_report(&protocol.user_id, &protocol.protocol_id)
            .unwrap();
        assert_eq!(report.aggregate.completed_windows, 1);
        assert_eq!(report.windows.len(), 1);
        assert_eq!(report.windows[0].sample_out_start_time_ms, 25 * 3_600_000);
        assert!(
            state
                .backtests
                .delete(
                    "alice",
                    report.windows[0].sample_in_run_id.as_deref().unwrap(),
                )
                .is_err()
        );
        assert_eq!(state.validation.list_reports("alice").unwrap().len(), 1);
        assert!(state.validation.list_reports("bob").unwrap().is_empty());
        let markdown = state
            .validation
            .export_report("alice", &report.report_id, "markdown")
            .unwrap();
        assert!(markdown.contains(&report.protocol_id));
        assert!(markdown.contains("research-metrics.md"));

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frozen_context_binding_survives_state_reload_and_attempt_lookup() {
        let (root, state, watchlist) = local_data_state("research-context-binding");
        let draft = adaq_factor_research::ResearchEvidenceContextDraft {
            user_id: "alice".into(),
            market: "crypto".into(),
            venue: "okx".into(),
            range_start_ms: 1,
            range_end_ms: 2,
            snapshot_id: "snapshot-1".into(),
            universe_id: Some("universe-1".into()),
            evidence: vec![adaq_factor_research::EvidenceBinding {
                id: "evidence-1".into(),
                lineage_hash: "lineage-1".into(),
                user_id: "alice".into(),
                market: "crypto".into(),
                venue: "okx".into(),
                snapshot_id: "snapshot-1".into(),
                universe_id: Some("universe-1".into()),
                feature_id: None,
                factor_id: None,
                model_id: None,
                grade: adaq_factor_research::EvidenceGrade::ProviderGraded,
                accessible: true,
                complete: true,
                fresh: true,
            }],
            feature_dataset: None,
        };
        let context = adaq_factor_research::ResearchEvidenceContext::establish_for_stage(
            draft,
            adaq_factor_research::ResearchStage::Models,
            Default::default(),
        )
        .unwrap();
        state.store_research_context(context).unwrap();
        state
            .freeze_research_context(
                "alice",
                "model-dataset:one:model".into(),
                adaq_factor_research::ResearchStage::Models,
            )
            .unwrap();
        state
            .record_research_attempt_binding(
                "alice",
                "model-dataset:one:model",
                adaq_factor_research::ResearchStage::Models,
                "attempt-1",
            )
            .unwrap();
        state
            .record_research_attempt_binding(
                "alice",
                "model-dataset:one:model",
                adaq_factor_research::ResearchStage::Models,
                "attempt-2",
            )
            .unwrap();
        assert_eq!(
            state
                .research_context_for_attempt("alice", "attempt-1")
                .unwrap()
                .unwrap()
                .operation_id,
            "model-dataset:one:model"
        );
        assert_eq!(
            state
                .research_context_for_attempt("alice", "attempt-2")
                .unwrap()
                .unwrap()
                .operation_id,
            "model-dataset:one:model"
        );
        drop(state);
        let reloaded = LocalResearchState::open(&root).unwrap();
        assert_eq!(
            reloaded
                .research_context_for_attempt("alice", "attempt-1")
                .unwrap()
                .unwrap()
                .operation_id,
            "model-dataset:one:model"
        );
        assert_eq!(
            reloaded
                .research_context_for_attempt("alice", "attempt-2")
                .unwrap()
                .unwrap()
                .operation_id,
            "model-dataset:one:model"
        );
        drop(reloaded);
        drop(watchlist);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn foundation_acquisition_history_preserves_three_markets_cancel_and_retry() {
        let (root, state, watchlist) = local_data_state("foundation-lifecycle");
        for (operation_id, market, venue) in [
            ("okx-1", "crypto", "okx"),
            ("ashare-1", "a-share", "local"),
            ("us-1", "us-equity", "alpaca"),
        ] {
            state
                .foundation_acquisition_start("alice", operation_id, market, venue)
                .unwrap();
        }
        state
            .foundation_acquisition_finish("alice", "okx-1", "completed", Some(1), None)
            .unwrap();
        state
            .foundation_acquisition_finish(
                "alice",
                "ashare-1",
                "cancelled",
                None,
                Some("cancelled by user"),
            )
            .unwrap();
        state
            .foundation_acquisition_start("alice", "ashare-2", "a-share", "local")
            .unwrap();
        state
            .foundation_acquisition_finish(
                "alice",
                "ashare-2",
                "failed",
                None,
                Some("retry failed"),
            )
            .unwrap();

        let history = state.foundation_acquisition_history("alice").unwrap();
        assert_eq!(history.len(), 4);
        assert!(history.iter().any(|entry| {
            entry.operation_id == "ashare-1"
                && entry.state == "cancelled"
                && entry.error.as_deref() == Some("cancelled by user")
        }));
        assert!(history.iter().any(|entry| {
            entry.operation_id == "ashare-2"
                && entry.state == "failed"
                && entry.error.as_deref() == Some("retry failed")
        }));
        assert!(
            state
                .foundation_acquisition_history("bob")
                .unwrap()
                .is_empty()
        );
        drop(state);
        drop(watchlist);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_foundation_acquisition_is_recovered_on_reload() {
        let (root, state, watchlist) = local_data_state("foundation-recovery");
        state
            .foundation_acquisition_start("alice", "crypto-foundation-1", "crypto", "okx")
            .unwrap();
        drop(state);

        let reloaded = LocalResearchState::open(&root).unwrap();
        let history = reloaded.foundation_acquisition_history("alice").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].state, "failed");
        assert_eq!(
            history[0].error.as_deref(),
            Some("operation interrupted by host restart")
        );
        drop(reloaded);
        drop(watchlist);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_okx_backfill_retains_its_exact_request_for_retry() {
        let (root, state, watchlist) = local_data_state("okx-backfill-retry");
        let request = adaq_data_pipeline::okx::OkxBackfillRequest {
            task_id: "crypto-foundation-1".into(),
            user_id: "alice".into(),
            start_time_ms: 10,
            end_time_ms: 20,
            interval: BarInterval::OneHour,
            instrument_codes: vec!["BTC-USDT".into()],
            universe_snapshot_id: Some("master-1".into()),
            checkpoint_operation_id: None,
            max_gap_retries: 2,
            publication_evidence_name: None,
        };
        state.foundation_okx_backfill_start(&request).unwrap();
        assert!(
            state
                .foundation_okx_backfill_request("alice", "crypto-foundation-1")
                .unwrap_err()
                .contains("only failed or cancelled")
        );
        drop(state);

        let reloaded = LocalResearchState::open(&root).unwrap();
        let retained = reloaded
            .foundation_okx_backfill_request("alice", "crypto-foundation-1")
            .unwrap();
        assert_eq!(retained, request);
        assert_eq!(
            reloaded.foundation_acquisition_history("alice").unwrap()[0].state,
            "failed"
        );
        drop(reloaded);
        drop(watchlist);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_source_publication_composes_snapshot_and_point_in_time_universe() {
        let (root, state, watchlist) = local_data_state("gate-two-composition");
        let user_id = "alice";
        let start_time_ms = 1_704_067_200_000;
        let end_time_ms = start_time_ms + 2 * 3_600_000;
        let venue = Venue::crypto_spot("okx").unwrap();
        let instrument = InstrumentId::new(venue.clone(), "BTC-USDT").unwrap();
        state
            .okx
            .record_instrument_master(
                user_id,
                InstrumentMasterAcquisition {
                    retrieved_at_ms: start_time_ms,
                    response_sha256: "master-response".into(),
                    connector_version: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
                    diagnostics: OkxRequestDiagnostics::default(),
                    instruments: vec![SpotInstrument {
                        src: "okx".into(),
                        code: "BTC-USDT".into(),
                        base_asset: "BTC".into(),
                        quote_asset: "USDT".into(),
                        status: InstrumentStatus::Live,
                        listing_time_ms: None,
                        continuous_trading_time_ms: None,
                        price_increment: Decimal::new(1, 1),
                        quantity_increment: Decimal::new(1, 4),
                        minimum_quantity: Decimal::new(1, 4),
                    }],
                    quote_volume_24h_usdt: Default::default(),
                },
            )
            .unwrap();
        let records = (0..2)
            .map(|offset| SourceMarketRecord {
                provider_symbol: "BTC-USDT".into(),
                instrument: instrument.clone(),
                interval: BarInterval::OneHour,
                open_time_ms: start_time_ms + offset * 3_600_000,
                open: Some("1".into()),
                high: Some("2".into()),
                low: Some("0.5".into()),
                close: Some("1.5".into()),
                base_volume: Some("1".into()),
                quote_volume: Some("1.5".into()),
                raw_payload: serde_json::Value::Null,
            })
            .collect();
        let publication = state
            .pipeline
            .publish(
                user_id,
                SourceAcquisition {
                    provider: "okx".into(),
                    actual_upstream: Some("OKX public history-candles REST".into()),
                    connector: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
                    connector_version: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
                    request_parameters: serde_json::json!({
                        "instrument": instrument,
                        "interval": "1h",
                        "startTimeMs": start_time_ms,
                        "endTimeMs": end_time_ms,
                    }),
                    retrieved_at_ms: end_time_ms,
                    response_sha256s: vec!["response".into()],
                    acquisition_content_sha256: None,
                    capability_snapshot: ProviderCapabilitySnapshot {
                        provider: "okx".into(),
                        captured_at_ms: end_time_ms,
                        venues: vec!["OKX Spot".into()],
                        record_types: vec!["candles".into()],
                        ..ProviderCapabilitySnapshot::default()
                    },
                    acquisition_diagnostics: AcquisitionDiagnostics {
                        request_count: 1,
                        response_statuses: vec![200],
                        ..AcquisitionDiagnostics::default()
                    },
                    price_basis: adaq_data_core::market::PriceBasis::Unadjusted,
                    records,
                },
                {
                    let mut request = CanonicalizationRequest::new(
                        InstrumentId::new(venue, "BTC-USDT").unwrap(),
                        BarInterval::OneHour,
                        CalendarEvidence::UtcGrid {
                            calendar_id: "okx-utc-grid".into(),
                            closures: Vec::new(),
                        },
                    )
                    .unwrap();
                    request.historical_range = Some(HistoricalBarRange {
                        start_time_ms,
                        end_time_ms,
                    });
                    request
                },
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            state
                .publish_okx_backfill(
                    user_id,
                    start_time_ms,
                    end_time_ms,
                    BarInterval::OneHour,
                    &["BTC-USDT".into()],
                    &cancelled,
                    std::slice::from_ref(&publication),
                    None,
                )
                .unwrap_err()
                .contains("cancelled")
        );
        let universe = state
            .publish_okx_backfill(
                user_id,
                start_time_ms,
                end_time_ms,
                BarInterval::OneHour,
                &["BTC-USDT".into()],
                &CancellationToken::new(),
                std::slice::from_ref(&publication),
                None,
            )
            .unwrap();

        assert_eq!(universe.components.len(), 1);
        assert_eq!(universe.universe.instruments.len(), 1);
        assert_eq!(
            universe.components[0].dataset.source_id,
            publication.source.source_id
        );
        let persisted = state
            .snapshots
            .universe_snapshot_for_user(user_id, &universe.snapshot_id)
            .unwrap();
        assert_eq!(persisted.snapshot_id, universe.snapshot_id);
        assert_eq!(persisted.components.len(), 1);
        drop(state);
        drop(watchlist);
        fs::remove_dir_all(root).unwrap();
    }
}
