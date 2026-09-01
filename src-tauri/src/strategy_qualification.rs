//! Gate 11 Strategy Qualification.
//!
//! A Qualification is an immutable, server-revalidated join of one exact
//! Candidate Revision, generated Strategy Component Package, Backtest Run,
//! Validation Protocol, and Validation Report. It never starts Gate 12.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_backtest_core::{
    ExecutionProfile, MarketDataSnapshot, MarketDataUniverseSnapshot, RiskPolicy,
};
use adaq_component_tooling::{
    ComponentKind, ComponentManifest, ComponentPackage, ComponentParameterValue, ComponentTemplate,
    FeatureSlotSource, QualificationAttempt, RunLimits, WasmLoader,
    build_project_offline_with_diagnostics, create_project, qualify_package_with_parameter_grid,
    verify_package,
};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    backtest::{
        BacktestRun, BacktestRunRequest, BacktestRunView, FactorInstanceRequest,
        PortfolioBacktestView, SignalInstanceRequest, StrategyQualificationBinding,
    },
    strategy_candidate::{StrategyCandidateRevision, StrategyInputBinding, StrategyValue},
    user::validate_user,
    validation::{
        ValidationProtocol, ValidationProtocolCreateRequest, ValidationReport,
        ValidationWindowRequest,
    },
};

pub(crate) const STRATEGY_QUALIFICATION_SCHEMA_VERSION: &str = "adaq:strategy-qualification@1";
const STRATEGY_GENERATOR_ID: &str = "adaq:strategy-rust-sdk-generator@1";
const STRATEGY_CANONICALIZATION_VERSION: &str = "adaq:canonical-json@1";
const STRATEGY_TARGET: &str = "wasm32-unknown-unknown";
const STRATEGY_BUILD_COMMANDS: &[&str] = &[
    "cargo generate-lockfile --offline",
    "cargo test --offline --locked",
    "rustup run stable cargo component build --offline --locked --release --target wasm32-unknown-unknown",
];
const MAX_DIAGNOSTIC_BYTES: usize = 512;

