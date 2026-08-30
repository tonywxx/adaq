//! Host-owned Strategy Candidate definitions and immutable revisions.
//!
//! This module deliberately stays separate from Backtest's historical
//! StrategyProject. A Candidate is a reusable, pre-backtest research input
//! whose accepted upstream identities are resolved at the Host boundary.

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_python_research::sha256;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::user::validate_user;

pub(crate) const STRATEGY_CANDIDATE_SCHEMA_VERSION: &str = "adaq:strategy-candidate@1";
pub(crate) const STRATEGY_OPERATION_CATALOG_VERSION: &str = "adaq:strategy-operations@1";
const MAX_INPUT_SLOTS: usize = 16;
const MAX_NODES: usize = 32;
const MAX_ALIAS_BYTES: usize = 64;
const MAX_DIAGNOSTICS: usize = 32;
const MAX_DIAGNOSTIC_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrategyScope {
    SingleInstrument,
    Portfolio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub(crate) enum StrategyValue {
    Decimal(String),
    Integer(i64),
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrategyInputType {
    FactorScore,
    ForecastSignal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorInputBinding {
    pub decision_id: String,
    pub decision_hash: String,
    pub candidate_hash: String,
    pub output_name: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub component_id: String,
    pub component_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelInputBinding {
    pub qualification_report_id: String,
    pub decision_id: String,
    pub final_evaluation_report_id: String,
    pub artifact_sha256: String,
    pub transformation_sha256: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub component_id: String,
    pub component_version: String,
    pub model_profile: String,
    pub exporter_id: String,
    pub sdk_version: String,
    pub abi_version: String,
    pub runtime_identity: String,
    pub input_slots: Vec<String>,
    pub output_name: String,
    pub target_id: String,
    pub target_horizon_bars: u32,
    pub forecast_contract: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StrategyInputBinding {
    Factor(FactorInputBinding),
    Model(ModelInputBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyInputSlot {
    pub alias: String,
    pub input_type: StrategyInputType,
    pub binding: StrategyInputBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyOperationNode {
    pub node_id: String,
    pub operation: String,
    pub input_aliases: Vec<String>,
    pub parameters: BTreeMap<String, StrategyValue>,
    pub output_alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StrategyOutputContract {
    TargetDecision { node_id: String },
    PortfolioTarget { node_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyDefinition {
    pub schema_version: String,
    pub catalog_version: String,
    pub input_slots: Vec<StrategyInputSlot>,
    pub nodes: Vec<StrategyOperationNode>,
    pub output: StrategyOutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyCandidateDraft {
    #[serde(default)]
    pub candidate_id: Option<String>,
    pub scope: StrategyScope,
    pub definition: StrategyDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategySemanticContext {
    pub feature_plan_hash: String,
    pub research_context_hash: String,
    pub snapshot_id: String,
    pub universe_id: String,
    pub market: String,
    pub venue: String,
    pub input_evidence_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyCandidateRevision {
    pub candidate_id: String,
    pub revision: u64,
    pub scope: StrategyScope,
    pub definition: StrategyDefinition,
    pub semantic_context: StrategySemanticContext,
    pub created_at_ms: i64,
    pub created_by_attempt_id: String,
    pub revision_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrategyAttemptStatus {
    ReadyToCreate,
    Rejected,
    Published,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyDiagnostic {
    pub code: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StrategyCandidateState {
    Draft,
    FrozenRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyRevisionView {
    pub revision: StrategyCandidateRevision,
    pub eligible: bool,
    pub stale_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyAttemptView {
    pub attempt_id: String,
    pub candidate_id: String,
    pub status: StrategyAttemptStatus,
    pub diagnostics: Vec<StrategyDiagnostic>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyCandidateView {
    pub candidate_id: String,
    pub user_id: String,
    pub scope: StrategyScope,
    pub state: StrategyCandidateState,
    pub eligible: bool,
    pub revisions: Vec<StrategyRevisionView>,
    pub attempts: Vec<StrategyAttemptView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyPreflightResult {
    pub attempt_id: String,
    pub candidate_id: String,
    pub next_revision: u64,
    pub status: StrategyAttemptStatus,
    pub diagnostics: Vec<StrategyDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyCandidateCatalog {
    pub schema_version: String,
    pub catalog_version: String,
    pub operations: Vec<StrategyOperationSpec>,
    pub factor_inputs: Vec<ResolvedFactorInput>,
    pub model_inputs: Vec<ResolvedModelInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyOperationSpec {
    pub operation: String,
    pub scopes: Vec<StrategyScope>,
    pub parameters: Vec<StrategyParameterSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StrategyParameterSpec {
    pub name: String,
    pub value_type: String,
    pub default_value: StrategyValue,
    pub allowed_values: Vec<StrategyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedFactorInput {
    pub decision_id: String,
    pub decision_hash: String,
    pub candidate_hash: String,
    pub output_name: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub component_id: String,
    pub component_version: String,
    pub feature_plan_hash: String,
    pub context_hash: String,
    pub snapshot_id: String,
    pub universe_id: String,
    pub market: String,
    pub venue: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedModelInput {
    pub qualification_report_id: String,
    pub decision_id: String,
    pub final_evaluation_report_id: String,
    pub artifact_sha256: String,
    pub transformation_sha256: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub component_id: String,
    pub component_version: String,
    pub model_profile: String,
    pub exporter_id: String,
    pub sdk_version: String,
    pub abi_version: String,
    pub runtime_identity: String,
    pub input_slots: Vec<String>,
    pub output_name: String,
    pub target_id: String,
    pub target_horizon_bars: u32,
    pub forecast_contract: String,
    pub input_evidence_sha256: String,
}

pub(crate) trait StrategyCandidateSource: Send + Sync {
    fn factor_inputs(&self, user_id: &str) -> Result<Vec<ResolvedFactorInput>, String>;
    fn model_inputs(&self, user_id: &str) -> Result<Vec<ResolvedModelInput>, String>;
    fn resolve_factor(
        &self,
        user_id: &str,
        binding: &FactorInputBinding,
    ) -> Result<ResolvedFactorInput, String>;
    fn resolve_model(
        &self,
        user_id: &str,
        binding: &ModelInputBinding,
    ) -> Result<ResolvedModelInput, String>;
}

#[derive(Clone)]
pub(crate) struct StrategyCandidateStore {
    database: Arc<Mutex<Connection>>,
    source: Arc<dyn StrategyCandidateSource>,
}

impl StrategyCandidateStore {
    pub(crate) fn open(
        database: Arc<Mutex<Connection>>,
        source: Arc<dyn StrategyCandidateSource>,
    ) -> Result<Self, String> {
        database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS strategy_candidates (
                    candidate_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS strategy_candidate_revisions (
                    candidate_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    revision_hash TEXT NOT NULL,
                    revision_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(candidate_id, revision),
                    FOREIGN KEY(candidate_id) REFERENCES strategy_candidates(candidate_id)
                );
                CREATE TABLE IF NOT EXISTS strategy_candidate_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    candidate_id TEXT NOT NULL,
                    draft_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    diagnostics_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );",
            )
            .map_err(sql_error)?;
        Ok(Self { database, source })
    }

    pub(crate) fn catalog(&self, user_id: &str) -> Result<StrategyCandidateCatalog, String> {
        validate_user(user_id)?;
        Ok(StrategyCandidateCatalog {
            schema_version: STRATEGY_CANDIDATE_SCHEMA_VERSION.into(),
            catalog_version: STRATEGY_OPERATION_CATALOG_VERSION.into(),
            operations: operation_catalog(),
            factor_inputs: self.source.factor_inputs(user_id)?,
            model_inputs: self.source.model_inputs(user_id)?,
        })
    }

    pub(crate) fn preflight(
        &self,
        user_id: &str,
        mut draft: StrategyCandidateDraft,
    ) -> Result<StrategyPreflightResult, String> {
        validate_user(user_id)?;
        let candidate_id = draft
            .candidate_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        draft.candidate_id = Some(candidate_id.clone());
        let attempt_id = Uuid::new_v4().to_string();
        let (status, diagnostics) = match self.prepare(user_id, &draft) {
            Ok(()) => (StrategyAttemptStatus::ReadyToCreate, Vec::new()),
            Err(diagnostics) => (StrategyAttemptStatus::Rejected, diagnostics),
        };
        let next_revision = if status == StrategyAttemptStatus::ReadyToCreate {
            self.next_revision(user_id, &candidate_id, draft.scope)?
        } else {
            0
        };
        if Uuid::parse_str(&candidate_id).is_ok() {
            self.ensure_draft_candidate(user_id, &candidate_id, draft.scope)?;
        }
        self.record_attempt(
            user_id,
            &candidate_id,
            &attempt_id,
            &draft,
            status,
            &diagnostics,
        )?;
        Ok(StrategyPreflightResult {
            attempt_id,
            candidate_id,
            next_revision,
            status,
            diagnostics,
        })
    }

    pub(crate) fn retry(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<StrategyPreflightResult, String> {
        validate_user(user_id)?;
        let (candidate_id, status, draft_json) = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?
            .query_row(
                "SELECT candidate_id, status, draft_json
                   FROM strategy_candidate_attempts
                  WHERE user_id = ?1 AND attempt_id = ?2",
                params![user_id, attempt_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|_| "Strategy Candidate Attempt was not found".to_owned())?;
        if status != "rejected" {
            return Err("Only a rejected Strategy Candidate preflight can retry".into());
        }
        let mut draft: StrategyCandidateDraft =
            serde_json::from_str(&draft_json).map_err(json_error)?;
        draft.candidate_id = Some(candidate_id);
        self.preflight(user_id, draft)
    }

    pub(crate) fn create(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<StrategyCandidateView, String> {
        validate_user(user_id)?;
        let (candidate_id, status, draft_json) = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?
            .query_row(
                "SELECT candidate_id, status, draft_json
                   FROM strategy_candidate_attempts
                  WHERE user_id = ?1 AND attempt_id = ?2",
                params![user_id, attempt_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|_| "Strategy Candidate Attempt was not found".to_owned())?;
        if status != "ready-to-create" {
            return Err("Strategy Candidate Create requires a successful Host preflight".into());
        }
        let draft: StrategyCandidateDraft =
            serde_json::from_str(&draft_json).map_err(json_error)?;
        if draft.candidate_id.as_deref() != Some(candidate_id.as_str()) {
            return Err("Strategy Candidate Attempt identity is invalid".into());
        }
        if let Err(diagnostics) = self.prepare(user_id, &draft) {
            self.update_attempt_status(
                user_id,
                attempt_id,
                StrategyAttemptStatus::Rejected,
                &diagnostics,
            )?;
            return Err(format!(
                "strategy-candidate-preflight-invalid:{}",
                diagnostics
                    .first()
                    .map(|item| item.code.as_str())
                    .unwrap_or("unknown")
            ));
        }

        let revision_number = self.next_revision(user_id, &candidate_id, draft.scope)?;
        if revision_number == 0 {
            return Err("Strategy Candidate revision overflow".into());
        }
        let revision = self.build_revision(user_id, &draft, revision_number, attempt_id)?;
        let revision_json = serde_json::to_string(&revision).map_err(json_error)?;
        let now = unix_now_ms();
        let mut database = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
        let transaction = database.transaction().map_err(sql_error)?;
        let existing: Option<(String, String)> = transaction
            .query_row(
                "SELECT user_id, scope FROM strategy_candidates WHERE candidate_id = ?1",
                [candidate_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some((owner, scope)) = existing {
            if owner != user_id || scope != scope_name(draft.scope) {
                return Err("Strategy Candidate ownership or scope changed".into());
            }
        } else {
            transaction
                .execute(
                    "INSERT INTO strategy_candidates(candidate_id, user_id, scope, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![candidate_id, user_id, scope_name(draft.scope), now],
                )
                .map_err(sql_error)?;
        }
        if transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM strategy_candidate_revisions
                     WHERE candidate_id = ?1 AND revision = ?2
                )",
                params![revision.candidate_id, revision.revision as i64],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql_error)?
            != 0
        {
            return Err("Strategy Candidate revision identity already exists".into());
        }
        transaction
            .execute(
                "INSERT INTO strategy_candidate_revisions
                    (candidate_id, revision, revision_hash, revision_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    revision.candidate_id,
                    revision.revision as i64,
                    revision.revision_hash,
                    revision_json,
                    now
                ],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "UPDATE strategy_candidate_attempts
                    SET status = ?1, updated_at_ms = ?2
                  WHERE user_id = ?3 AND attempt_id = ?4 AND status = ?5",
                params![
                    status_name(StrategyAttemptStatus::Published),
                    now,
                    user_id,
                    attempt_id,
                    status_name(StrategyAttemptStatus::ReadyToCreate)
                ],
            )
            .map_err(sql_error)?;
        if transaction.changes() != 1 {
            return Err("Strategy Candidate Attempt was already consumed".into());
        }
        transaction.commit().map_err(sql_error)?;
        drop(database);
        self.get(user_id, &candidate_id)
    }

    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<StrategyCandidateView>, String> {
        validate_user(user_id)?;
        let ids = {
            let database = self
                .database
                .lock()
                .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
            let mut statement = database
                .prepare(
                    "SELECT candidate_id FROM strategy_candidates
                      WHERE user_id = ?1 ORDER BY candidate_id ASC",
                )
                .map_err(sql_error)?;
            statement
                .query_map([user_id], |row| row.get::<_, String>(0))
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        ids.into_iter()
            .map(|candidate_id| self.get(user_id, &candidate_id))
            .collect()
    }

    pub(crate) fn get(
        &self,
        user_id: &str,
        candidate_id: &str,
    ) -> Result<StrategyCandidateView, String> {
        validate_user(user_id)?;
        let (owner, scope, revisions, attempts) = {
            let database = self
                .database
                .lock()
                .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
            let (owner, scope): (String, String) = database
                .query_row(
                    "SELECT user_id, scope FROM strategy_candidates
                      WHERE candidate_id = ?1",
                    [candidate_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| "Strategy Candidate was not found".to_owned())?;
            if owner != user_id {
                return Err("Strategy Candidate is not available to this User".into());
            }
            let mut revision_statement = database
                .prepare(
                    "SELECT revision_json FROM strategy_candidate_revisions
                      WHERE candidate_id = ?1 ORDER BY revision ASC",
                )
                .map_err(sql_error)?;
            let revisions = revision_statement
                .query_map([candidate_id], |row| row.get::<_, String>(0))
                .map_err(sql_error)?
                .map(|row| {
                    row.map_err(sql_error)
                        .and_then(|json| serde_json::from_str(&json).map_err(json_error))
                        .and_then(|revision: StrategyCandidateRevision| {
                            revision.validate()?;
                            Ok(revision)
                        })
                })
                .collect::<Result<Vec<StrategyCandidateRevision>, String>>()?;
            for (index, revision) in revisions.iter().enumerate() {
                if revision.revision != index as u64 + 1 {
                    return Err("Strategy Candidate revision sequence is corrupt".into());
                }
            }
            let mut attempt_statement = database
                .prepare(
                    "SELECT attempt_id, candidate_id, status, diagnostics_json,
                            created_at_ms, updated_at_ms
                       FROM strategy_candidate_attempts
                      WHERE user_id = ?1 AND candidate_id = ?2
                      ORDER BY created_at_ms ASC, attempt_id ASC",
                )
                .map_err(sql_error)?;
            let attempts = attempt_statement
                .query_map(params![user_id, candidate_id], |row| {
                    let status: String = row.get(2)?;
                    let diagnostics_json: String = row.get(3)?;
                    Ok(StrategyAttemptView {
                        attempt_id: row.get(0)?,
                        candidate_id: row.get(1)?,
                        status: parse_status(&status).map_err(|error| {
                            rusqlite::Error::ToSqlConversionFailure(error.into())
                        })?,
                        diagnostics: serde_json::from_str(&diagnostics_json).map_err(|error| {
                            rusqlite::Error::ToSqlConversionFailure(error.into())
                        })?,
                        created_at_ms: row.get(4)?,
                        updated_at_ms: row.get(5)?,
                    })
                })
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?;
            (owner, parse_scope(&scope)?, revisions, attempts)
        };
        let revision_views = revisions
            .into_iter()
            .map(|revision| {
                let stale_reason = self.revision_stale_reason(user_id, &revision);
                StrategyRevisionView {
                    revision,
                    eligible: stale_reason.is_none(),
                    stale_reason,
                }
            })
            .collect::<Vec<_>>();
        let eligible = revision_views.iter().any(|revision| revision.eligible);
        Ok(StrategyCandidateView {
            candidate_id: candidate_id.into(),
            user_id: owner,
            scope,
            state: if revision_views.is_empty() {
                StrategyCandidateState::Draft
            } else {
                StrategyCandidateState::FrozenRevision
            },
            eligible,
            revisions: revision_views,
            attempts,
        })
    }

    pub(crate) fn reset_user(&self, user_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
        let transaction = database.transaction().map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM strategy_candidate_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM strategy_candidate_revisions
                  WHERE candidate_id IN (
                    SELECT candidate_id FROM strategy_candidates WHERE user_id = ?1
                  )",
                [user_id],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM strategy_candidates WHERE user_id = ?1",
                [user_id],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)
    }

    fn prepare(
        &self,
        user_id: &str,
        draft: &StrategyCandidateDraft,
    ) -> Result<(), Vec<StrategyDiagnostic>> {
        let Some(candidate_id) = draft.candidate_id.as_deref() else {
            return Err(vec![diagnostic(
                "strategy-candidate-id-missing",
                "candidateId",
            )]);
        };
        if Uuid::parse_str(candidate_id).is_err() {
            return Err(vec![diagnostic(
                "strategy-candidate-id-invalid",
                "candidateId",
            )]);
        }
        if let Err(diagnostic) = draft.definition.validate(draft.scope) {
            return Err(vec![diagnostic]);
        }
        let mut source_diagnostics = Vec::new();
        for (index, slot) in draft.definition.input_slots.iter().enumerate() {
            match &slot.binding {
                StrategyInputBinding::Factor(binding) => {
                    match self.source.resolve_factor(user_id, binding) {
                        Ok(resolved) if resolved_matches_factor(&resolved, binding) => {}
                        Ok(_) => source_diagnostics.push(diagnostic(
                            "strategy-factor-input-hash-mismatch",
                            &format!("definition.inputSlots[{index}]"),
                        )),
                        Err(_) => source_diagnostics.push(diagnostic(
                            "strategy-factor-input-not-accepted",
                            &format!("definition.inputSlots[{index}]"),
                        )),
                    }
                }
                StrategyInputBinding::Model(binding) => {
                    match self.source.resolve_model(user_id, binding) {
                        Ok(resolved) if resolved_matches_model(&resolved, binding) => {}
                        Ok(_) => source_diagnostics.push(diagnostic(
                            "strategy-model-input-hash-mismatch",
                            &format!("definition.inputSlots[{index}]"),
                        )),
                        Err(_) => source_diagnostics.push(diagnostic(
                            "strategy-model-input-not-accepted",
                            &format!("definition.inputSlots[{index}]"),
                        )),
                    }
                }
            }
        }
        if source_diagnostics.is_empty() {
            if let Err(diagnostic) = self.semantic_context(user_id, &draft.definition) {
                source_diagnostics.push(diagnostic);
            }
        }
        if source_diagnostics.is_empty() {
            let database = self.database.lock().map_err(|_| {
                vec![diagnostic(
                    "strategy-candidate-database-unavailable",
                    "candidateId",
                )]
            })?;
            let existing: Option<(String, String)> = database
                .query_row(
                    "SELECT user_id, scope FROM strategy_candidates WHERE candidate_id = ?1",
                    [candidate_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| {
                    vec![diagnostic(
                        "strategy-candidate-database-unavailable",
                        "candidateId",
                    )]
                })?;
            if let Some((owner, scope)) = existing {
                if owner != user_id {
                    source_diagnostics
                        .push(diagnostic("strategy-candidate-not-owned", "candidateId"));
                } else if scope != scope_name(draft.scope) {
                    source_diagnostics.push(diagnostic("strategy-scope-immutable", "scope"));
                }
            }
        }
        if source_diagnostics.is_empty() {
            Ok(())
        } else {
            Err(limit_diagnostics(source_diagnostics))
        }
    }

    fn semantic_context(
        &self,
        user_id: &str,
        definition: &StrategyDefinition,
    ) -> Result<StrategySemanticContext, StrategyDiagnostic> {
        let mut factors = Vec::new();
        let mut models = Vec::new();
        for slot in &definition.input_slots {
            match &slot.binding {
                StrategyInputBinding::Factor(binding) => self
                    .source
                    .resolve_factor(user_id, binding)
                    .map(|resolved| factors.push(resolved))
                    .map_err(|_| diagnostic("strategy-factor-input-not-accepted", "definition"))?,
                StrategyInputBinding::Model(binding) => self
                    .source
                    .resolve_model(user_id, binding)
                    .map(|resolved| models.push(resolved))
                    .map_err(|_| diagnostic("strategy-model-input-not-accepted", "definition"))?,
            }
        }
        let factor = factors
            .first()
            .ok_or_else(|| diagnostic("strategy-factor-input-required", "definition.inputSlots"))?;
        if factors.iter().any(|item| {
            item.feature_plan_hash != factor.feature_plan_hash
                || item.context_hash != factor.context_hash
                || item.snapshot_id != factor.snapshot_id
                || item.universe_id != factor.universe_id
                || item.market != factor.market
                || item.venue != factor.venue
        }) {
            return Err(diagnostic(
                "strategy-input-semantic-context-mismatch",
                "definition.inputSlots",
            ));
        }
        if models.is_empty() {
            return Err(diagnostic(
                "strategy-model-input-required",
                "definition.inputSlots",
            ));
        }
        let mut hashes = factors
            .iter()
            .map(|item| item.decision_hash.clone())
            .collect::<Vec<_>>();
        hashes.extend(models.iter().map(|item| item.input_evidence_sha256.clone()));
        Ok(StrategySemanticContext {
            feature_plan_hash: factor.feature_plan_hash.clone(),
            research_context_hash: factor.context_hash.clone(),
            snapshot_id: factor.snapshot_id.clone(),
            universe_id: factor.universe_id.clone(),
            market: factor.market.clone(),
            venue: factor.venue.clone(),
            input_evidence_hashes: hashes,
        })
    }

    fn build_revision(
        &self,
        user_id: &str,
        draft: &StrategyCandidateDraft,
        revision_number: u64,
        attempt_id: &str,
    ) -> Result<StrategyCandidateRevision, String> {
        let semantic_context = self
            .semantic_context(user_id, &draft.definition)
            .map_err(|diagnostic| diagnostic.code)?;
        let mut revision = StrategyCandidateRevision {
            candidate_id: draft
                .candidate_id
                .clone()
                .ok_or_else(|| "Strategy Candidate identity is missing".to_owned())?,
            revision: revision_number,
            scope: draft.scope,
            definition: draft.definition.clone(),
            semantic_context,
            created_at_ms: unix_now_ms(),
            created_by_attempt_id: attempt_id.into(),
            revision_hash: String::new(),
        };
        revision.revision_hash = revision_hash(&revision)?;
        Ok(revision)
    }

    fn next_revision(
        &self,
        user_id: &str,
        candidate_id: &str,
        scope: StrategyScope,
    ) -> Result<u64, String> {
        let database = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
        let existing: Option<(String, String)> = database
            .query_row(
                "SELECT user_id, scope FROM strategy_candidates WHERE candidate_id = ?1",
                [candidate_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some((owner, stored_scope)) = existing {
            if owner != user_id {
                return Err("Strategy Candidate is owned by another User".into());
            }
            if stored_scope != scope_name(scope) {
                return Err("Strategy Candidate Scope is immutable".into());
            }
        }
        let mut statement = database
            .prepare(
                "SELECT revision FROM strategy_candidate_revisions
                  WHERE candidate_id = ?1 ORDER BY revision ASC",
            )
            .map_err(sql_error)?;
        let revisions = statement
            .query_map([candidate_id], |row| row.get::<_, i64>(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        if revisions
            .iter()
            .enumerate()
            .any(|(index, revision)| *revision != index as i64 + 1)
        {
            return Err("Strategy Candidate revision sequence is corrupt".into());
        }
        u64::try_from(revisions.len())
            .ok()
            .and_then(|revision| revision.checked_add(1))
            .ok_or_else(|| "Strategy Candidate revision overflow".into())
    }

    fn ensure_draft_candidate(
        &self,
        user_id: &str,
        candidate_id: &str,
        scope: StrategyScope,
    ) -> Result<(), String> {
        let database = self
            .database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?;
        let existing: Option<(String, String)> = database
            .query_row(
                "SELECT user_id, scope FROM strategy_candidates WHERE candidate_id = ?1",
                [candidate_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        match existing {
            Some((owner, stored_scope))
                if owner != user_id || stored_scope != scope_name(scope) =>
            {
                Ok(())
            }
            Some(_) => Ok(()),
            None => database
                .execute(
                    "INSERT INTO strategy_candidates(candidate_id, user_id, scope, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![candidate_id, user_id, scope_name(scope), unix_now_ms()],
                )
                .map(|_| ())
                .map_err(sql_error),
        }
    }

    fn record_attempt(
        &self,
        user_id: &str,
        candidate_id: &str,
        attempt_id: &str,
        draft: &StrategyCandidateDraft,
        status: StrategyAttemptStatus,
        diagnostics: &[StrategyDiagnostic],
    ) -> Result<(), String> {
        let now = unix_now_ms();
        let draft_json = serde_json::to_string(draft).map_err(json_error)?;
        let diagnostics_json = serde_json::to_string(diagnostics).map_err(json_error)?;
        self.database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?
            .execute(
                "INSERT INTO strategy_candidate_attempts
                    (attempt_id, user_id, candidate_id, draft_json, status,
                     diagnostics_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    attempt_id,
                    user_id,
                    candidate_id,
                    draft_json,
                    status_name(status),
                    diagnostics_json,
                    now
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    fn update_attempt_status(
        &self,
        user_id: &str,
        attempt_id: &str,
        status: StrategyAttemptStatus,
        diagnostics: &[StrategyDiagnostic],
    ) -> Result<(), String> {
        let diagnostics_json = serde_json::to_string(diagnostics).map_err(json_error)?;
        self.database
            .lock()
            .map_err(|_| "Strategy Candidate database lock poisoned".to_owned())?
            .execute(
                "UPDATE strategy_candidate_attempts
                    SET status = ?1, diagnostics_json = ?2, updated_at_ms = ?3
                  WHERE user_id = ?4 AND attempt_id = ?5",
                params![
                    status_name(status),
                    diagnostics_json,
                    unix_now_ms(),
                    user_id,
                    attempt_id
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    fn revision_stale_reason(
        &self,
        user_id: &str,
        revision: &StrategyCandidateRevision,
    ) -> Option<String> {
        for slot in &revision.definition.input_slots {
            let result = match &slot.binding {
                StrategyInputBinding::Factor(binding) => self
                    .source
                    .resolve_factor(user_id, binding)
                    .map(|resolved| resolved_matches_factor(&resolved, binding)),
                StrategyInputBinding::Model(binding) => self
                    .source
                    .resolve_model(user_id, binding)
                    .map(|resolved| resolved_matches_model(&resolved, binding)),
            };
            match result {
                Ok(true) => {}
                Ok(false) => return Some("strategy-upstream-hash-mismatch".into()),
                Err(_) => return Some("strategy-upstream-stale-or-superseded".into()),
            }
        }
        None
    }
}

fn operation_catalog() -> Vec<StrategyOperationSpec> {
    vec![
        StrategyOperationSpec {
            operation: "weighted-sum".into(),
            scopes: vec![StrategyScope::SingleInstrument, StrategyScope::Portfolio],
            parameters: vec![StrategyParameterSpec {
                name: "forecast-weight".into(),
                value_type: "decimal".into(),
                default_value: StrategyValue::Decimal("0.7".into()),
                allowed_values: vec![
                    StrategyValue::Decimal("0.5".into()),
                    StrategyValue::Decimal("0.7".into()),
                ],
            }],
        },
        StrategyOperationSpec {
            operation: "top-n".into(),
            scopes: vec![StrategyScope::Portfolio],
            parameters: vec![StrategyParameterSpec {
                name: "top-n".into(),
                value_type: "integer".into(),
                default_value: StrategyValue::Integer(3),
                allowed_values: vec![StrategyValue::Integer(3), StrategyValue::Integer(5)],
            }],
        },
        StrategyOperationSpec {
            operation: "equal-weight".into(),
            scopes: vec![StrategyScope::Portfolio],
            parameters: Vec::new(),
        },
        StrategyOperationSpec {
            operation: "cash-reserve".into(),
            scopes: vec![StrategyScope::Portfolio],
            parameters: vec![StrategyParameterSpec {
                name: "cash-reserve".into(),
                value_type: "decimal".into(),
                default_value: StrategyValue::Decimal("0.1".into()),
                allowed_values: vec![
                    StrategyValue::Decimal("0".into()),
                    StrategyValue::Decimal("0.1".into()),
                ],
            }],
        },
    ]
}

impl StrategyDefinition {
    fn validate(&self, scope: StrategyScope) -> Result<(), StrategyDiagnostic> {
        if self.schema_version != STRATEGY_CANDIDATE_SCHEMA_VERSION {
            return Err(diagnostic(
                "strategy-schema-version-unsupported",
                "definition.schemaVersion",
            ));
        }
        if self.catalog_version != STRATEGY_OPERATION_CATALOG_VERSION {
            return Err(diagnostic(
                "strategy-operation-catalog-unsupported",
                "definition.catalogVersion",
            ));
        }
        if self.input_slots.is_empty() || self.input_slots.len() > MAX_INPUT_SLOTS {
            return Err(diagnostic(
                "strategy-input-slots-invalid",
                "definition.inputSlots",
            ));
        }
        let mut aliases = BTreeSet::new();
        let mut factor_count = 0;
        let mut model_count = 0;
        for (index, slot) in self.input_slots.iter().enumerate() {
            if !valid_alias(&slot.alias) || !aliases.insert(slot.alias.clone()) {
                return Err(diagnostic(
                    "strategy-input-alias-invalid-or-duplicate",
                    &format!("definition.inputSlots[{index}].alias"),
                ));
            }
            match (&slot.input_type, &slot.binding) {
                (StrategyInputType::FactorScore, StrategyInputBinding::Factor(binding)) => {
                    factor_count += 1;
                    validate_factor_binding(binding).map_err(|code| {
                        diagnostic(code, &format!("definition.inputSlots[{index}].binding"))
                    })?;
                }
                (StrategyInputType::ForecastSignal, StrategyInputBinding::Model(binding)) => {
                    model_count += 1;
                    validate_model_binding(binding).map_err(|code| {
                        diagnostic(code, &format!("definition.inputSlots[{index}].binding"))
                    })?;
                }
                _ => {
                    return Err(diagnostic(
                        "strategy-input-type-binding-mismatch",
                        &format!("definition.inputSlots[{index}]"),
                    ));
                }
            }
        }
        if factor_count == 0 {
            return Err(diagnostic(
                "strategy-factor-input-required",
                "definition.inputSlots",
            ));
        }
        if model_count == 0 {
            return Err(diagnostic(
                "strategy-model-input-required",
                "definition.inputSlots",
            ));
        }
        if self.nodes.is_empty() || self.nodes.len() > MAX_NODES {
            return Err(diagnostic("strategy-nodes-invalid", "definition.nodes"));
        }
        let mut node_ids = BTreeSet::new();
        let mut node_outputs = BTreeSet::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if !valid_alias(&node.node_id) || !node_ids.insert(node.node_id.clone()) {
                return Err(diagnostic(
                    "strategy-node-id-invalid-or-duplicate",
                    &format!("definition.nodes[{index}].nodeId"),
                ));
            }
            if !valid_alias(&node.output_alias) || !node_outputs.insert(node.output_alias.clone()) {
                return Err(diagnostic(
                    "strategy-node-output-invalid-or-duplicate",
                    &format!("definition.nodes[{index}].outputAlias"),
                ));
            }
            if !operation_catalog().iter().any(|operation| {
                operation.operation == node.operation && operation.scopes.contains(&scope)
            }) {
                return Err(diagnostic(
                    "strategy-operation-unsupported-for-scope",
                    &format!("definition.nodes[{index}].operation"),
                ));
            }
            for (input_index, input_alias) in node.input_aliases.iter().enumerate() {
                let known_input = aliases.contains(input_alias)
                    || self.nodes[..index]
                        .iter()
                        .any(|previous| previous.output_alias == *input_alias);
                if !valid_alias(input_alias) || !known_input {
                    return Err(diagnostic(
                        "strategy-node-input-must-reference-earlier-value",
                        &format!("definition.nodes[{index}].inputAliases[{input_index}]"),
                    ));
                }
            }
            validate_node_parameters(node, &format!("definition.nodes[{index}].parameters"))?;
        }
        let last = self.nodes.last().expect("nodes checked non-empty");
        match (&self.output, scope, last.operation.as_str()) {
            (
                StrategyOutputContract::TargetDecision { node_id },
                StrategyScope::SingleInstrument,
                "weighted-sum",
            ) if node_id == &last.node_id => {}
            (
                StrategyOutputContract::PortfolioTarget { node_id },
                StrategyScope::Portfolio,
                operation,
            ) if node_id == &last.node_id
                && matches!(operation, "top-n" | "equal-weight" | "cash-reserve") => {}
            _ => {
                return Err(diagnostic(
                    "strategy-output-contract-incomplete",
                    "definition.output",
                ));
            }
        }
        Ok(())
    }
}

impl StrategyCandidateRevision {
    fn validate(&self) -> Result<(), String> {
        if Uuid::parse_str(&self.candidate_id).is_err()
            || self.revision == 0
            || self.created_by_attempt_id.trim().is_empty()
            || !is_sha256(&self.semantic_context.feature_plan_hash)
            || !is_sha256(&self.semantic_context.research_context_hash)
            || self.semantic_context.snapshot_id.trim().is_empty()
            || self.semantic_context.universe_id.trim().is_empty()
            || self.semantic_context.market.trim().is_empty()
            || self.semantic_context.venue.trim().is_empty()
            || self.semantic_context.input_evidence_hashes.is_empty()
            || self
                .semantic_context
                .input_evidence_hashes
                .iter()
                .any(|hash| !is_sha256(hash))
            || !is_sha256(&self.revision_hash)
            || self.definition.validate(self.scope).is_err()
            || revision_hash(self)? != self.revision_hash
        {
            return Err("Strategy Candidate Revision is invalid".into());
        }
        Ok(())
    }
}

fn validate_node_parameters(
    node: &StrategyOperationNode,
    path: &str,
) -> Result<(), StrategyDiagnostic> {
    let expected = operation_catalog()
        .into_iter()
        .find(|operation| operation.operation == node.operation)
        .ok_or_else(|| diagnostic("strategy-operation-unsupported", path))?;
    if node.parameters.len() != expected.parameters.len()
        || node.parameters.keys().any(|name| {
            !expected
                .parameters
                .iter()
                .any(|parameter| parameter.name == *name)
        })
    {
        return Err(diagnostic("strategy-operation-parameters-invalid", path));
    }
    for parameter in expected.parameters {
        let value = node
            .parameters
            .get(&parameter.name)
            .ok_or_else(|| diagnostic("strategy-operation-parameters-incomplete", path))?;
        if !parameter
            .allowed_values
            .iter()
            .any(|allowed| allowed == value)
        {
            return Err(diagnostic("strategy-operation-parameter-not-allowed", path));
        }
        if parameter.value_type == "decimal"
            && !matches!(value, StrategyValue::Decimal(value) if Decimal::from_str(value).is_ok())
        {
            return Err(diagnostic("strategy-operation-parameter-not-finite", path));
        }
        if parameter.value_type == "integer" && !matches!(value, StrategyValue::Integer(_)) {
            return Err(diagnostic(
                "strategy-operation-parameter-type-invalid",
                path,
            ));
        }
    }
    Ok(())
}

fn validate_factor_binding(binding: &FactorInputBinding) -> Result<(), &'static str> {
    if Uuid::parse_str(&binding.decision_id).is_err()
        || !is_sha256(&binding.decision_hash)
        || !is_sha256(&binding.candidate_hash)
        || !valid_alias(&binding.output_name)
        || !is_sha256(&binding.package_archive_sha256)
        || !is_sha256(&binding.package_wasm_sha256)
        || Uuid::parse_str(&binding.component_id).is_err()
        || binding.component_version.trim().is_empty()
    {
        Err("strategy-factor-binding-invalid")
    } else {
        Ok(())
    }
}

fn validate_model_binding(binding: &ModelInputBinding) -> Result<(), &'static str> {
    if !is_sha256(&binding.qualification_report_id)
        || !is_sha256(&binding.decision_id)
        || !is_sha256(&binding.final_evaluation_report_id)
        || !is_sha256(&binding.artifact_sha256)
        || !is_sha256(&binding.transformation_sha256)
        || !is_sha256(&binding.package_archive_sha256)
        || !is_sha256(&binding.package_wasm_sha256)
        || Uuid::parse_str(&binding.component_id).is_err()
        || binding.component_version.trim().is_empty()
        || binding.model_profile.trim().is_empty()
        || binding.exporter_id.trim().is_empty()
        || binding.sdk_version.trim().is_empty()
        || binding.abi_version.trim().is_empty()
        || binding.runtime_identity.trim().is_empty()
        || binding.input_slots.is_empty()
        || binding.input_slots.iter().any(|slot| !valid_alias(slot))
        || !valid_alias(&binding.output_name)
        || binding.target_id.trim().is_empty()
        || binding.target_horizon_bars == 0
        || binding.forecast_contract.trim().is_empty()
    {
        Err("strategy-model-binding-invalid")
    } else {
        Ok(())
    }
}

fn resolved_matches_factor(resolved: &ResolvedFactorInput, binding: &FactorInputBinding) -> bool {
    resolved.decision_id == binding.decision_id
        && resolved.decision_hash == binding.decision_hash
        && resolved.candidate_hash == binding.candidate_hash
        && resolved.output_name == binding.output_name
        && resolved.package_archive_sha256 == binding.package_archive_sha256
        && resolved.package_wasm_sha256 == binding.package_wasm_sha256
        && resolved.component_id == binding.component_id
        && resolved.component_version == binding.component_version
}

fn resolved_matches_model(resolved: &ResolvedModelInput, binding: &ModelInputBinding) -> bool {
    resolved.qualification_report_id == binding.qualification_report_id
        && resolved.decision_id == binding.decision_id
        && resolved.final_evaluation_report_id == binding.final_evaluation_report_id
        && resolved.artifact_sha256 == binding.artifact_sha256
        && resolved.transformation_sha256 == binding.transformation_sha256
        && resolved.package_archive_sha256 == binding.package_archive_sha256
        && resolved.package_wasm_sha256 == binding.package_wasm_sha256
        && resolved.component_id == binding.component_id
        && resolved.component_version == binding.component_version
        && resolved.model_profile == binding.model_profile
        && resolved.exporter_id == binding.exporter_id
        && resolved.sdk_version == binding.sdk_version
        && resolved.abi_version == binding.abi_version
        && resolved.runtime_identity == binding.runtime_identity
        && resolved.input_slots == binding.input_slots
        && resolved.output_name == binding.output_name
        && resolved.target_id == binding.target_id
        && resolved.target_horizon_bars == binding.target_horizon_bars
        && resolved.forecast_contract == binding.forecast_contract
}

fn revision_hash(revision: &StrategyCandidateRevision) -> Result<String, String> {
    let mut content = revision.clone();
    content.created_at_ms = 0;
    content.created_by_attempt_id.clear();
    content.revision_hash.clear();
    serde_json::to_vec(&content)
        .map(|bytes| sha256(&bytes))
        .map_err(json_error)
}

fn valid_alias(value: &str) -> bool {
    value.len() <= MAX_ALIAS_BYTES
        && !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && !(index == 0 && byte == b'-')
        })
        && !value.ends_with('-')
        && !value.contains("--")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn scope_name(scope: StrategyScope) -> &'static str {
    match scope {
        StrategyScope::SingleInstrument => "single-instrument",
        StrategyScope::Portfolio => "portfolio",
    }
}

fn parse_scope(value: &str) -> Result<StrategyScope, String> {
    match value {
        "single-instrument" => Ok(StrategyScope::SingleInstrument),
        "portfolio" => Ok(StrategyScope::Portfolio),
        _ => Err("Strategy Candidate Scope is invalid".into()),
    }
}

fn status_name(status: StrategyAttemptStatus) -> &'static str {
    match status {
        StrategyAttemptStatus::ReadyToCreate => "ready-to-create",
        StrategyAttemptStatus::Rejected => "rejected",
        StrategyAttemptStatus::Published => "published",
    }
}

fn parse_status(value: &str) -> Result<StrategyAttemptStatus, String> {
    match value {
        "ready-to-create" => Ok(StrategyAttemptStatus::ReadyToCreate),
        "rejected" => Ok(StrategyAttemptStatus::Rejected),
        "published" => Ok(StrategyAttemptStatus::Published),
        _ => Err("Strategy Candidate Attempt status is invalid".into()),
    }
}

fn diagnostic(code: impl Into<String>, path: impl Into<String>) -> StrategyDiagnostic {
    StrategyDiagnostic {
        code: code.into(),
        path: path.into(),
    }
}

fn limit_diagnostics(mut diagnostics: Vec<StrategyDiagnostic>) -> Vec<StrategyDiagnostic> {
    diagnostics.truncate(MAX_DIAGNOSTICS);
    for diagnostic in &mut diagnostics {
        diagnostic.code.truncate(MAX_DIAGNOSTIC_BYTES);
        diagnostic.path.truncate(MAX_DIAGNOSTIC_BYTES);
    }
    diagnostics
}

fn sql_error(error: rusqlite::Error) -> String {
    format!("Strategy Candidate storage error: {error}")
}

fn json_error(error: impl std::fmt::Display) -> String {
    format!("Strategy Candidate JSON error: {error}")
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyCandidateDraftRequest {
    pub draft: StrategyCandidateDraft,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyCandidateAttemptRequest {
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StrategyCandidateIdRequest {
    pub candidate_id: String,
}

#[tauri::command]
pub(crate) async fn strategy_candidate_catalog(
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<StrategyCandidateCatalog, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.catalog(&user_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_candidate_preflight(
    request: StrategyCandidateDraftRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<StrategyPreflightResult, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.preflight(&user_id, request.draft))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_candidate_create(
    request: StrategyCandidateAttemptRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<StrategyCandidateView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.create(&user_id, &request.attempt_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_candidate_retry(
    request: StrategyCandidateAttemptRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<StrategyPreflightResult, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.retry(&user_id, &request.attempt_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_candidate_list(
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<Vec<StrategyCandidateView>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.list(&user_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn strategy_candidate_get(
    request: StrategyCandidateIdRequest,
    window: tauri::WebviewWindow,
    auth: tauri::State<'_, crate::auth::AuthState>,
    state: tauri::State<'_, Arc<StrategyCandidateStore>>,
) -> Result<StrategyCandidateView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.get(&user_id, &request.candidate_id))
        .await
        .map_err(|error| error.to_string())?
}