pub(crate) trait StrategyQualificationSource: Send + Sync {
    fn candidate_revision(
        &self,
        user_id: &str,
        candidate_id: &str,
        revision: u64,
    ) -> Result<(StrategyCandidateRevision, bool), String>;
    fn import_strategy_package(
        &self,
        user_id: &str,
        bytes: &[u8],
    ) -> Result<ComponentPackage, String>;
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String>;
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<adaq_data_core::OhlcvBar>), String>;
    fn universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<MarketDataUniverseSnapshot, String>;
    fn run_backtest(&self, request: BacktestRunRequest) -> Result<BacktestRunView, String>;
    fn load_backtest(&self, user_id: &str, run_id: &str) -> Result<BacktestRun, String>;
    fn run_portfolio_backtest(
        &self,
        request: BacktestRunRequest,
    ) -> Result<PortfolioBacktestView, String>;
    fn load_portfolio_backtest(
        &self,
        user_id: &str,
        run_id: &str,
    ) -> Result<PortfolioBacktestView, String>;
    fn create_protocol(
        &self,
        request: ValidationProtocolCreateRequest,
    ) -> Result<ValidationProtocol, String>;
    fn protocol_for_user(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<ValidationProtocol, String>;
    fn run_report(&self, user_id: &str, protocol_id: &str) -> Result<ValidationReport, String>;
    fn report_for_user(&self, user_id: &str, report_id: &str) -> Result<ValidationReport, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyWindow {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyEvaluationContext {
    pub snapshot_id: String,
    pub universe_snapshot_id: String,
    pub universe_id: String,
    pub selection_window: StrategyWindow,
    pub final_window: StrategyWindow,
    pub risk_policy: RiskPolicy,
    pub execution_profile: ExecutionProfile,
    pub signal_instances: Vec<SignalInstanceRequest>,
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_quote_allocation: Decimal,
    pub seed: u64,
    pub validation_method_version: String,
    pub aggregation_rule_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrategyQualificationAttemptStatus {
    Running,
    Failed,
    ReadyForReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyQualificationDiagnostic {
    pub stage: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyPackageProvenance {
    pub schema_version: String,
    pub generator_id: String,
    pub sdk_version: String,
    pub abi_version: String,
    pub toolchain: String,
    pub compiler: String,
    pub target: String,
    pub canonicalization_version: String,
    pub canonicalization_sha256: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub candidate_revision_hash: String,
    pub source_definition_sha256: String,
    pub generated_source_sha256: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub parameter_grid: Vec<BTreeMap<String, String>>,
    pub qualification: QualificationAttempt,
    pub diagnostic_log_sha256: String,
    pub commands: Vec<String>,
    pub package_provenance_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyQualificationAttempt {
    pub attempt_id: String,
    pub user_id: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub candidate_revision_hash: String,
    pub status: StrategyQualificationAttemptStatus,
    pub package: Option<StrategyPackageProvenance>,
    pub context: StrategyEvaluationContext,
    pub backtest_run_id: Option<String>,
    pub validation_protocol_id: Option<String>,
    pub validation_report_id: Option<String>,
    pub diagnostics: Vec<StrategyQualificationDiagnostic>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyQualificationRunRequest {
    pub user_id: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub snapshot_id: String,
    pub universe_snapshot_id: String,
    pub selection_window: StrategyWindow,
    pub final_window: StrategyWindow,
    pub signal_instances: Vec<SignalInstanceRequest>,
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_quote_allocation: Decimal,
    pub execution_profile: ExecutionProfile,
    pub risk_policy: RiskPolicy,
    pub seed: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyQualificationAttemptRequest {
    pub attempt_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyQualification {
    pub qualification_id: String,
    pub attempt_id: String,
    pub user_id: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub candidate_revision_hash: String,
    pub package: StrategyPackageProvenance,
    pub context: StrategyEvaluationContext,
    pub backtest_run_id: String,
    pub validation_protocol_id: String,
    pub validation_report_id: String,
    pub gate12_eligible: bool,
    pub gate12_continuation_required: bool,
    pub evidence_hash: String,
    pub reviewed_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyQualificationIdRequest {
    pub qualification_id: String,
}

#[derive(Debug)]
struct EvaluationFailure {
    stage: &'static str,
    message: String,
}

impl EvaluationFailure {
    fn new(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct StrategyQualificationStore {
    database: Arc<Mutex<Connection>>,
    source: Arc<dyn StrategyQualificationSource>,
}

impl StrategyQualificationStore {
    pub(crate) fn open(
        database: Arc<Mutex<Connection>>,
        source: Arc<dyn StrategyQualificationSource>,
    ) -> Result<Self, String> {
        database
            .lock()
            .map_err(|_| "Strategy Qualification database lock poisoned".to_owned())?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS strategy_qualification_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    candidate_id TEXT NOT NULL,
                    candidate_revision INTEGER NOT NULL,
                    attempt_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS strategy_qualifications (
                    qualification_id TEXT PRIMARY KEY,
                    attempt_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    candidate_id TEXT NOT NULL,
                    candidate_revision INTEGER NOT NULL,
                    qualification_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS strategy_qualification_attempts_user_idx
                    ON strategy_qualification_attempts(user_id, created_at_ms DESC);
                 CREATE INDEX IF NOT EXISTS strategy_qualifications_user_idx
                    ON strategy_qualifications(user_id, created_at_ms DESC);",
            )
            .map_err(string)?;
        let store = Self { database, source };
        store.recover_interrupted_attempts()?;
        Ok(store)
    }

    pub(crate) fn run(
        &self,
        request: StrategyQualificationRunRequest,
    ) -> Result<StrategyQualificationAttempt, String> {
        validate_user(&request.user_id)?;
        let now = unix_now_ms();
        let mut attempt = StrategyQualificationAttempt {
            attempt_id: Uuid::new_v4().to_string(),
            user_id: request.user_id.clone(),
            candidate_id: request.candidate_id.clone(),
            candidate_revision: request.candidate_revision,
            candidate_revision_hash: String::new(),
            status: StrategyQualificationAttemptStatus::Running,
            package: None,
            context: context_from_request(&request),
            backtest_run_id: None,
            validation_protocol_id: None,
            validation_report_id: None,
            diagnostics: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.save_attempt(&attempt)?;
        match self.evaluate(&request) {
            Ok(result) => {
                attempt.candidate_revision_hash = result.candidate_revision_hash;
                attempt.package = Some(result.package);
                attempt.context = result.context;
                attempt.backtest_run_id = Some(result.backtest_run_id);
                attempt.validation_protocol_id = Some(result.validation_protocol_id);
                attempt.validation_report_id = Some(result.validation_report_id);
                attempt.status = StrategyQualificationAttemptStatus::ReadyForReview;
            }
            Err(error) => {
                attempt.status = StrategyQualificationAttemptStatus::Failed;
                attempt.diagnostics.push(StrategyQualificationDiagnostic {
                    stage: error.stage.into(),
                    code: format!("strategy-qualification-{}", error.stage),
                    message: bounded(&error.message),
                });
            }
        }
        attempt.updated_at_ms = unix_now_ms();
        self.save_attempt(&attempt)?;
        Ok(attempt)
    }

    pub(crate) fn qualify(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<StrategyQualification, String> {
        validate_user(user_id)?;
        let attempt = self.attempt_for_user(user_id, attempt_id)?;
        if attempt.status != StrategyQualificationAttemptStatus::ReadyForReview {
            return Err("Strategy Qualification requires a ready immutable Attempt".into());
        }
        let (revision, eligible) = self.source.candidate_revision(
            user_id,
            &attempt.candidate_id,
            attempt.candidate_revision,
        )?;
        revision.validate()?;
        if !eligible
            || revision.revision_hash != attempt.candidate_revision_hash
            || revision.candidate_id != attempt.candidate_id
        {
            return Err("Strategy Candidate Revision is stale or no longer eligible".into());
        }
        let package_provenance = attempt
            .package
            .clone()
            .ok_or("Strategy Qualification Attempt has no Package provenance")?;
        let package = self
            .source
            .package_for_user(user_id, &package_provenance.package_archive_sha256)?;
        if package.archive_sha256 != package_provenance.package_archive_sha256
            || package.manifest.kind != ComponentKind::Strategy
            || package.manifest.wasm_sha256 != package_provenance.package_wasm_sha256
            || matches!(
                revision.scope,
                crate::strategy_candidate::StrategyScope::Portfolio
            ) != matches!(
                package.manifest.strategy_scope,
                adaq_component_tooling::StrategyScope::Portfolio
            )
            || manifest_default_parameters(&package.manifest) != package_provenance.parameters
        {
            return Err(
                "Strategy Package provenance no longer matches the entitled Package".into(),
            );
        }
        if package_provenance_hash(&package_provenance)?
            != package_provenance.package_provenance_hash
        {
            return Err("Strategy Package provenance identity is invalid".into());
        }
        let is_portfolio = matches!(
            revision.scope,
            crate::strategy_candidate::StrategyScope::Portfolio
        );
        let backtest_run_id = attempt
            .backtest_run_id
            .as_deref()
            .ok_or("Strategy Qualification Attempt has no Backtest Run")?;
        let expected_binding = qualification_binding(&attempt, &package_provenance);
        if is_portfolio {
            let run = self
                .source
                .load_portfolio_backtest(user_id, backtest_run_id)?;
            validate_portfolio_run_binding(
                &run,
                &expected_binding,
                &attempt.context,
                &package_provenance.package_archive_sha256,
                &package_provenance.parameters,
            )?;
        } else {
            let run = self.source.load_backtest(user_id, backtest_run_id)?;
            validate_run_binding(
                &run,
                &expected_binding,
                &attempt.context,
                &package_provenance.package_archive_sha256,
                &package_provenance.parameters,
            )?;
        }
        let protocol_id = attempt
            .validation_protocol_id
            .as_deref()
            .ok_or("Strategy Qualification Attempt has no Validation Protocol")?;
        let protocol = self.source.protocol_for_user(user_id, protocol_id)?;
        validate_protocol_binding(
            &protocol,
            &expected_binding,
            &attempt.context,
            &package_provenance.package_archive_sha256,
            &package_provenance.parameters,
            is_portfolio,
        )?;
        let report_id = attempt
            .validation_report_id
            .as_deref()
            .ok_or("Strategy Qualification Attempt has no Validation Report")?;
        let report = self.source.report_for_user(user_id, report_id)?;
        validate_report_binding(&report, &protocol, &expected_binding, &attempt.context)?;
        for window in &report.windows {
            if let Some(run_id) = &window.sample_in_run_id {
                if is_portfolio {
                    validate_portfolio_run_binding_for_window(
                        &self.source.load_portfolio_backtest(user_id, run_id)?,
                        &expected_binding,
                        &attempt.context,
                        &package_provenance.package_archive_sha256,
                        &package_provenance.parameters,
                        window.sample_in_start_time_ms,
                        window.sample_in_end_time_ms,
                    )?;
                } else {
                    validate_run_binding_for_window(
                        &self.source.load_backtest(user_id, run_id)?,
                        &expected_binding,
                        &attempt.context,
                        &package_provenance.package_archive_sha256,
                        &package_provenance.parameters,
                        &window.sample_in_snapshot_id,
                        window.sample_in_start_time_ms,
                        window.sample_in_end_time_ms,
                    )?;
                }
            }
            if let Some(run_id) = &window.sample_out_run_id {
                if is_portfolio {
                    validate_portfolio_run_binding_for_window(
                        &self.source.load_portfolio_backtest(user_id, run_id)?,
                        &expected_binding,
                        &attempt.context,
                        &package_provenance.package_archive_sha256,
                        &package_provenance.parameters,
                        Some(window.sample_out_start_time_ms),
                        window.sample_out_end_time_ms,
                    )?;
                } else {
                    validate_run_binding_for_window(
                        &self.source.load_backtest(user_id, run_id)?,
                        &expected_binding,
                        &attempt.context,
                        &package_provenance.package_archive_sha256,
                        &package_provenance.parameters,
                        &window.sample_out_snapshot_id,
                        Some(window.sample_out_start_time_ms),
                        window.sample_out_end_time_ms,
                    )?;
                }
            }
        }
        let mut qualification = StrategyQualification {
            qualification_id: String::new(),
            attempt_id: attempt.attempt_id.clone(),
            user_id: user_id.into(),
            candidate_id: attempt.candidate_id.clone(),
            candidate_revision: attempt.candidate_revision,
            candidate_revision_hash: attempt.candidate_revision_hash.clone(),
            package: package_provenance,
            context: attempt.context.clone(),
            backtest_run_id: backtest_run_id.into(),
            validation_protocol_id: protocol.protocol_id,
            validation_report_id: report.report_id,
            gate12_eligible: true,
            // This explicit reviewed action is the continuation approval for
            // the immutable qualification evidence; deployment still checks
            // the persisted flag and every exact upstream identity.
            gate12_continuation_required: false,
            evidence_hash: String::new(),
            reviewed_at_ms: unix_now_ms(),
        };
        qualification.evidence_hash = qualification_hash(&qualification)?;
        qualification.qualification_id = qualification.evidence_hash.clone();
        self.save_qualification(&qualification)?;
        Ok(qualification)
    }

    pub(crate) fn attempt_list(
        &self,
        user_id: &str,
    ) -> Result<Vec<StrategyQualificationAttempt>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT attempt_json FROM strategy_qualification_attempts
                 WHERE user_id = ?1 ORDER BY created_at_ms DESC, attempt_id DESC",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    pub(crate) fn attempt_for_user(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<StrategyQualificationAttempt, String> {
        validate_user(user_id)?;
        let json: String = self
            .database
            .lock()
            .map_err(string)?
            .query_row(
                "SELECT attempt_json FROM strategy_qualification_attempts
                 WHERE user_id = ?1 AND attempt_id = ?2",
                params![user_id, attempt_id],
                |row| row.get(0),
            )
            .map_err(|_| "Strategy Qualification Attempt was not found".to_owned())?;
        serde_json::from_str(&json).map_err(string)
    }

    pub(crate) fn qualification_list(
        &self,
        user_id: &str,
    ) -> Result<Vec<StrategyQualification>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT qualification_json FROM strategy_qualifications
                 WHERE user_id = ?1 ORDER BY created_at_ms DESC, qualification_id DESC",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    pub(crate) fn qualification_for_user(
        &self,
        user_id: &str,
        qualification_id: &str,
    ) -> Result<StrategyQualification, String> {
        validate_user(user_id)?;
        let json: String = self
            .database
            .lock()
            .map_err(string)?
            .query_row(
                "SELECT qualification_json FROM strategy_qualifications
                 WHERE user_id = ?1 AND qualification_id = ?2",
                params![user_id, qualification_id],
                |row| row.get(0),
            )
            .map_err(|_| "Strategy Qualification was not found".to_owned())?;
        serde_json::from_str(&json).map_err(string)
    }

    pub(crate) fn reset_user(&self, user_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        self.database
            .lock()
            .map_err(string)?
            .execute(
                "DELETE FROM strategy_qualifications WHERE user_id = ?1;
                 DELETE FROM strategy_qualification_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        Ok(())
    }

    fn save_attempt(&self, attempt: &StrategyQualificationAttempt) -> Result<(), String> {
        self.database
            .lock()
            .map_err(string)?
            .execute(
                "INSERT OR REPLACE INTO strategy_qualification_attempts
                    (attempt_id, user_id, candidate_id, candidate_revision, attempt_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    attempt.attempt_id,
                    attempt.user_id,
                    attempt.candidate_id,
                    i64::try_from(attempt.candidate_revision).map_err(string)?,
                    serde_json::to_string(attempt).map_err(string)?,
                    attempt.created_at_ms,
                ],
            )
            .map_err(string)?;
        Ok(())
    }

    fn recover_interrupted_attempts(&self) -> Result<(), String> {
        let attempts = {
            let database = self.database.lock().map_err(string)?;
            let mut statement = database
                .prepare(
                    "SELECT attempt_json FROM strategy_qualification_attempts
                     WHERE json_extract(attempt_json, '$.status') = 'running'",
                )
                .map_err(string)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(string)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(string)?
        };
        for json in attempts {
            let mut attempt: StrategyQualificationAttempt =
                serde_json::from_str(&json).map_err(string)?;
            mark_interrupted_attempt(&mut attempt, unix_now_ms());
            self.save_attempt(&attempt)?;
        }
        Ok(())
    }

    fn save_qualification(&self, qualification: &StrategyQualification) -> Result<(), String> {
        self.database
            .lock()
            .map_err(string)?
            .execute(
                "INSERT INTO strategy_qualifications
                    (qualification_id, attempt_id, user_id, candidate_id, candidate_revision,
                     qualification_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    qualification.qualification_id,
                    qualification.attempt_id,
                    qualification.user_id,
                    qualification.candidate_id,
                    i64::try_from(qualification.candidate_revision).map_err(string)?,
                    serde_json::to_string(qualification).map_err(string)?,
                    qualification.reviewed_at_ms,
                ],
            )
            .map_err(|error| {
                if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
                    format!("Strategy Qualification identity already exists: {error}")
                } else {
                    error.to_string()
                }
            })?;
        Ok(())
    }

    fn evaluate(
        &self,
        request: &StrategyQualificationRunRequest,
    ) -> Result<EvaluationResult, EvaluationFailure> {
        validate_request_shape(request)?;
        let (revision, eligible) = self
            .source
            .candidate_revision(
                &request.user_id,
                &request.candidate_id,
                request.candidate_revision,
            )
            .map_err(|error| EvaluationFailure::new("candidate", error))?;
        if !eligible {
            return Err(EvaluationFailure::new(
                "candidate",
                "Strategy Candidate Revision is draft, stale, superseded, or no longer accepted",
            ));
        }
        if revision.candidate_id != request.candidate_id
            || revision.revision != request.candidate_revision
        {
            return Err(EvaluationFailure::new(
                "candidate",
                "Strategy Candidate Revision identity does not match the request",
            ));
        }
        revision
            .validate()
            .map_err(|error| EvaluationFailure::new("candidate", error))?;
        let (snapshot, bars) = self
            .source
            .snapshot_for_user(&request.user_id, &request.snapshot_id)
            .map_err(|error| EvaluationFailure::new("context", error))?;
        let universe = self
            .source
            .universe_snapshot_for_user(&request.user_id, &request.universe_snapshot_id)
            .map_err(|error| EvaluationFailure::new("context", error))?;
        validate_context(&revision, request, &snapshot, &universe, &bars)?;
        let mut context = context_from_request(request);
        context.universe_id = universe.universe.universe_id.clone();
        let generated = generate_strategy_package(&revision)
            .map_err(|error| EvaluationFailure::new("generator", error))?;
        let package = self
            .source
            .import_strategy_package(&request.user_id, &generated.package_bytes)
            .map_err(|error| EvaluationFailure::new("package", error))?;
        if package.archive_sha256 != generated.provenance.package_archive_sha256
            || package.manifest.wasm_sha256 != generated.provenance.package_wasm_sha256
            || matches!(
                revision.scope,
                crate::strategy_candidate::StrategyScope::Portfolio
            ) != matches!(
                package.manifest.strategy_scope,
                adaq_component_tooling::StrategyScope::Portfolio
            )
        {
            return Err(EvaluationFailure::new(
                "package",
                "imported Strategy Package identity differs from generated provenance",
            ));
        }
        let binding = StrategyQualificationBinding {
            candidate_id: revision.candidate_id.clone(),
            candidate_revision: revision.revision,
            candidate_revision_hash: revision.revision_hash.clone(),
            package_provenance_hash: generated.provenance.package_provenance_hash.clone(),
        };
        let factor_instances = revision
            .definition
            .input_slots
            .iter()
            .filter_map(|slot| match &slot.binding {
                StrategyInputBinding::Factor(binding) => Some(FactorInstanceRequest {
                    alias: slot.alias.clone(),
                    archive_sha256: binding.package_archive_sha256.clone(),
                    parameters: HashMap::new(),
                }),
                StrategyInputBinding::Model(_) => None,
            })
            .collect::<Vec<_>>();
        let is_portfolio = matches!(
            revision.scope,
            crate::strategy_candidate::StrategyScope::Portfolio
        );
        validate_signal_instances(&revision, &request.signal_instances, is_portfolio)?;
        let strategy_parameters = generated.parameters.into_iter().collect::<HashMap<_, _>>();
        let run_request = BacktestRunRequest {
            user_id: request.user_id.clone(),
            snapshot_id: snapshot.snapshot_id.clone(),
            portfolio_universe_snapshot_id: is_portfolio
                .then(|| request.universe_snapshot_id.clone()),
            run_start_time_ms: Some(request.selection_window.start_time_ms),
            run_end_time_ms: Some(request.final_window.end_time_ms),
            factor_instances,
            signal_instances: request.signal_instances.clone(),
            strategy_archive_sha256: package.archive_sha256.clone(),
            strategy_parameters,
            initial_quote_allocation: request.initial_quote_allocation,
            execution_profile: request.execution_profile.clone(),
            strategy_binding: Some(binding.clone()),
            risk_policy: Some(request.risk_policy.clone()),
            seed: request.seed,
        };
        let backtest_run_id = if is_portfolio {
            let run = self
                .source
                .run_portfolio_backtest(run_request.clone())
                .map_err(|error| EvaluationFailure::new("backtest", error))?;
            let stored_run = self
                .source
                .load_portfolio_backtest(&request.user_id, &run.run_id)
                .map_err(|error| EvaluationFailure::new("backtest", error))?;
            validate_portfolio_run_binding(
                &stored_run,
                &binding,
                &context,
                &generated.provenance.package_archive_sha256,
                &generated.provenance.parameters,
            )
            .map_err(|error| EvaluationFailure::new("backtest", error))?;
            run.run_id
        } else {
            let run = self
                .source
                .run_backtest(run_request.clone())
                .map_err(|error| EvaluationFailure::new("backtest", error))?;
            let stored_run = self
                .source
                .load_backtest(&request.user_id, &run.run_id)
                .map_err(|error| EvaluationFailure::new("backtest", error))?;
            validate_run_binding(
                &stored_run,
                &binding,
                &context,
                &generated.provenance.package_archive_sha256,
                &generated.provenance.parameters,
            )
            .map_err(|error| EvaluationFailure::new("backtest", error))?;
            run.run_id
        };
        let protocol = self
            .source
            .create_protocol(ValidationProtocolCreateRequest {
                user_id: request.user_id.clone(),
                run: run_request,
                windows: vec![ValidationWindowRequest {
                    snapshot_id: snapshot.snapshot_id,
                    sample_out_start_time_ms: request.final_window.start_time_ms,
                    sample_out_end_time_ms: Some(request.final_window.end_time_ms),
                    sample_in_start_time_ms: Some(request.selection_window.start_time_ms),
                    sample_in_end_time_ms: Some(request.selection_window.end_time_ms),
                }],
                walk_forward: None,
                cross_market: None,
                method_version: "chronological-holdout@1".into(),
                aggregation_rule_version: "equal-window@1".into(),
                strategy_binding: Some(binding.clone()),
                final_evidence_sealed: true,
            })
            .map_err(|error| EvaluationFailure::new("validation-protocol", error))?;
        let report = self
            .source
            .run_report(&request.user_id, &protocol.protocol_id)
            .map_err(|error| EvaluationFailure::new("validation-report", error))?;
        validate_protocol_binding(
            &protocol,
            &binding,
            &context,
            &generated.provenance.package_archive_sha256,
            &generated.provenance.parameters,
            is_portfolio,
        )
        .map_err(|error| EvaluationFailure::new("validation-report", error))?;
        validate_report_binding(&report, &protocol, &binding, &context)
            .map_err(|error| EvaluationFailure::new("validation-report", error))?;
        Ok(EvaluationResult {
            candidate_revision_hash: revision.revision_hash,
            package: generated.provenance,
            backtest_run_id,
            validation_protocol_id: protocol.protocol_id,
            validation_report_id: report.report_id,
            context,
        })
    }
}

struct EvaluationResult {
    candidate_revision_hash: String,
    package: StrategyPackageProvenance,
    backtest_run_id: String,
    validation_protocol_id: String,
    validation_report_id: String,
    context: StrategyEvaluationContext,
}

fn context_from_request(request: &StrategyQualificationRunRequest) -> StrategyEvaluationContext {
    StrategyEvaluationContext {
        snapshot_id: request.snapshot_id.clone(),
        universe_snapshot_id: request.universe_snapshot_id.clone(),
        universe_id: String::new(),
        selection_window: request.selection_window.clone(),
        final_window: request.final_window.clone(),
        risk_policy: request.risk_policy.clone(),
        execution_profile: request.execution_profile.clone(),
        signal_instances: request.signal_instances.clone(),
        initial_quote_allocation: request.initial_quote_allocation,
        seed: request.seed,
        validation_method_version: "chronological-holdout@1".into(),
        aggregation_rule_version: "equal-window@1".into(),
    }
}

fn validate_request_shape(
    request: &StrategyQualificationRunRequest,
) -> Result<(), EvaluationFailure> {
    if request.candidate_id.trim().is_empty()
        || request.candidate_revision == 0
        || request.snapshot_id.trim().is_empty()
        || request.universe_snapshot_id.trim().is_empty()
        || request.selection_window.start_time_ms > request.selection_window.end_time_ms
        || request.final_window.start_time_ms > request.final_window.end_time_ms
        || request.selection_window.end_time_ms >= request.final_window.start_time_ms
        || request.initial_quote_allocation <= Decimal::ZERO
        || request.risk_policy.policy_id.trim().is_empty()
        || request.risk_policy.max_instrument_weight <= Decimal::ZERO
        || request.risk_policy.max_instrument_weight > Decimal::ONE
        || request
            .risk_policy
            .max_turnover
            .is_some_and(|turnover| turnover < Decimal::ZERO)
    {
        return Err(EvaluationFailure::new(
            "context",
            "Strategy Qualification context is invalid or Selection and Final overlap",
        ));
    }
    Ok(())
}

fn validate_context(
    revision: &StrategyCandidateRevision,
    request: &StrategyQualificationRunRequest,
    snapshot: &MarketDataSnapshot,
    universe: &MarketDataUniverseSnapshot,
    bars: &[adaq_data_core::OhlcvBar],
) -> Result<(), EvaluationFailure> {
    if revision.semantic_context.snapshot_id != snapshot.snapshot_id
        || revision.semantic_context.universe_id != universe.snapshot_id
        || universe.universe.evidence_state == "unknown"
        || universe.universe.evidence_reasons.is_empty()
        || snapshot.interval != universe.interval
        || request.selection_window.start_time_ms < snapshot.start_time_ms
        || request.final_window.end_time_ms > snapshot.end_time_ms
        || request.selection_window.start_time_ms < universe.start_time_ms
        || request.final_window.end_time_ms > universe.end_time_ms
        || !exact_bar_boundary(bars, request.selection_window.start_time_ms)
        || !exact_bar_boundary(bars, request.selection_window.end_time_ms)
        || !exact_bar_boundary(bars, request.final_window.start_time_ms)
        || !exact_bar_boundary(bars, request.final_window.end_time_ms)
    {
        return Err(EvaluationFailure::new(
            "context",
            "Candidate, Snapshot, Universe, and closed evaluation windows are not exact",
        ));
    }
    Ok(())
}

fn exact_bar_boundary(bars: &[adaq_data_core::OhlcvBar], boundary: i64) -> bool {
    bars.iter().any(|bar| bar.open_time_ms == boundary)
}

fn validate_signal_instances(
    revision: &StrategyCandidateRevision,
    signals: &[SignalInstanceRequest],
    portfolio: bool,
) -> Result<(), EvaluationFailure> {
    let expected = revision
        .definition
        .input_slots
        .iter()
        .filter_map(|slot| match &slot.binding {
            StrategyInputBinding::Model(binding) => {
                Some((slot.alias.as_str(), binding.output_name.as_str()))
            }
            StrategyInputBinding::Factor(_) => None,
        })
        .collect::<Vec<_>>();
    let invalid_single = expected.len() != signals.len()
        || signals.iter().any(|signal| {
            expected
                .iter()
                .filter(|(slot, output)| *slot == signal.slot && *output == signal.signal_name)
                .count()
                != 1
        })
        || signals
            .iter()
            .map(|signal| signal.slot.as_str())
            .collect::<HashSet<_>>()
            .len()
            != signals.len();
    let invalid_portfolio = expected
        .is_empty()
        .then_some(!signals.is_empty())
        .unwrap_or_else(|| {
            signals.len() % expected.len() != 0
                || signals.is_empty()
                || signals.iter().any(|signal| {
                    expected
                        .iter()
                        .filter(|(slot, output)| {
                            *slot == signal.slot && *output == signal.signal_name
                        })
                        .count()
                        != 1
                })
                || signals
                    .iter()
                    .map(|signal| (signal.slot.as_str(), signal.dataset_id.as_str()))
                    .collect::<HashSet<_>>()
                    .len()
                    != signals.len()
        });
    if if portfolio {
        invalid_portfolio
    } else {
        invalid_single
    } {
        return Err(EvaluationFailure::new(
            "context",
            "Forecast Signal bindings must match every accepted Model input exactly",
        ));
    }
    Ok(())
}

fn qualification_binding(
    attempt: &StrategyQualificationAttempt,
    package: &StrategyPackageProvenance,
) -> StrategyQualificationBinding {
    StrategyQualificationBinding {
        candidate_id: attempt.candidate_id.clone(),
        candidate_revision: attempt.candidate_revision,
        candidate_revision_hash: attempt.candidate_revision_hash.clone(),
        package_provenance_hash: package.package_provenance_hash.clone(),
    }
}

fn validate_run_binding(
    run: &BacktestRun,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
    package_archive_sha256: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<(), String> {
    validate_run_binding_for_window(
        run,
        binding,
        context,
        package_archive_sha256,
        parameters,
        &context.snapshot_id,
        Some(context.selection_window.start_time_ms),
        Some(context.final_window.end_time_ms),
    )
}

fn validate_portfolio_run_binding(
    run: &PortfolioBacktestView,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
    package_archive_sha256: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<(), String> {
    validate_portfolio_run_binding_for_window(
        run,
        binding,
        context,
        package_archive_sha256,
        parameters,
        Some(context.selection_window.start_time_ms),
        Some(context.final_window.end_time_ms),
    )
}

fn validate_portfolio_run_binding_for_window(
    run: &PortfolioBacktestView,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
    package_archive_sha256: &str,
    parameters: &BTreeMap<String, String>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
) -> Result<(), String> {
    let start_time_ms = start_time_ms.ok_or("Portfolio Backtest window has no start time")?;
    let end_time_ms = end_time_ms.ok_or("Portfolio Backtest window has no end time")?;
    let provenance = run
        .provenance
        .as_ref()
        .ok_or("Portfolio Backtest has no immutable provenance")?;
    let expected_component_archives = std::iter::once(package_archive_sha256.to_owned())
        .chain(
            provenance
                .factor_instances
                .iter()
                .map(|factor| factor.archive_sha256.clone()),
        )
        .collect::<Vec<_>>();
    let actual_component_archives = provenance
        .component_lock
        .iter()
        .map(|component| component.archive_sha256.clone())
        .collect::<Vec<_>>();
    let locked_signals = provenance
        .signal_locks
        .iter()
        .map(|signal| SignalInstanceRequest {
            slot: signal.slot.clone(),
            dataset_id: signal.dataset_id.clone(),
            signal_name: signal.signal_name.clone(),
        })
        .collect::<Vec<_>>();
    if run.metrics.is_none()
        || run.evidence.metrics.is_none()
        || run.metrics != run.evidence.metrics
        || run.evidence.initial_capital != context.initial_quote_allocation
        || run.evidence.decisions.is_empty()
        || provenance.strategy_binding.as_ref() != Some(binding)
        || provenance.risk_policy.as_ref() != Some(&context.risk_policy)
        || provenance.snapshot_id != context.snapshot_id
        || provenance.universe_snapshot_id != context.universe_snapshot_id
        || provenance.universe_id != context.universe_id
        || provenance.run_start_time_ms != start_time_ms
        || provenance.run_end_time_ms != end_time_ms
        || provenance.strategy_archive_sha256 != package_archive_sha256
        || provenance.strategy_parameters != *parameters
        || !same_signal_instances(&provenance.signal_instances, &context.signal_instances)
        || provenance.initial_quote_allocation != context.initial_quote_allocation
        || provenance.execution_profile != context.execution_profile
        || provenance.seed != context.seed
        || provenance.feature_plans.is_empty()
        || provenance.feature_plan_hash.len() != 64
        || provenance.strategy_wasm_sha256.len() != 64
        || actual_component_archives != expected_component_archives
        || !same_signal_instances(&locked_signals, &provenance.signal_instances)
    {
        return Err(
            "Portfolio Backtest does not match the exact Qualification identity or context".into(),
        );
    }
    Ok(())
}

fn same_signal_instances(left: &[SignalInstanceRequest], right: &[SignalInstanceRequest]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_by(|left, right| {
        (
            left.slot.as_str(),
            left.dataset_id.as_str(),
            left.signal_name.as_str(),
        )
            .cmp(&(
                right.slot.as_str(),
                right.dataset_id.as_str(),
                right.signal_name.as_str(),
            ))
    });
    right.sort_by(|left, right| {
        (
            left.slot.as_str(),
            left.dataset_id.as_str(),
            left.signal_name.as_str(),
        )
            .cmp(&(
                right.slot.as_str(),
                right.dataset_id.as_str(),
                right.signal_name.as_str(),
            ))
    });
    left == right
}

fn validate_run_binding_for_window(
    run: &BacktestRun,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
    package_archive_sha256: &str,
    parameters: &BTreeMap<String, String>,
    snapshot_id: &str,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
) -> Result<(), String> {
    let provenance = run
        .provenance
        .as_ref()
        .ok_or("Backtest Run has no immutable provenance")?;
    if provenance.strategy_binding.as_ref() != Some(binding)
        || provenance.normalized_request.strategy_binding.as_ref() != Some(binding)
        || provenance.risk_policy.as_ref() != Some(&context.risk_policy)
        || provenance.normalized_request.risk_policy.as_ref() != Some(&context.risk_policy)
        || provenance.normalized_request.snapshot_id != snapshot_id
        || run.snapshot.snapshot_id != snapshot_id
        || provenance.normalized_request.run_start_time_ms != start_time_ms
        || provenance.normalized_request.run_end_time_ms != end_time_ms
        || provenance.normalized_request.strategy_archive_sha256 != package_archive_sha256
        || provenance.normalized_request.strategy_parameters != *parameters
        || provenance.normalized_request.signal_instances != context.signal_instances
        || provenance.normalized_request.initial_quote_allocation
            != context.initial_quote_allocation
        || provenance.normalized_request.execution_profile != context.execution_profile
        || provenance.normalized_request.seed != context.seed
    {
        return Err(
            "Backtest Run does not match the exact Qualification identity or context".into(),
        );
    }
    Ok(())
}

fn validate_protocol_binding(
    protocol: &ValidationProtocol,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
    package_archive_sha256: &str,
    parameters: &BTreeMap<String, String>,
    portfolio: bool,
) -> Result<(), String> {
    if protocol.strategy_binding.as_ref() != Some(binding)
        || protocol.run.strategy_binding.as_ref() != Some(binding)
        || protocol.run.risk_policy.as_ref() != Some(&context.risk_policy)
        || protocol.run.snapshot_id != context.snapshot_id
        || protocol.run.portfolio_universe_snapshot_id.as_deref()
            != portfolio.then_some(context.universe_snapshot_id.as_str())
        || protocol.run.strategy_archive_sha256 != package_archive_sha256
        || !strategy_parameters_match(&protocol.run.strategy_parameters, parameters)
        || protocol.run.signal_instances != context.signal_instances
        || protocol.run.run_start_time_ms != Some(context.selection_window.start_time_ms)
        || protocol.run.run_end_time_ms != Some(context.final_window.end_time_ms)
        || protocol.run.initial_quote_allocation != context.initial_quote_allocation
        || protocol.run.execution_profile != context.execution_profile
        || protocol.method_version != context.validation_method_version
        || protocol.aggregation_rule_version != context.aggregation_rule_version
        || !protocol.final_evidence_sealed
        || protocol.windows.len() != 1
    {
        return Err("Validation Protocol does not match the exact Qualification context".into());
    }
    let window = &protocol.windows[0];
    if window.snapshot_id != context.snapshot_id
        || window.sample_in_start_time_ms != Some(context.selection_window.start_time_ms)
        || window.sample_in_end_time_ms != Some(context.selection_window.end_time_ms)
        || window.sample_out_start_time_ms != context.final_window.start_time_ms
        || window.sample_out_end_time_ms != Some(context.final_window.end_time_ms)
    {
        return Err("Validation Protocol Selection and Final windows are not exact".into());
    }
    Ok(())
}

fn strategy_parameters_match(
    actual: &HashMap<String, String>,
    expected: &BTreeMap<String, String>,
) -> bool {
    actual.len() == expected.len()
        && expected
            .iter()
            .all(|(name, value)| actual.get(name) == Some(value))
}

fn manifest_default_parameters(manifest: &ComponentManifest) -> BTreeMap<String, String> {
    manifest
        .parameters
        .iter()
        .map(|parameter| (parameter.name.clone(), parameter.default_value.clone()))
        .collect()
}

fn strategy_parameter_grid(
    revision: &StrategyCandidateRevision,
) -> Result<Vec<BTreeMap<String, String>>, String> {
    let mut combinations = vec![BTreeMap::new()];
    for node in &revision.definition.nodes {
        for (name, value) in &node.parameters {
            let parameter_name = format!("{}-{}", node.node_id, name);
            let (_, default_value, allowed_values) =
                strategy_value_manifest(&node.operation, name, value)?;
            let values = if allowed_values.is_empty() {
                vec![default_value]
            } else {
                allowed_values
            };
            let next_len = combinations
                .len()
                .checked_mul(values.len())
                .ok_or("Strategy parameter grid size overflowed")?;
            if next_len > 256 {
                return Err("Strategy parameter grid exceeds the 256-combination limit".into());
            }
            combinations = combinations
                .into_iter()
                .flat_map(|combination| {
                    let parameter_name = parameter_name.clone();
                    values.iter().map(move |value| {
                        let mut next = combination.clone();
                        next.insert(parameter_name.clone(), value.clone());
                        next
                    })
                })
                .collect();
        }
    }
    Ok(combinations)
}

fn validate_report_binding(
    report: &ValidationReport,
    protocol: &ValidationProtocol,
    binding: &StrategyQualificationBinding,
    context: &StrategyEvaluationContext,
) -> Result<(), String> {
    if report.protocol_id != protocol.protocol_id
        || report.strategy_binding.as_ref() != Some(binding)
        || !report.final_evidence_sealed
        || report.method_version != context.validation_method_version
        || report.aggregation_rule_version != context.aggregation_rule_version
        || report.windows.len() != 1
        || report.aggregate.completed_windows != 1
        || report.aggregate.failed_windows != 0
    {
        return Err("Validation Report is incomplete, unsealed, or identity-mismatched".into());
    }
    let window = &report.windows[0];
    if window.failure.is_some()
        || window.sample_in_run_id.is_none()
        || window.sample_out_run_id.is_none()
        || window.sample_in_metrics.is_none()
        || window.sample_out_metrics.is_none()
        || window.sample_in_snapshot_id.is_empty()
        || window.sample_out_snapshot_id.is_empty()
        || window.sample_in_start_time_ms != Some(context.selection_window.start_time_ms)
        || window.sample_in_end_time_ms != Some(context.selection_window.end_time_ms)
        || window.sample_out_start_time_ms != context.final_window.start_time_ms
        || window.sample_out_end_time_ms != Some(context.final_window.end_time_ms)
    {
        return Err(
            "Validation Report lacks a complete chronological holdout evidence window".into(),
        );
    }
    Ok(())
}

struct GeneratedStrategy {
    package_bytes: Vec<u8>,
    provenance: StrategyPackageProvenance,
    parameters: BTreeMap<String, String>,
}

fn generate_strategy_package(
    revision: &StrategyCandidateRevision,
) -> Result<GeneratedStrategy, String> {
    revision.validate()?;
    validate_generator_shape(revision)?;
    let source_definition_sha256 = sha256_json(&revision.definition)?;
    let canonicalization_sha256 = source_definition_sha256.clone();
    let sdk_version = adaq_component_sdk::SDK_VERSION.to_owned();
    let abi_version = adaq_component_sdk::ABI_VERSION.to_owned();
    let identity_material = serde_json::json!({
        "schemaVersion": STRATEGY_QUALIFICATION_SCHEMA_VERSION,
        "generatorId": STRATEGY_GENERATOR_ID,
        "sdkVersion": sdk_version,
        "abiVersion": abi_version,
        "toolchain": "stable",
        "target": STRATEGY_TARGET,
        "canonicalizationVersion": STRATEGY_CANONICALIZATION_VERSION,
        "candidateId": revision.candidate_id,
        "candidateRevision": revision.revision,
        "candidateRevisionHash": revision.revision_hash,
        "sourceDefinitionSha256": source_definition_sha256,
    });
    let component_id = deterministic_uuid(&sha256_json(&identity_material)?).to_string();
    let package_name = format!("adaq-gate11-{}", &sha256_json(&identity_material)?[..16]);
    let parameter_grid = strategy_parameter_grid(revision)?;
    let (manifest, parameters) = generated_manifest(revision, &component_id, &package_name)?;
    let source = render_strategy_source(revision)?;
    let generated_source_sha256 = sha256(source.as_bytes());
    let local_sdk_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/adaq-component-sdk");
    let sdk_path = local_sdk_path.is_dir().then_some(local_sdk_path.as_path());
    let project_parent = strategy_project_parent()?;
    let project = match create_project(
        ComponentTemplate::Strategy,
        &package_name,
        &project_parent,
        sdk_path,
    ) {
        Ok(project) => project,
        Err(error) => {
            let _ = fs::remove_dir_all(&project_parent);
            return Err(error);
        }
    };
    let result = (|| {
        fs::write(project.join("src/lib.rs"), &source).map_err(string)?;
        fs::write(
            project.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(string)?,
        )
        .map_err(string)?;
        let lock_diagnostics = generate_lockfile(&project)?;
        let build = build_project_offline_with_diagnostics(&project)?;
        let bytes = fs::read(&build.package_path).map_err(string)?;
        let package = ComponentPackage::read(&bytes).map_err(string)?;
        verify_package(&package)?;
        if package.manifest.component_id.to_string() != component_id
            || package.manifest.feature_slots != manifest.feature_slots
            || package.manifest.parameters != manifest.parameters
            || package.manifest.dependencies != manifest.dependencies
        {
            return Err("generated Strategy Package does not match its frozen contract".into());
        }
        let qualification = qualify_package_with_parameter_grid(
            format!("strategy-package-{component_id}"),
            &package,
            &parameter_grid,
            |qualified_package, parameters| {
                verify_strategy_equivalence(revision, qualified_package, parameters)
            },
        );
        if !qualification.qualified {
            let diagnostic = qualification
                .evidence
                .iter()
                .filter_map(|evidence| evidence.diagnostic.as_deref())
                .next()
                .unwrap_or("unknown package qualification failure");
            return Err(format!(
                "generated Strategy Package conformance/equivalence failed ({:?}): {diagnostic}",
                package.manifest.strategy_scope
            ));
        }
        let compiler = compiler_identity()?;
        let diagnostics = [lock_diagnostics, build.diagnostics]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let mut provenance = StrategyPackageProvenance {
            schema_version: STRATEGY_QUALIFICATION_SCHEMA_VERSION.into(),
            generator_id: STRATEGY_GENERATOR_ID.into(),
            sdk_version: adaq_component_sdk::SDK_VERSION.into(),
            abi_version: adaq_component_sdk::ABI_VERSION.into(),
            toolchain: "stable".into(),
            compiler,
            target: STRATEGY_TARGET.into(),
            canonicalization_version: STRATEGY_CANONICALIZATION_VERSION.into(),
            canonicalization_sha256,
            candidate_id: revision.candidate_id.clone(),
            candidate_revision: revision.revision,
            candidate_revision_hash: revision.revision_hash.clone(),
            source_definition_sha256,
            generated_source_sha256,
            package_archive_sha256: package.archive_sha256.clone(),
            package_wasm_sha256: package.manifest.wasm_sha256.clone(),
            parameters: parameters.clone(),
            parameter_grid: parameter_grid.clone(),
            qualification,
            diagnostic_log_sha256: sha256(diagnostics.as_bytes()),
            commands: STRATEGY_BUILD_COMMANDS
                .iter()
                .map(|value| (*value).into())
                .collect(),
            package_provenance_hash: String::new(),
        };
        provenance.package_provenance_hash = package_provenance_hash(&provenance)?;
        Ok(GeneratedStrategy {
            package_bytes: bytes,
            provenance,
            parameters,
        })
    })();
    let _ = fs::remove_dir_all(&project_parent);
    result
}

fn strategy_project_parent() -> Result<std::path::PathBuf, String> {
    let parent = std::env::temp_dir().join(format!("adaq-gate11-{}", Uuid::new_v4()));
    fs::create_dir(&parent).map_err(string)?;
    Ok(parent)
}

fn verify_strategy_equivalence(
    revision: &StrategyCandidateRevision,
    package: &ComponentPackage,
    parameters: &[ComponentParameterValue],
) -> Result<(), String> {
    if matches!(
        revision.scope,
        crate::strategy_candidate::StrategyScope::Portfolio
    ) {
        return verify_portfolio_strategy_equivalence(revision, package, parameters);
    }
    let slots = package
        .manifest
        .feature_slots
        .iter()
        .map(|slot| {
            adaq_component_sdk::host::strategy_abi::exports::adaq::strategy::api::FeatureSlot {
                name: slot.name.clone(),
            }
        })
        .collect::<Vec<_>>();
    let values = slots
        .iter()
        .enumerate()
        .map(|(index, _)| 0.2 + index as f64 * 0.17)
        .collect::<Vec<_>>();
    let frame =
        adaq_component_sdk::host::strategy_abi::exports::adaq::strategy::api::FeatureFrame {
            open_time_ms: 1,
            values: values.clone(),
        };
    let loader = WasmLoader::with_limits(RunLimits::default());
    loader.load_strategy_bytes(&package.wasm, slots, parameters)?;
    let actual = loader.process_strategy(vec![frame])?;
    let expected = vec![reference_strategy_output(revision, parameters, &values)?];
    if actual != expected {
        return Err(format!(
            "generated Strategy output differs from the deterministic reference: expected {expected:?}, got {actual:?}"
        ));
    }
    Ok(())
}

fn verify_portfolio_strategy_equivalence(
    revision: &StrategyCandidateRevision,
    package: &ComponentPackage,
    parameters: &[ComponentParameterValue],
) -> Result<(), String> {
    use adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api as abi;

    let slots = package
        .manifest
        .feature_slots
        .iter()
        .map(|slot| abi::FeatureSlot {
            name: slot.name.clone(),
        })
        .collect::<Vec<_>>();
    let rows = (0..5)
        .map(|index| abi::FeatureRow {
            instrument_id: format!("I{index}"),
            values: (0..slots.len())
                .map(|slot| 0.2 + slot as f64 * 0.17 + index as f64 * 0.11)
                .collect(),
        })
        .collect::<Vec<_>>();
    let frame = abi::PortfolioFrame {
        decision_time_ms: 1,
        universe_id: "qualification-universe".into(),
        rows,
        state: abi::PortfolioState {
            cash: "10000".into(),
            positions: Vec::new(),
        },
    };
    let loader = WasmLoader::with_limits(RunLimits::default());
    loader.load_portfolio_strategy_bytes(&package.wasm, slots, parameters)?;
    let actual = loader.process_portfolio_strategy(vec![frame.clone()])?;
    let expected = vec![reference_portfolio_strategy_output(
        revision, parameters, &frame,
    )?];
    if !portfolio_targets_equal(&actual, &expected) {
        return Err(format!(
            "generated Portfolio Strategy output differs from the deterministic reference: expected {expected:?}, got {actual:?}"
        ));
    }
    Ok(())
}

fn reference_portfolio_strategy_output(
    revision: &StrategyCandidateRevision,
    parameters: &[ComponentParameterValue],
    frame: &adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioFrame,
) -> Result<adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioTarget, String>{
    use adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api as abi;

    enum Value {
        Scores(BTreeMap<String, f64>),
        Target {
            weights: BTreeMap<String, f64>,
            cash_reserve: f64,
        },
    }

    let mut values = BTreeMap::new();
    for (index, slot) in revision.definition.input_slots.iter().enumerate() {
        values.insert(
            slot.alias.clone(),
            Value::Scores(
                frame
                    .rows
                    .iter()
                    .map(|row| (row.instrument_id.clone(), row.values[index]))
                    .collect(),
            ),
        );
    }

    let mut parameter_index = 0;
    for node in &revision.definition.nodes {
        let inputs = node
            .input_aliases
            .iter()
            .map(|alias| {
                values
                    .get(alias)
                    .ok_or_else(|| "reference Portfolio Strategy input alias is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let parameter = || {
            parameters
                .get(parameter_index)
                .ok_or_else(|| "reference Portfolio Strategy parameter is missing".to_owned())
                .and_then(component_parameter_f64)
        };
        let output = match node.operation.as_str() {
            "weighted-sum" => {
                let weight = parameter()?;
                let (Value::Scores(left), Value::Scores(right)) = (&inputs[0], &inputs[1]) else {
                    return Err("reference weighted-sum requires score inputs".into());
                };
                Value::Scores(
                    left.iter()
                        .map(|(id, left)| {
                            (
                                id.clone(),
                                left * (1.0 - weight)
                                    + right.get(id).copied().unwrap_or_default() * weight,
                            )
                        })
                        .collect(),
                )
            }
            "top-n" => {
                let top_n = parameter()? as usize;
                let Value::Scores(scores) = &inputs[0] else {
                    return Err("reference top-n requires score input".into());
                };
                if !matches!(top_n, 3 | 5) || top_n > scores.len() {
                    return Err("reference top-n does not fit the Point-in-Time Universe".into());
                }
                let mut ranked = scores.iter().collect::<Vec<_>>();
                ranked.sort_by(|(left_id, left), (right_id, right)| {
                    right
                        .partial_cmp(left)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| left_id.cmp(right_id))
                });
                let weight = 1.0 / top_n as f64;
                let selected = ranked
                    .into_iter()
                    .take(top_n)
                    .map(|(id, _)| (id.clone(), weight))
                    .collect::<BTreeMap<_, _>>();
                Value::Target {
                    weights: scores
                        .keys()
                        .map(|id| (id.clone(), selected.get(id).copied().unwrap_or(0.0)))
                        .collect(),
                    cash_reserve: 0.0,
                }
            }
            "equal-weight" => {
                let Value::Scores(scores) = &inputs[0] else {
                    return Err("reference equal-weight requires score input".into());
                };
                if scores.is_empty() {
                    return Err("reference equal-weight requires a non-empty Universe".into());
                }
                let weight = 1.0 / scores.len() as f64;
                Value::Target {
                    weights: scores.keys().map(|id| (id.clone(), weight)).collect(),
                    cash_reserve: 0.0,
                }
            }
            "cash-reserve" => {
                let reserve = parameter()?;
                let Value::Target {
                    weights,
                    cash_reserve: _,
                } = &inputs[0]
                else {
                    return Err("reference cash-reserve requires a Portfolio Target".into());
                };
                Value::Target {
                    weights: weights
                        .iter()
                        .map(|(id, weight)| (id.clone(), weight * (1.0 - reserve)))
                        .collect(),
                    cash_reserve: reserve,
                }
            }
            _ => return Err("reference Portfolio Strategy operation is unsupported".into()),
        };
        values.insert(node.output_alias.clone(), output);
        parameter_index += node.parameters.len();
    }

    let last_alias = revision
        .definition
        .nodes
        .last()
        .map(|node| node.output_alias.as_str())
        .ok_or("reference Portfolio Strategy output is missing")?;
    let Value::Target {
        weights,
        cash_reserve,
    } = values
        .remove(last_alias)
        .ok_or("reference Portfolio Strategy output is missing")?
    else {
        return Err("reference Portfolio Strategy output is not a complete target".into());
    };
    let cash_reserve = exact_portfolio_decimal(cash_reserve)?;
    let mut total = cash_reserve;
    let output = frame
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let raw = weights
                .get(&row.instrument_id)
                .copied()
                .ok_or("reference Portfolio Strategy output omits a Universe member")?;
            let weight = if index + 1 == frame.rows.len() {
                adaq_component_sdk::Decimal::ONE - total
            } else {
                exact_portfolio_decimal(raw)?
            };
            if weight < adaq_component_sdk::Decimal::ZERO {
                return Err("reference Portfolio Strategy target weight is negative".into());
            }
            total += weight;
            Ok(abi::TargetWeight {
                instrument_id: row.instrument_id.clone(),
                weight: weight.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if total != adaq_component_sdk::Decimal::ONE {
        return Err("reference Portfolio Strategy target does not sum to one".into());
    }
    Ok(abi::PortfolioTarget {
        decision_time_ms: frame.decision_time_ms,
        universe_id: frame.universe_id.clone(),
        weights: output,
        cash_reserve: cash_reserve.to_string(),
    })
}

fn portfolio_targets_equal(
    left: &[adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioTarget],
    right: &[adaq_component_sdk::host::portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioTarget],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.decision_time_ms == right.decision_time_ms
                && left.universe_id == right.universe_id
                && left.cash_reserve == right.cash_reserve
                && left.weights.len() == right.weights.len()
                && left
                    .weights
                    .iter()
                    .zip(&right.weights)
                    .all(|(left, right)| {
                        left.instrument_id == right.instrument_id && left.weight == right.weight
                    })
        })
}

fn exact_portfolio_decimal(value: f64) -> Result<Decimal, String> {
    if !value.is_finite() {
        return Err("reference Portfolio Strategy decimal is not finite".into());
    }
    adaq_component_sdk::parse_decimal(&format!("{value:.12}"))
}

fn reference_strategy_output(
    revision: &StrategyCandidateRevision,
    parameters: &[ComponentParameterValue],
    input_values: &[f64],
) -> Result<String, String> {
    let mut values = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (slot.alias.clone(), input_values[index]))
        .collect::<BTreeMap<_, _>>();
    let mut parameter_index = 0;
    for node in &revision.definition.nodes {
        let inputs = node
            .input_aliases
            .iter()
            .map(|alias| {
                values
                    .get(alias)
                    .copied()
                    .ok_or_else(|| "reference Strategy input alias is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let parameter = || {
            parameters
                .get(parameter_index)
                .ok_or_else(|| "reference Strategy parameter is missing".to_owned())
                .and_then(component_parameter_f64)
        };
        let output = match node.operation.as_str() {
            "weighted-sum" => {
                let weight = parameter()?;
                inputs[0] * (1.0 - weight) + inputs[1] * weight
            }
            "top-n" | "equal-weight" => inputs[0].clamp(0.0, 1.0),
            "cash-reserve" => {
                let reserve = parameter()?;
                (inputs[0] * (1.0 - reserve)).clamp(0.0, 1.0)
            }
            _ => return Err("reference Strategy operation is unsupported".into()),
        };
        if !output.is_finite() {
            return Err("reference Strategy output is not finite".into());
        }
        values.insert(node.output_alias.clone(), output);
        parameter_index += node.parameters.len();
    }
    let last = revision
        .definition
        .nodes
        .last()
        .and_then(|node| values.get(&node.output_alias))
        .copied()
        .ok_or("reference Strategy output is missing")?;
    Ok(format_exposure(last))
}

fn component_parameter_f64(value: &ComponentParameterValue) -> Result<f64, String> {
    match value {
        ComponentParameterValue::Decimal(value) => {
            adaq_component_sdk::decimal_to_f64(adaq_component_sdk::parse_decimal(value)?)
        }
        ComponentParameterValue::Integer(value) => Ok(*value as f64),
        ComponentParameterValue::Boolean(_) | ComponentParameterValue::String(_) => {
            Err("reference Strategy parameter has an unsupported type".into())
        }
    }
}

fn format_exposure(value: f64) -> String {
    let value = value.clamp(0.0, 1.0);
    let text = format!("{value:.12}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn validate_generator_shape(revision: &StrategyCandidateRevision) -> Result<(), String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ValueKind {
        Scores,
        Target,
    }

    let mut known = revision
        .definition
        .input_slots
        .iter()
        .map(|slot| slot.alias.clone())
        .collect::<HashSet<_>>();
    let mut kinds = revision
        .definition
        .input_slots
        .iter()
        .map(|slot| (slot.alias.clone(), ValueKind::Scores))
        .collect::<HashMap<_, _>>();
    for node in &revision.definition.nodes {
        let arity = match node.operation.as_str() {
            "weighted-sum" => 2,
            "top-n" | "equal-weight" | "cash-reserve" => 1,
            _ => {
                return Err(
                    "Strategy operation is unsupported by the deterministic generator".into(),
                );
            }
        };
        if node.input_aliases.len() != arity
            || node
                .input_aliases
                .iter()
                .any(|alias| !known.contains(alias))
        {
            return Err(format!(
                "Strategy operation {} has an unsupported input shape",
                node.node_id
            ));
        }
        if !known.insert(node.output_alias.clone()) {
            return Err("Strategy output aliases are not unique".into());
        }
        if node
            .parameters
            .values()
            .any(|value| matches!(value, StrategyValue::Boolean(_) | StrategyValue::Text(_)))
        {
            return Err(
                "Strategy parameter type is unsupported by the deterministic generator".into(),
            );
        }
        let input_kinds = node
            .input_aliases
            .iter()
            .map(|alias| kinds.get(alias).copied())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| "Strategy operation input kind is unavailable".to_owned())?;
        let output_kind = match node.operation.as_str() {
            "weighted-sum" if input_kinds == [ValueKind::Scores, ValueKind::Scores] => {
                ValueKind::Scores
            }
            "top-n" | "equal-weight" if input_kinds == [ValueKind::Scores] => ValueKind::Target,
            "cash-reserve" if input_kinds == [ValueKind::Target] => ValueKind::Target,
            _ => {
                return Err(format!(
                    "Strategy operation {} has incompatible value kinds",
                    node.node_id
                ));
            }
        };
        kinds.insert(node.output_alias.clone(), output_kind);
    }
    let output_kind = revision
        .definition
        .nodes
        .last()
        .and_then(|node| kinds.get(&node.output_alias))
        .copied();
    let valid_output_kind = match revision.scope {
        crate::strategy_candidate::StrategyScope::SingleInstrument => Some(ValueKind::Scores),
        crate::strategy_candidate::StrategyScope::Portfolio => Some(ValueKind::Target),
    };
    if output_kind != valid_output_kind {
        return Err("Strategy output kind does not match its declared scope".into());
    }
    Ok(())
}

fn generated_manifest(
    revision: &StrategyCandidateRevision,
    component_id: &str,
    package_name: &str,
) -> Result<(ComponentManifest, BTreeMap<String, String>), String> {
    let mut feature_slots = Vec::new();
    let mut dependencies = Vec::new();
    for slot in &revision.definition.input_slots {
        match &slot.binding {
            StrategyInputBinding::Factor(binding) => {
                feature_slots.push(serde_json::json!({
                    "name": slot.alias,
                    "source": {
                        "kind": "external",
                        "dependencyAlias": slot.alias,
                        "output": binding.output_name,
                    }
                }));
                dependencies.push(serde_json::json!({
                    "componentId": binding.component_id,
                    "version": format!("={}", binding.component_version),
                    "alias": slot.alias,
                }));
            }
            StrategyInputBinding::Model(binding) => {
                if binding.forecast_contract != "forecast:continuous-future-close-return:native@1" {
                    return Err(
                        "Model Forecast Contract is unsupported by the Strategy generator".into(),
                    );
                }
                feature_slots.push(serde_json::json!({
                    "name": slot.alias,
                    "source": {
                        "kind": "signal",
                        "predictionKind": { "kind": "expected-value" },
                        "forecastTarget": {
                            "kind": "builtin",
                            "target": "future-close-return"
                        },
                        "valueScale": { "kind": "native" },
                        "horizonBars": binding.target_horizon_bars,
                    }
                }));
            }
        }
    }
    let mut parameters = Vec::new();
    let mut values = BTreeMap::new();
    for node in &revision.definition.nodes {
        for (name, value) in &node.parameters {
            let parameter_name = format!("{}-{}", node.node_id, name);
            let (parameter_type, default_value, allowed_values) =
                strategy_value_manifest(&node.operation, name, value)?;
            parameters.push(serde_json::json!({
                "name": parameter_name,
                "parameterType": parameter_type,
                "defaultValue": default_value,
                "allowedValues": if allowed_values.is_empty() {
                    Vec::<String>::new()
                } else {
                    vec![default_value.clone()]
                },
            }));
            values.insert(parameter_name, default_value);
        }
    }
    let manifest: ComponentManifest = serde_json::from_value(serde_json::json!({
        "manifestSchemaVersion": "1.0.0",
        "componentId": component_id,
        "version": "0.1.0",
        "name": package_name,
        "kind": "strategy",
        "strategyScope": match revision.scope {
            crate::strategy_candidate::StrategyScope::SingleInstrument => "single-instrument",
            crate::strategy_candidate::StrategyScope::Portfolio => "portfolio",
        },
        "sdkVersion": adaq_component_sdk::SDK_VERSION,
        "abiVersion": adaq_component_sdk::ABI_VERSION,
        "parameters": parameters,
        "featureSlots": feature_slots,
        "dependencies": dependencies,
        "warmupBars": 0,
    }))
    .map_err(string)?;
    if manifest.kind != ComponentKind::Strategy
        || manifest.feature_slots.iter().any(|slot| {
            !matches!(
                slot.source,
                FeatureSlotSource::External { .. } | FeatureSlotSource::Signal { .. }
            )
        })
    {
        return Err("generated Strategy manifest is not a valid external/signal contract".into());
    }
    Ok((manifest, values))
}

fn strategy_value_manifest(
    operation: &str,
    name: &str,
    value: &StrategyValue,
) -> Result<(&'static str, String, Vec<String>), String> {
    let allowed_values = match (operation, name) {
        ("weighted-sum", "forecast-weight") => vec!["0.5".into(), "0.7".into()],
        ("top-n", "top-n") => vec!["3".into(), "5".into()],
        ("cash-reserve", "cash-reserve") => vec!["0".into(), "0.1".into()],
        ("equal-weight", _) => Vec::new(),
        _ => return Err("Strategy parameter is not in the portable catalog".into()),
    };
    let (parameter_type, default_value) = match value {
        StrategyValue::Decimal(value) => ("decimal", value.clone()),
        StrategyValue::Integer(value) => ("integer", value.to_string()),
        StrategyValue::Boolean(_) | StrategyValue::Text(_) => {
            return Err("Strategy parameter type is not portable in Gate 11".into());
        }
    };
    if !allowed_values.is_empty() && !allowed_values.contains(&default_value) {
        return Err("Strategy parameter default is outside the portable catalog".into());
    }
    Ok((parameter_type, default_value, allowed_values))
}

fn render_strategy_source(revision: &StrategyCandidateRevision) -> Result<String, String> {
    if matches!(
        revision.scope,
        crate::strategy_candidate::StrategyScope::Portfolio
    ) {
        return render_portfolio_strategy_source(revision);
    }
    render_single_strategy_source(revision)
}

fn render_single_strategy_source(revision: &StrategyCandidateRevision) -> Result<String, String> {
    let struct_slots = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, _)| format!("    {}: usize,", slot_field(index)))
        .collect::<Vec<_>>()
        .join("\n");
    let struct_parameters = revision
        .definition
        .nodes
        .iter()
        .flat_map(|node| node.parameters.keys())
        .enumerate()
        .map(|(index, _)| format!("    {}: f64,", parameter_field(index)))
        .collect::<Vec<_>>()
        .join("\n");
    let slots = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            format!(
                "            {}: slots.index({})?,",
                slot_field(index),
                rust_string(&slot.alias)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let parameters = revision
        .definition
        .nodes
        .iter()
        .flat_map(|node| node.parameters.keys().map(move |name| (node, name)))
        .enumerate()
        .map(|(index, (node, name))| {
            let parameter_name = format!("{}-{}", node.node_id, name);
            let matcher = match node.parameters.get(name) {
                Some(StrategyValue::Decimal(_)) => {
                    "Some(ParameterValue::Decimal(value)) => adaq_component_sdk::decimal_to_f64(adaq_component_sdk::parse_decimal(value.as_str())?)?"
                }
                Some(StrategyValue::Integer(_)) => "Some(ParameterValue::Integer(value)) => *value as f64",
                _ => "_ => return Err(\"unsupported generated parameter\".into())",
            };
            format!(
                "            {}: match parameters.get({}) {{ {} , _ => return Err({}.into()) }},",
                parameter_field(index),
                index,
                matcher,
                rust_string(&format!("invalid generated parameter {parameter_name}"))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            (
                slot.alias.clone(),
                format!("frame.values[self.{}]", slot_field(index)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut node_code = Vec::new();
    let mut parameter_index = 0;
    for (node_index, node) in revision.definition.nodes.iter().enumerate() {
        let inputs = node
            .input_aliases
            .iter()
            .map(|alias| {
                values
                    .get(alias)
                    .cloned()
                    .ok_or_else(|| "generated input alias is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expression = match node.operation.as_str() {
            "weighted-sum" => format!(
                "({} * (1.0 - self.{}) + {} * self.{})",
                inputs[0],
                parameter_field(parameter_index),
                inputs[1],
                parameter_field(parameter_index)
            ),
            "top-n" => format!("({}).clamp(0.0, 1.0)", inputs[0]),
            "equal-weight" => format!("({}).clamp(0.0, 1.0)", inputs[0]),
            "cash-reserve" => format!(
                "({} * (1.0 - self.{})).clamp(0.0, 1.0)",
                inputs[0],
                parameter_field(parameter_index)
            ),
            _ => return Err("unsupported generated Strategy operation".into()),
        };
        let output = format!("v{node_index}");
        node_code.push(format!("                let {output} = {expression};"));
        values.insert(node.output_alias.clone(), output);
        parameter_index += node.parameters.len();
    }
    let last_value = revision
        .definition
        .nodes
        .last()
        .and_then(|node| values.get(&node.output_alias))
        .ok_or("generated Strategy has no output")?;
    Ok(format!(
        "use adaq_component_sdk::strategy::{{
    FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance,
    ParameterValue, SlotIndexes,
}};

struct Component;

struct Instance {{
{slots}
{parameters}
}}

impl Guest for Component {{
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {{
        if feature_slots.len() != {slot_count} || parameters.len() != {parameter_count} {{
            return Err(\"generated Strategy contract length mismatch\".into());
        }}
        let slots = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance {{
{slot_bindings}
{parameter_bindings}
        }}))
    }}
}}

impl GuestInstance for Instance {{
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {{
        frames
            .into_iter()
            .map(|frame| {{
                if frame.values.len() != {slot_count}
                    || frame.values.iter().any(|value| !value.is_finite())
                {{
                    return Err(\"generated Strategy Feature Frame is invalid\".into());
                }}
{node_code}
                let value = {last_value};
                if !value.is_finite() {{
                    return Err(\"generated Strategy output is not finite\".into());
                }}
                Ok(exposure(value))
            }})
            .collect()
    }}
}}

fn exposure(value: f64) -> String {{
    let value = value.clamp(0.0, 1.0);
    let text = format!(\"{{value:.12}}\");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}}

adaq_component_sdk::strategy::bindings::export_strategy!(
    Component with_types_in adaq_component_sdk::strategy::bindings
);
",
        slots = struct_slots,
        parameters = struct_parameters,
        slot_count = revision.definition.input_slots.len(),
        parameter_count = revision
            .definition
            .nodes
            .iter()
            .map(|node| node.parameters.len())
            .sum::<usize>(),
        slot_bindings = slots,
        parameter_bindings = parameters,
        node_code = node_code.join("\n"),
        last_value = last_value,
    ))
}

fn render_portfolio_strategy_source(
    revision: &StrategyCandidateRevision,
) -> Result<String, String> {
    let struct_slots = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, _)| format!("    {}: usize,", slot_field(index)))
        .collect::<Vec<_>>()
        .join("\n");
    let struct_parameters = revision
        .definition
        .nodes
        .iter()
        .flat_map(|node| node.parameters.keys())
        .enumerate()
        .map(|(index, _)| format!("    {}: f64,", parameter_field(index)))
        .collect::<Vec<_>>()
        .join("\n");
    let slots = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            format!(
                "            {}: slots.index({})?,",
                slot_field(index),
                rust_string(&slot.alias)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let parameters = revision
        .definition
        .nodes
        .iter()
        .flat_map(|node| node.parameters.keys().map(move |name| (node, name)))
        .enumerate()
        .map(|(index, (node, name))| {
            let parameter_name = format!("{}-{}", node.node_id, name);
            let matcher = match node.parameters.get(name) {
                Some(StrategyValue::Decimal(_)) => {
                    "Some(ParameterValue::Decimal(value)) => adaq_component_sdk::decimal_to_f64(adaq_component_sdk::parse_decimal(value.as_str())?)?"
                }
                Some(StrategyValue::Integer(_)) => "Some(ParameterValue::Integer(value)) => *value as f64",
                _ => "_ => return Err(\"unsupported generated parameter\".into())",
            };
            format!(
                "            {}: match parameters.get({}) {{ {} , _ => return Err({}.into()) }},",
                parameter_field(index),
                index,
                matcher,
                rust_string(&format!("invalid generated parameter {parameter_name}"))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let input_bindings = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, _)| {
            format!(
                "                let input_{index} = scores(&frame.rows, self.{})?;",
                slot_field(index)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = revision
        .definition
        .input_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (slot.alias.clone(), format!("input_{index}")))
        .collect::<BTreeMap<_, _>>();
    let mut node_code = Vec::new();
    let mut parameter_index = 0;
    for (node_index, node) in revision.definition.nodes.iter().enumerate() {
        let inputs = node
            .input_aliases
            .iter()
            .map(|alias| {
                values
                    .get(alias)
                    .cloned()
                    .ok_or_else(|| "generated input alias is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expression = match node.operation.as_str() {
            "weighted-sum" => format!(
                "weighted_sum(&{}, &{}, self.{})?",
                inputs[0],
                inputs[1],
                parameter_field(parameter_index)
            ),
            "top-n" => format!(
                "top_n(&{}, self.{} as usize)?",
                inputs[0],
                parameter_field(parameter_index)
            ),
            "equal-weight" => format!("equal_weight(&{})?", inputs[0]),
            "cash-reserve" => format!(
                "cash_reserve(&{}, self.{})?",
                inputs[0],
                parameter_field(parameter_index)
            ),
            _ => return Err("unsupported generated Portfolio Strategy operation".into()),
        };
        let output = format!("v{node_index}");
        node_code.push(format!("                let {output} = {expression};"));
        values.insert(node.output_alias.clone(), output);
        parameter_index += node.parameters.len();
    }
    let last_value = revision
        .definition
        .nodes
        .last()
        .and_then(|node| values.get(&node.output_alias))
        .ok_or("generated Portfolio Strategy has no output")?;
    Ok(format!(
        "use std::{{cmp::Ordering, collections::BTreeMap}};
use adaq_component_sdk::portfolio_strategy::{{
    FeatureRow, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance,
    ParameterValue, PortfolioFrame, PortfolioTarget, SlotIndexes, TargetWeight,
}};

enum Value {{
    Scores(BTreeMap<String, f64>),
    Target {{
        weights: BTreeMap<String, f64>,
        cash_reserve: f64,
    }},
}}

struct Component;

struct Instance {{
{slots}
{parameters}
}}

impl Guest for Component {{
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {{
        if feature_slots.len() != {slot_count} || parameters.len() != {parameter_count} {{
            return Err(\"generated Portfolio Strategy contract length mismatch\".into());
        }}
        let slots = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance {{
{slot_bindings}
{parameter_bindings}
        }}))
    }}
}}

impl GuestInstance for Instance {{
    fn process(&self, frames: Vec<PortfolioFrame>) -> Result<Vec<PortfolioTarget>, String> {{
        frames
            .into_iter()
            .map(|frame| {{
                if frame.universe_id.is_empty() || frame.rows.is_empty() {{
                    return Err(\"generated Portfolio Strategy frame is invalid\".into());
                }}
                adaq_component_sdk::parse_decimal(&frame.state.cash)?;
{input_bindings}
{node_code}
                target({last_value}, &frame)
            }})
            .collect()
    }}
}}

fn scores(rows: &[FeatureRow], slot: usize) -> Result<Value, String> {{
    let mut values = BTreeMap::new();
    for row in rows {{
        if row.instrument_id.is_empty()
            || row.values.len() <= slot
            || !row.values[slot].is_finite()
            || values.insert(row.instrument_id.clone(), row.values[slot]).is_some()
        {{
            return Err(\"generated Portfolio Strategy feature rows are invalid\".into());
        }}
    }}
    Ok(Value::Scores(values))
}}

fn weighted_sum(left: &Value, right: &Value, weight: f64) -> Result<Value, String> {{
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {{
        return Err(\"generated Portfolio Strategy weight is invalid\".into());
    }}
    let (Value::Scores(left), Value::Scores(right)) = (left, right) else {{
        return Err(\"weighted-sum requires score inputs\".into());
    }};
    if left.keys().ne(right.keys()) {{
        return Err(\"weighted-sum input Universes do not match\".into());
    }}
    let values = left
        .iter()
        .map(|(id, left)| {{
            let right = right.get(id).ok_or(\"weighted-sum input is incomplete\")?;
            let value = left * (1.0 - weight) + right * weight;
            value.is_finite()
                .then_some((id.clone(), value))
                .ok_or(\"weighted-sum output is not finite\")
        }})
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(Value::Scores(values))
}}

fn top_n(value: &Value, top_n: usize) -> Result<Value, String> {{
    let Value::Scores(scores) = value else {{
        return Err(\"top-n requires score input\".into());
    }};
    if !matches!(top_n, 3 | 5) || top_n > scores.len() {{
        return Err(\"top-n does not fit the Point-in-Time Universe\".into());
    }}
    let mut ranked = scores.iter().collect::<Vec<_>>();
    ranked.sort_by(|(left_id, left), (right_id, right)| {{
        right
            .partial_cmp(left)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_id.cmp(right_id))
    }});
    let weight = 1.0 / top_n as f64;
    let selected = ranked
        .into_iter()
        .take(top_n)
        .map(|(id, _)| (id.clone(), weight))
        .collect::<BTreeMap<_, _>>();
    Ok(Value::Target {{
        weights: scores
            .keys()
            .map(|id| (id.clone(), selected.get(id).copied().unwrap_or(0.0)))
            .collect(),
        cash_reserve: 0.0,
    }})
}}

fn equal_weight(value: &Value) -> Result<Value, String> {{
    let Value::Scores(scores) = value else {{
        return Err(\"equal-weight requires score input\".into());
    }};
    if scores.is_empty() {{
        return Err(\"equal-weight requires a non-empty Universe\".into());
    }}
    let weight = 1.0 / scores.len() as f64;
    Ok(Value::Target {{
        weights: scores.keys().map(|id| (id.clone(), weight)).collect(),
        cash_reserve: 0.0,
    }})
}}

fn cash_reserve(value: &Value, reserve: f64) -> Result<Value, String> {{
    if !reserve.is_finite() || !(0.0..=1.0).contains(&reserve) {{
        return Err(\"cash-reserve is invalid\".into());
    }}
    let Value::Target {{ weights, cash_reserve }} = value else {{
        return Err(\"cash-reserve requires a Portfolio Target\".into());
    }};
    let scale = 1.0 - reserve;
    Ok(Value::Target {{
        weights: weights
            .iter()
            .map(|(id, weight)| {{
                let value = weight * scale;
                value.is_finite()
                    .then_some((id.clone(), value))
                    .ok_or(\"cash-reserve output is not finite\")
            }})
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        cash_reserve: reserve,
    }})
}}

fn target(value: Value, frame: &PortfolioFrame) -> Result<PortfolioTarget, String> {{
    let Value::Target {{ weights, cash_reserve }} = value else {{
        return Err(\"Portfolio Strategy output is not a complete Portfolio Target\".into());
    }};
    let reserve = decimal(cash_reserve)?;
    if reserve < adaq_component_sdk::Decimal::ZERO {{
        return Err(\"Portfolio Strategy cash reserve is negative\".into());
    }}
    let mut total = reserve;
    let mut output = Vec::with_capacity(frame.rows.len());
    for (index, row) in frame.rows.iter().enumerate() {{
        let raw = weights
            .get(&row.instrument_id)
            .copied()
            .ok_or(\"Portfolio Strategy output omits a Universe member\")?;
        let weight = if index + 1 == frame.rows.len() {{
            adaq_component_sdk::Decimal::ONE - total
        }} else {{
            decimal(raw)?
        }};
        if weight < adaq_component_sdk::Decimal::ZERO {{
            return Err(\"Portfolio Strategy target weight is negative\".into());
        }}
        total += weight;
        output.push(TargetWeight {{
            instrument_id: row.instrument_id.clone(),
            weight: weight.to_string(),
        }});
    }}
    if total != adaq_component_sdk::Decimal::ONE {{
        return Err(\"Portfolio Strategy target does not sum to one\".into());
    }}
    Ok(PortfolioTarget {{
        decision_time_ms: frame.decision_time_ms,
        universe_id: frame.universe_id.clone(),
        weights: output,
        cash_reserve: reserve.to_string(),
    }})
}}

fn decimal(value: f64) -> Result<adaq_component_sdk::Decimal, String> {{
    if !value.is_finite() {{
        return Err(\"Portfolio Strategy decimal is not finite\".into());
    }}
    adaq_component_sdk::parse_decimal(&format!(\"{{value:.12}}\"))
}}

adaq_component_sdk::portfolio_strategy::bindings::export_portfolio_strategy!(
    Component with_types_in adaq_component_sdk::portfolio_strategy::bindings
);
",
        slots = struct_slots,
        parameters = struct_parameters,
        slot_count = revision.definition.input_slots.len(),
        parameter_count = revision
            .definition
            .nodes
            .iter()
            .map(|node| node.parameters.len())
            .sum::<usize>(),
        slot_bindings = slots,
        parameter_bindings = parameters,
        input_bindings = input_bindings,
        node_code = node_code.join("\n"),
        last_value = last_value,
    ))
}

fn slot_field(index: usize) -> String {
    format!("slot_{index}")
}

fn parameter_field(index: usize) -> String {
    format!("parameter_{index}")
}

fn rust_string(value: &str) -> String {
    serde_json::to_string(value).expect("JSON strings are valid Rust string literals")
}

fn package_provenance_hash(provenance: &StrategyPackageProvenance) -> Result<String, String> {
    let mut content = provenance.clone();
    content.package_provenance_hash.clear();
    sha256_json(&content)
}

fn qualification_hash(qualification: &StrategyQualification) -> Result<String, String> {
    let mut content = qualification.clone();
    content.qualification_id.clear();
    content.evidence_hash.clear();
    sha256_json(&content)
}

fn sha256_json(value: &impl Serialize) -> Result<String, String> {
    let json = serde_json::to_value(value).map_err(string)?;
    let bytes = serde_json::to_vec(&canonical_json(json)).map_err(string)?;
    Ok(sha256(&bytes))
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect(),
        ),
        value => value,
    }
}

fn deterministic_uuid(hash: &str) -> Uuid {
    let bytes = (0..16)
        .map(|index| u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16).unwrap_or_default())
        .collect::<Vec<_>>();
    let mut raw = [0_u8; 16];
    raw.copy_from_slice(&bytes);
    raw[6] = (raw[6] & 0x0f) | 0x50;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    Uuid::from_bytes(raw)
}

fn generate_lockfile(root: &Path) -> Result<String, String> {
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .env("CARGO_NET_OFFLINE", "true")
        .current_dir(root)
        .output()
        .map_err(string)?;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(diagnostics)
    } else {
        Err(format!(
            "cargo generate-lockfile --offline failed with {}: {}",
            output.status,
            bounded(&diagnostics)
        ))
    }
}

fn compiler_identity() -> Result<String, String> {
    let output = Command::new("rustup")
        .args(["run", "stable", "rustc", "--version", "--verbose"])
        .output()
        .map_err(string)?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(string)
    } else {
        Err(format!(
            "stable rustc identity failed with {}",
            output.status
        ))
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bounded(value: &str) -> String {
    value.chars().take(MAX_DIAGNOSTIC_BYTES).collect()
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

fn mark_interrupted_attempt(attempt: &mut StrategyQualificationAttempt, now_ms: i64) {
    if attempt.status != StrategyQualificationAttemptStatus::Running {
        return;
    }
    attempt.status = StrategyQualificationAttemptStatus::Failed;
    attempt.diagnostics.push(StrategyQualificationDiagnostic {
        stage: "lifecycle".into(),
        code: "strategy-qualification-interrupted".into(),
        message: "The previous Desktop session ended while this Attempt was running; rerun it to create new evidence.".into(),
    });
    attempt.updated_at_ms = now_ms;
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub(crate) async fn strategy_qualification_run(
    mut request: StrategyQualificationRunRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<StrategyQualificationAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.run(request))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_qualification_qualify(
    request: StrategyQualificationAttemptRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<StrategyQualification, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.qualify(&user_id, &request.attempt_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_qualification_attempt_list(
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<Vec<StrategyQualificationAttempt>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.inner().attempt_list(&user_id)
}

#[tauri::command]
pub(crate) async fn strategy_qualification_attempt_get(
    request: StrategyQualificationAttemptRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<StrategyQualificationAttempt, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .inner()
        .attempt_for_user(&user_id, &request.attempt_id)
}

#[tauri::command]
pub(crate) async fn strategy_qualification_list(
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<Vec<StrategyQualification>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state.inner().qualification_list(&user_id)
}

#[tauri::command]
pub(crate) async fn strategy_qualification_get(
    request: StrategyQualificationIdRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyQualificationStore>>,
) -> Result<StrategyQualification, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    state
        .inner()
        .qualification_for_user(&user_id, &request.qualification_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_candidate::{
        FactorInputBinding, StrategyDefinition, StrategyInputSlot, StrategyInputType,
        StrategyOperationNode, StrategyOutputContract, StrategyScope, StrategySemanticContext,
    };

    fn hash(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    fn test_revision() -> StrategyCandidateRevision {
        let mut parameters = BTreeMap::new();
        parameters.insert(
            "forecast-weight".into(),
            StrategyValue::Decimal("0.7".into()),
        );
        let mut revision = StrategyCandidateRevision {
            candidate_id: Uuid::from_u128(1).to_string(),
            revision: 1,
            scope: StrategyScope::SingleInstrument,
            definition: StrategyDefinition {
                schema_version: crate::strategy_candidate::STRATEGY_CANDIDATE_SCHEMA_VERSION.into(),
                catalog_version: crate::strategy_candidate::STRATEGY_OPERATION_CATALOG_VERSION
                    .into(),
                input_slots: vec![
                    StrategyInputSlot {
                        alias: "factor-score".into(),
                        input_type: StrategyInputType::FactorScore,
                        binding: StrategyInputBinding::Factor(FactorInputBinding {
                            decision_id: Uuid::from_u128(2).to_string(),
                            decision_hash: hash('a'),
                            candidate_hash: hash('b'),
                            output_name: "score".into(),
                            package_archive_sha256: hash('c'),
                            package_wasm_sha256: hash('d'),
                            component_id: Uuid::from_u128(3).to_string(),
                            component_version: "1.0.0".into(),
                        }),
                    },
                    StrategyInputSlot {
                        alias: "forecast-signal".into(),
                        input_type: StrategyInputType::ForecastSignal,
                        binding: StrategyInputBinding::Model(
                            crate::strategy_candidate::ModelInputBinding {
                                qualification_report_id: hash('e'),
                                decision_id: hash('f'),
                                final_evaluation_report_id: hash('0'),
                                artifact_sha256: hash('1'),
                                transformation_sha256: hash('2'),
                                package_archive_sha256: hash('3'),
                                package_wasm_sha256: hash('4'),
                                component_id: Uuid::from_u128(4).to_string(),
                                component_version: "1.0.0".into(),
                                model_profile: "wasi-linear".into(),
                                exporter_id: "exporter".into(),
                                sdk_version: "1.0.0".into(),
                                abi_version: "1.0.0".into(),
                                runtime_identity: "runtime".into(),
                                input_slots: vec!["feature".into()],
                                output_name: "forecast".into(),
                                target_id: "future-close-return".into(),
                                target_horizon_bars: 1,
                                forecast_contract:
                                    "forecast:continuous-future-close-return:native@1".into(),
                            },
                        ),
                    },
                ],
                nodes: vec![StrategyOperationNode {
                    node_id: "blend".into(),
                    operation: "weighted-sum".into(),
                    input_aliases: vec!["factor-score".into(), "forecast-signal".into()],
                    parameters,
                    output_alias: "target-exposure".into(),
                }],
                output: StrategyOutputContract::TargetDecision {
                    node_id: "blend".into(),
                },
            },
            semantic_context: StrategySemanticContext {
                feature_plan_hash: hash('5'),
                research_context_hash: hash('6'),
                snapshot_id: "snapshot-1".into(),
                universe_id: "universe-1".into(),
                market: "crypto".into(),
                venue: "okx".into(),
                input_evidence_hashes: vec![hash('7')],
            },
            created_at_ms: 1,
            created_by_attempt_id: "attempt-1".into(),
            revision_hash: String::new(),
        };
        rehash_revision(&mut revision);
        revision
    }

    fn rehash_revision(revision: &mut StrategyCandidateRevision) {
        let mut content = revision.clone();
        content.created_at_ms = 0;
        content.created_by_attempt_id.clear();
        content.revision_hash.clear();
        revision.revision_hash = sha256(&serde_json::to_vec(&content).unwrap());
    }

    fn portfolio_test_revision() -> StrategyCandidateRevision {
        let mut revision = test_revision();
        revision.scope = StrategyScope::Portfolio;
        let weighted = revision.definition.nodes.remove(0);
        let mut top_n_parameters = BTreeMap::new();
        top_n_parameters.insert("top-n".into(), StrategyValue::Integer(3));
        let mut cash_reserve_parameters = BTreeMap::new();
        cash_reserve_parameters.insert("cash-reserve".into(), StrategyValue::Decimal("0.1".into()));
        revision.definition.nodes = vec![
            weighted,
            StrategyOperationNode {
                node_id: "select".into(),
                operation: "top-n".into(),
                input_aliases: vec!["target-exposure".into()],
                parameters: top_n_parameters,
                output_alias: "selected-exposure".into(),
            },
            StrategyOperationNode {
                node_id: "reserve".into(),
                operation: "cash-reserve".into(),
                input_aliases: vec!["selected-exposure".into()],
                parameters: cash_reserve_parameters,
                output_alias: "portfolio-exposure".into(),
            },
        ];
        revision.definition.output = StrategyOutputContract::PortfolioTarget {
            node_id: "reserve".into(),
        };
        rehash_revision(&mut revision);
        revision
    }

    #[test]
    fn generated_strategy_contract_is_deterministic_and_complete() {
        let revision = test_revision();
        revision.validate().unwrap();
        let component_id = Uuid::from_u128(5).to_string();
        let (left, left_values) =
            generated_manifest(&revision, &component_id, "test-strategy").unwrap();
        let (right, right_values) =
            generated_manifest(&revision, &component_id, "test-strategy").unwrap();
        assert_eq!(left, right);
        assert_eq!(left_values, right_values);
        assert_eq!(left.parameters.len(), 1);
        assert_eq!(left.parameters[0].allowed_values, vec!["0.7".to_owned()]);
        let source = render_strategy_source(&revision).unwrap();
        assert!(source.contains("SlotIndexes::bind"));
        assert!(source.contains("parse_decimal"));
        assert_eq!(source, render_strategy_source(&revision).unwrap());
    }

    #[test]
    fn generated_strategy_reference_is_exact_decimal_text() {
        let revision = test_revision();
        let output = reference_strategy_output(
            &revision,
            &[ComponentParameterValue::Decimal("0.7".into())],
            &[0.2, 0.37],
        )
        .unwrap();
        assert_eq!(output, "0.319");
    }

    #[test]
    fn generated_portfolio_strategy_uses_the_portfolio_contract() {
        let revision = portfolio_test_revision();
        revision.validate().unwrap();
        let (manifest, _) = generated_manifest(
            &revision,
            &Uuid::from_u128(6).to_string(),
            "portfolio-strategy",
        )
        .unwrap();
        assert_eq!(
            manifest.strategy_scope,
            adaq_component_tooling::StrategyScope::Portfolio
        );
        let source = render_strategy_source(&revision).unwrap();
        assert!(source.contains("export_portfolio_strategy"));
        assert!(source.contains("PortfolioFrame"));
        assert!(!source.contains("export_strategy!"));
    }

    #[test]
    #[ignore = "requires the local offline cargo-component toolchain"]
    fn generated_strategy_package_builds_and_qualifies() {
        for revision in [test_revision(), portfolio_test_revision()] {
            let generated = generate_strategy_package(&revision).unwrap();
            assert!(generated.provenance.qualification.qualified);
            assert_eq!(
                generated.provenance.package_archive_sha256,
                sha256(&generated.package_bytes)
            );
        }
    }

    #[test]
    fn deterministic_uuid_sets_uuid_v5_bits() {
        let uuid = deterministic_uuid(&hash('a'));
        assert_eq!(uuid.get_version_num(), 5);
        assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
        assert_eq!(uuid, deterministic_uuid(&hash('a')));
    }

    #[test]
    fn strategy_project_parents_are_unique() {
        let first = strategy_project_parent().unwrap();
        let second = strategy_project_parent().unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir());
        assert!(second.is_dir());
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn interrupted_attempt_is_failed_once_with_recovery_diagnostic() {
        let context: StrategyEvaluationContext = serde_json::from_value(serde_json::json!({
            "snapshotId": "snapshot-1",
            "universeSnapshotId": "universe-1",
            "universeId": "universe",
            "selectionWindow": {"startTimeMs": 0, "endTimeMs": 60_000},
            "finalWindow": {"startTimeMs": 120_000, "endTimeMs": 180_000},
            "riskPolicy": {
                "policyId": "gate-11-default",
                "maxInstrumentWeight": "1",
                "maxTurnover": null
            },
            "executionProfile": {
                "makerFeeRate": "0.0008",
                "takerFeeRate": "0.001",
                "adverseSlippageRate": "0",
                "rebalanceThreshold": "0",
                "priceIncrement": "0.0001",
                "quantityIncrement": "0.0001",
                "minimumQuantity": "0.0001",
                "riskFreeRate": "0",
                "fillPolicy": "taker"
            },
            "signalInstances": [],
            "initialQuoteAllocation": "10000",
            "seed": 0,
            "validationMethodVersion": "chronological-holdout@1",
            "aggregationRuleVersion": "equal-window@1"
        }))
        .unwrap();
        let mut attempt = StrategyQualificationAttempt {
            attempt_id: "attempt".into(),
            user_id: "user".into(),
            candidate_id: "candidate".into(),
            candidate_revision: 1,
            candidate_revision_hash: String::new(),
            status: StrategyQualificationAttemptStatus::Running,
            package: None,
            context,
            backtest_run_id: None,
            validation_protocol_id: None,
            validation_report_id: None,
            diagnostics: Vec::new(),
            created_at_ms: 1,
            updated_at_ms: 1,
        };

        mark_interrupted_attempt(&mut attempt, 2);
        mark_interrupted_attempt(&mut attempt, 3);

        assert_eq!(attempt.status, StrategyQualificationAttemptStatus::Failed);
        assert_eq!(attempt.updated_at_ms, 2);
        assert_eq!(attempt.diagnostics.len(), 1);
        assert_eq!(
            attempt.diagnostics[0].code,
            "strategy-qualification-interrupted"
        );
    }

    #[test]
    fn context_matches_the_published_universe_snapshot_identity() {
        let revision = test_revision();
        let request: StrategyQualificationRunRequest = serde_json::from_value(serde_json::json!({
            "userId": "user",
            "candidateId": revision.candidate_id.clone(),
            "candidateRevision": 1,
            "snapshotId": "snapshot-1",
            "universeSnapshotId": "universe-1",
            "selectionWindow": {"startTimeMs": 0, "endTimeMs": 60_000},
            "finalWindow": {"startTimeMs": 120_000, "endTimeMs": 180_000},
            "signalInstances": [],
            "initialQuoteAllocation": "10000",
            "executionProfile": {
                "makerFeeRate": "0.0008",
                "takerFeeRate": "0.001",
                "adverseSlippageRate": "0",
                "rebalanceThreshold": "0",
                "priceIncrement": "0.0001",
                "quantityIncrement": "0.0001",
                "minimumQuantity": "0.0001",
                "riskFreeRate": "0",
                "fillPolicy": "taker"
            },
            "riskPolicy": {
                "policyId": "gate-11-default",
                "maxInstrumentWeight": "1",
                "maxTurnover": null
            },
            "seed": 0
        }))
        .unwrap();
        let snapshot: MarketDataSnapshot = serde_json::from_value(serde_json::json!({
            "snapshotId": "snapshot-1",
            "src": "okx",
            "code": "BTC-USDT",
            "interval": "1m",
            "startTimeMs": 0,
            "endTimeMs": 180_000,
            "barCount": 4,
            "gaps": [],
            "parquetPath": "/tmp/fixture.parquet"
        }))
        .unwrap();
        let universe: MarketDataUniverseSnapshot = serde_json::from_value(serde_json::json!({
            "snapshotId": "universe-1",
            "venue": {"id": "okx", "kind": "cryptoSpot", "timeZone": "UTC"},
            "interval": "1m",
            "startTimeMs": 0,
            "endTimeMs": 180_000,
            "universe": {
                "universeId": "content-universe-1",
                "asOfMs": 0,
                "evidenceState": "reconstructed",
                "evidenceReasons": ["fixture"],
                "coverageStartMs": 0,
                "coverageEndMs": 180_000,
                "instruments": []
            },
            "components": [],
            "qualityReportIds": [],
            "calendarSnapshotIds": [],
            "providerCapabilitySnapshots": [],
            "contentSha256": ""
        }))
        .unwrap();
        let bars = (0..4)
            .map(|index| adaq_data_core::OhlcvBar {
                open_time_ms: index * 60_000,
                open: Decimal::ONE,
                high: Decimal::ONE,
                low: Decimal::ONE,
                close: Decimal::ONE,
                base_volume: Decimal::ONE,
                quote_volume: Decimal::ONE,
            })
            .collect::<Vec<_>>();

        validate_context(&revision, &request, &snapshot, &universe, &bars).unwrap();
    }
}
