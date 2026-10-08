//! Immutable Paper evidence and human-reviewed research feedback.
//!
//! This boundary intentionally stores references and evidence state, not mutable
//! projections from the running Bot. Reports and review decisions are append-only.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FeedbackLens {
    Factor,
    Model,
    Strategy,
    Execution,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceState {
    NotYetRealized,
    InsufficientEvidence,
    Ready,
    Unknown,
    Missing,
    Incompatible,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewAction {
    NoChange,
    PauseBot,
    NewFactorEvaluation,
    NewModelTraining,
    NewStrategyBacktest,
    InvestigateOperations,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeedbackSnapshotRequest {
    pub bundle_id: String,
    pub bot_id: String,
    pub attempt_id: String,
    pub observation_start_ms: i64,
    pub observation_end_ms: i64,
    pub realization_cutoff_ms: i64,
    pub required_observations: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeedbackReportRequest {
    pub snapshot_id: String,
    pub lens: FeedbackLens,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReviewDecisionRequest {
    pub report_ids: Vec<String>,
    pub action: ReviewAction,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSnapshotInput {
    pub user_id: String,
    pub bundle_id: String,
    pub bot_id: String,
    #[serde(default)]
    pub attempt_id: String,
    pub observation_start_ms: i64,
    pub observation_end_ms: i64,
    pub realization_cutoff_ms: i64,
    pub realized_observations: u64,
    pub required_observations: u64,
    pub evidence: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSnapshot {
    pub snapshot_id: String,
    pub input: FeedbackSnapshotInput,
    pub evidence_state: EvidenceState,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReportInput {
    pub user_id: String,
    pub snapshot_id: String,
    pub lens: FeedbackLens,
    pub metrics: Value,
    pub comparable_evidence_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReport {
    pub report_id: String,
    pub input: FeedbackReportInput,
    pub evidence_state: EvidenceState,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecisionInput {
    pub user_id: String,
    pub report_ids: Vec<String>,
    pub action: ReviewAction,
    pub rationale: String,
    pub decided_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub decision_id: String,
    pub input: ReviewDecisionInput,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperFeedbackView {
    pub snapshots: Vec<FeedbackSnapshot>,
    pub reports: Vec<FeedbackReport>,
    pub decisions: Vec<ReviewDecision>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FeedbackSection {
    Snapshots,
    Reports,
    Decisions,
}

#[derive(Clone)]
pub struct PaperFeedbackStore {
    database: Arc<Mutex<Connection>>,
}

impl PaperFeedbackStore {
    pub(crate) fn page(
        &self,
        user_id: &str,
        section: FeedbackSection,
        page: usize,
    ) -> Result<crate::ui_commands::RecordPage, String> {
        let (source, projection, order) = match section {
            FeedbackSection::Snapshots => (
                "paper_feedback_snapshots s WHERE s.user_id=?1 AND ?2 IS NULL",
                "json_object('snapshotId', s.snapshot_id,
                    'input', json_remove(s.payload_json, '$.evidence'),
                    'evidenceState', s.evidence_state, 'createdAtMs', s.created_at_ms,
                    'existingLenses', json((SELECT json_group_array(json_extract(r.payload_json, '$.lens'))
                        FROM paper_feedback_reports r WHERE r.user_id=s.user_id AND r.snapshot_id=s.snapshot_id)))",
                "s.created_at_ms DESC, s.snapshot_id DESC",
            ),
            FeedbackSection::Reports => (
                "paper_feedback_reports r WHERE r.user_id=?1 AND ?2 IS NULL",
                "json_object('reportId', r.report_id, 'input', json(r.payload_json),
                    'evidenceState', r.evidence_state, 'createdAtMs', r.created_at_ms)",
                "r.created_at_ms DESC, r.report_id DESC",
            ),
            FeedbackSection::Decisions => (
                "research_review_decisions d WHERE d.user_id=?1 AND ?2 IS NULL",
                "json_object('decisionId', d.decision_id, 'input', json(d.payload_json))",
                "d.decided_at_ms DESC, d.decision_id DESC",
            ),
        };
        let database = self.database.lock().map_err(|error| error.to_string())?;
        crate::ui_commands::RecordPage::read(
            &database, source, projection, order, user_id, None, page,
        )
    }

    pub fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        database.lock().map_err(|e| e.to_string())?.execute_batch(
            "CREATE TABLE IF NOT EXISTS paper_feedback_snapshots (
                snapshot_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, payload_json TEXT NOT NULL,
                evidence_state TEXT NOT NULL, created_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS paper_feedback_reports (
                report_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, snapshot_id TEXT NOT NULL,
                payload_json TEXT NOT NULL, evidence_state TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(snapshot_id) REFERENCES paper_feedback_snapshots(snapshot_id)
            );
            CREATE TABLE IF NOT EXISTS research_review_decisions (
                decision_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, payload_json TEXT NOT NULL,
                decided_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS paper_feedback_reports_user ON paper_feedback_reports(user_id);
            CREATE INDEX IF NOT EXISTS feedback_snapshots_page ON paper_feedback_snapshots(user_id, created_at_ms DESC, snapshot_id DESC);
            CREATE INDEX IF NOT EXISTS feedback_reports_page ON paper_feedback_reports(user_id, created_at_ms DESC, report_id DESC);
            CREATE INDEX IF NOT EXISTS feedback_reports_snapshot ON paper_feedback_reports(user_id, snapshot_id);
            CREATE INDEX IF NOT EXISTS feedback_decisions_page ON research_review_decisions(user_id, decided_at_ms DESC, decision_id DESC);
        ").map_err(|e| e.to_string())?;
        Ok(Self { database })
    }

    #[cfg(test)]
    pub fn create_snapshot(
        &self,
        input: FeedbackSnapshotInput,
        created_at_ms: i64,
    ) -> Result<FeedbackSnapshot, String> {
        self.create_snapshot_with_state(input, created_at_ms, None)
    }

    pub(crate) fn create_snapshot_with_state(
        &self,
        input: FeedbackSnapshotInput,
        created_at_ms: i64,
        host_state: Option<EvidenceState>,
    ) -> Result<FeedbackSnapshot, String> {
        validate_user_and_range(
            &input.user_id,
            input.observation_start_ms,
            input.observation_end_ms,
        )?;
        if input.bundle_id.trim().is_empty()
            || input.bot_id.trim().is_empty()
            || input.attempt_id.trim().is_empty()
            || input.realization_cutoff_ms < input.observation_end_ms
            || input.required_observations == 0
            || input.required_observations > 1_000_000
            || !input.evidence.is_object()
            || created_at_ms <= 0
        {
            return Err("invalid Paper Feedback Snapshot binding".into());
        }
        let state = host_state.unwrap_or_else(|| {
            if input.realized_observations == 0 {
                EvidenceState::NotYetRealized
            } else if input.realized_observations < input.required_observations {
                EvidenceState::InsufficientEvidence
            } else {
                EvidenceState::Ready
            }
        });
        let snapshot = FeedbackSnapshot {
            snapshot_id: Uuid::new_v4().to_string(),
            input,
            evidence_state: state,
            created_at_ms,
        };
        let payload = serde_json::to_string(&snapshot.input).map_err(|e| e.to_string())?;
        let state_name = enum_name(state)?;
        self.database
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "INSERT INTO paper_feedback_snapshots VALUES (?1,?2,?3,?4,?5)",
                params![
                    snapshot.snapshot_id,
                    snapshot.input.user_id,
                    payload,
                    state_name,
                    created_at_ms
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(snapshot)
    }

    #[cfg(test)]
    pub fn create_report(
        &self,
        input: FeedbackReportInput,
        created_at_ms: i64,
    ) -> Result<FeedbackReport, String> {
        let snapshot_state = {
            let conn = self.database.lock().map_err(|e| e.to_string())?;
            let value: String = conn
                .query_row(
                    "SELECT evidence_state FROM paper_feedback_snapshots WHERE snapshot_id=?1 AND user_id=?2",
                    params![input.snapshot_id, input.user_id],
                    |row| row.get(0),
                )
                .map_err(|_| "Paper Feedback Snapshot was not found for User".to_owned())?;
            parse_enum(&value)?
        };
        self.create_report_with_state(input, created_at_ms, snapshot_state)
    }

    pub(crate) fn generate_report(
        &self,
        user_id: &str,
        request: FeedbackReportRequest,
        metadata: serde_json::Map<String, Value>,
        comparable_evidence_id: Option<String>,
        created_at_ms: i64,
    ) -> Result<FeedbackReport, String> {
        let snapshot = self.snapshot_for_user(user_id, &request.snapshot_id)?;
        let mut metrics = paper_feedback_metrics(&snapshot, request.lens);
        let state = paper_feedback_report_state(&snapshot, request.lens, &metrics);
        if metadata.keys().any(|key| metrics.get(key).is_some()) {
            return Err("Paper Feedback Report metadata cannot replace generated metrics".into());
        }
        if let Some(object) = metrics.as_object_mut() {
            object.extend(metadata);
        }
        self.create_report_with_state(
            FeedbackReportInput {
                user_id: user_id.to_owned(),
                snapshot_id: request.snapshot_id,
                lens: request.lens,
                metrics,
                comparable_evidence_id,
            },
            created_at_ms,
            state,
        )
    }

    fn create_report_with_state(
        &self,
        input: FeedbackReportInput,
        created_at_ms: i64,
        state: EvidenceState,
    ) -> Result<FeedbackReport, String> {
        validate_user(&input.user_id)?;
        if created_at_ms <= 0 {
            return Err("invalid Paper Feedback Report timestamp".into());
        }
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let snapshot_exists: bool = conn.query_row(
            "SELECT evidence_state FROM paper_feedback_snapshots WHERE snapshot_id=?1 AND user_id=?2",
            params![input.snapshot_id, input.user_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .is_some();
        if !snapshot_exists {
            return Err("Paper Feedback Snapshot was not found for User".to_string());
        }
        if input.snapshot_id.trim().is_empty()
            || !input.metrics.is_object()
            || input
                .comparable_evidence_id
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err("invalid Paper Feedback Report binding".into());
        }
        let report = FeedbackReport {
            report_id: Uuid::new_v4().to_string(),
            input,
            evidence_state: state,
            created_at_ms,
        };
        let payload = serde_json::to_string(&report.input).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO paper_feedback_reports VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                report.report_id,
                report.input.user_id,
                report.input.snapshot_id,
                payload,
                enum_name(state)?,
                created_at_ms
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(report)
    }

    pub(crate) fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<FeedbackSnapshot, String> {
        validate_user(user_id)?;
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT payload_json, evidence_state, created_at_ms
                 FROM paper_feedback_snapshots WHERE snapshot_id=?1 AND user_id=?2",
                params![snapshot_id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Paper Feedback Snapshot was not found for User".to_owned())?;
        Ok(FeedbackSnapshot {
            snapshot_id: snapshot_id.to_owned(),
            input: serde_json::from_str(&row.0).map_err(|e| e.to_string())?,
            evidence_state: parse_enum(&row.1)?,
            created_at_ms: row.2,
        })
    }

    pub fn view(&self, user_id: &str) -> Result<PaperFeedbackView, String> {
        validate_user(user_id)?;
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let snapshots = {
            let mut statement = conn
                .prepare(
                    "SELECT snapshot_id, payload_json, evidence_state, created_at_ms
                     FROM paper_feedback_snapshots WHERE user_id=?1
                     ORDER BY created_at_ms DESC, snapshot_id DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([user_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            rows.map(|row| {
                let (snapshot_id, payload, state, created_at_ms) =
                    row.map_err(|e| e.to_string())?;
                Ok(FeedbackSnapshot {
                    snapshot_id,
                    input: serde_json::from_str(&payload).map_err(|e| e.to_string())?,
                    evidence_state: parse_enum(&state)?,
                    created_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
        };
        let reports = {
            let mut statement = conn
                .prepare(
                    "SELECT report_id, payload_json, evidence_state, created_at_ms
                     FROM paper_feedback_reports WHERE user_id=?1
                     ORDER BY created_at_ms DESC, report_id DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([user_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            rows.map(|row| {
                let (report_id, payload, state, created_at_ms) = row.map_err(|e| e.to_string())?;
                Ok(FeedbackReport {
                    report_id,
                    input: serde_json::from_str(&payload).map_err(|e| e.to_string())?,
                    evidence_state: parse_enum(&state)?,
                    created_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
        };
        let decisions = {
            let mut statement = conn
                .prepare(
                    "SELECT decision_id, payload_json
                     FROM research_review_decisions WHERE user_id=?1
                     ORDER BY decided_at_ms DESC, decision_id DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([user_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            rows.map(|row| {
                let (decision_id, payload) = row.map_err(|e| e.to_string())?;
                Ok(ReviewDecision {
                    decision_id,
                    input: serde_json::from_str(&payload).map_err(|e| e.to_string())?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
        };
        Ok(PaperFeedbackView {
            snapshots,
            reports,
            decisions,
        })
    }

    pub(crate) fn recover_reviewed_reports(
        &self,
        operations: &crate::operations::OperationsStore,
        user_id: &str,
    ) -> Result<(), String> {
        let view = self.view(user_id)?;
        for alert in operations.alerts_for_user(user_id)? {
            if alert.dimension != crate::operations::HealthDimension::ResearchFeedback
                || alert.condition != "Research Review Required"
                || alert.state == crate::operations::AlertState::Resolved
            {
                continue;
            }
            let Some(decision) = view
                .decisions
                .iter()
                .find(|decision| decision.input.report_ids.contains(&alert.entity_id))
            else {
                continue;
            };
            let Some(report) = view
                .reports
                .iter()
                .find(|report| report.report_id == alert.entity_id)
            else {
                continue;
            };
            operations.observe(crate::operations::HealthObservation {
                user_id: user_id.into(),
                entity_id: report.report_id.clone(),
                dimension: crate::operations::HealthDimension::ResearchFeedback,
                state: crate::operations::HealthState::Healthy,
                condition: alert.condition,
                evidence: serde_json::json!({
                    "reportId": report.report_id,
                    "snapshotId": report.input.snapshot_id,
                    "reviewDecisionId": decision.decision_id,
                    "action": decision.input.action,
                    "evidenceState": report.evidence_state,
                }),
                required: false,
                observed_at_ms: crate::unix_now_ms(),
                event_kind: Some("research.review-completed".into()),
                evidence_id: Some(report.report_id.clone()),
                correlation_id: Some(report.input.snapshot_id.clone()),
                causation_id: Some(alert.last_event_id),
                diagnostic: Some("A recorded Research Review Decision addressed this Report; its original evidence state remains unchanged.".into()),
                metrics: Default::default(),
            })?;
        }
        Ok(())
    }

    pub fn record_review_decision(
        &self,
        input: ReviewDecisionInput,
    ) -> Result<ReviewDecision, String> {
        validate_user(&input.user_id)?;
        if input.report_ids.is_empty()
            || input.rationale.trim().is_empty()
            || input.decided_at_ms <= 0
        {
            return Err("a review decision requires reports and rationale".into());
        }
        if input.rationale.chars().count() > 2_000
            || input.report_ids.iter().any(|id| id.trim().is_empty())
            || input.report_ids.len() > 64
        {
            return Err("invalid Research Review Decision".into());
        }
        let unique_report_ids = input.report_ids.iter().collect::<HashSet<_>>();
        if unique_report_ids.len() != input.report_ids.len() {
            return Err("a Research Review Decision cannot cite a Report twice".into());
        }
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        for report_id in &input.report_ids {
            let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM paper_feedback_reports WHERE report_id=?1 AND user_id=?2)", params![report_id, input.user_id], |row| row.get(0)).map_err(|e| e.to_string())?;
            if !exists {
                return Err("review decision must cite User-owned feedback reports".into());
            }
        }
        let decision = ReviewDecision {
            decision_id: Uuid::new_v4().to_string(),
            input,
        };
        let payload = serde_json::to_string(&decision.input).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO research_review_decisions VALUES (?1,?2,?3,?4)",
            params![
                decision.decision_id,
                decision.input.user_id,
                payload,
                decision.input.decided_at_ms
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(decision)
    }
}

fn validate_user(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() {
        Err("User is required".into())
    } else {
        Ok(())
    }
}
fn validate_user_and_range(user_id: &str, start: i64, end: i64) -> Result<(), String> {
    validate_user(user_id)?;
    if start > end {
        Err("feedback observation range is invalid".into())
    } else {
        Ok(())
    }
}
fn enum_name<T: Serialize>(value: T) -> Result<String, String> {
    Ok(serde_json::to_string(&value)
        .map_err(|e| e.to_string())?
        .trim_matches('"')
        .to_owned())
}

fn parse_enum<T: for<'de> serde::Deserialize<'de>>(value: &str) -> Result<T, String> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> PaperFeedbackStore {
        PaperFeedbackStore::open(Arc::new(Mutex::new(Connection::open_in_memory().unwrap())))
            .unwrap()
    }
    fn snapshot_input() -> FeedbackSnapshotInput {
        FeedbackSnapshotInput {
            user_id: "user".into(),
            bundle_id: "bundle-v1".into(),
            bot_id: "bot".into(),
            attempt_id: "attempt".into(),
            observation_start_ms: 10,
            observation_end_ms: 20,
            realization_cutoff_ms: 20,
            realized_observations: 2,
            required_observations: 3,
            evidence: serde_json::json!({"orders": 2}),
        }
    }
    #[test]
    fn recorded_review_resolves_only_its_owned_report_without_changing_evidence() {
        let s = store();
        let operations = crate::operations::OperationsStore::open(s.database.clone()).unwrap();
        let snapshot = s.create_snapshot(snapshot_input(), 1).unwrap();
        let mut reports = Vec::new();
        for lens in [FeedbackLens::Factor, FeedbackLens::Model] {
            let report = s
                .create_report(
                    FeedbackReportInput {
                        user_id: "user".into(),
                        snapshot_id: snapshot.snapshot_id.clone(),
                        lens,
                        metrics: serde_json::json!({}),
                        comparable_evidence_id: None,
                    },
                    2,
                )
                .unwrap();
            operations
                .observe(crate::operations::HealthObservation {
                    user_id: "user".into(),
                    entity_id: report.report_id.clone(),
                    dimension: crate::operations::HealthDimension::ResearchFeedback,
                    state: crate::operations::HealthState::Degraded,
                    condition: "Research Review Required".into(),
                    evidence: serde_json::json!({"reportId": report.report_id}),
                    required: true,
                    observed_at_ms: 2,
                    event_kind: None,
                    evidence_id: Some(report.report_id.clone()),
                    correlation_id: None,
                    causation_id: None,
                    diagnostic: None,
                    metrics: Default::default(),
                })
                .unwrap();
            reports.push(report);
        }
        s.recover_reviewed_reports(&operations, "user").unwrap();
        assert!(
            operations
                .alerts_for_user("user")
                .unwrap()
                .iter()
                .all(|a| a.state == crate::operations::AlertState::Active)
        );
        assert!(
            s.record_review_decision(ReviewDecisionInput {
                user_id: "foreign".into(),
                report_ids: vec![reports[0].report_id.clone()],
                action: ReviewAction::InvestigateOperations,
                rationale: "Unavailable evidence requires investigation.".into(),
                decided_at_ms: 3,
            })
            .is_err()
        );
        let decision = s
            .record_review_decision(ReviewDecisionInput {
                user_id: "user".into(),
                report_ids: vec![reports[0].report_id.clone()],
                action: ReviewAction::InvestigateOperations,
                rationale: "Unavailable evidence requires investigation.".into(),
                decided_at_ms: 3,
            })
            .unwrap();
        s.recover_reviewed_reports(&operations, "user").unwrap();
        s.recover_reviewed_reports(&operations, "user").unwrap();
        let alerts = operations.alerts_for_user("user").unwrap();
        let resolved = alerts
            .iter()
            .find(|a| a.entity_id == reports[0].report_id)
            .unwrap();
        assert_eq!(resolved.state, crate::operations::AlertState::Resolved);
        assert_eq!(
            operations
                .alert_history_for_user("user", &resolved.alert_id)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            alerts
                .iter()
                .find(|a| a.entity_id == reports[1].report_id)
                .unwrap()
                .state,
            crate::operations::AlertState::Active
        );
        let view = s.view("user").unwrap();
        assert!(
            view.reports
                .iter()
                .all(|r| r.evidence_state == EvidenceState::InsufficientEvidence)
        );
        assert_eq!(
            operations.events_for_user("user", 1).unwrap()[0].evidence["reviewDecisionId"],
            decision.decision_id
        );
    }

    #[test]
    fn snapshots_gate_unrealized_and_insufficient_evidence() {
        let s = store();
        let mut i = snapshot_input();
        i.realized_observations = 0;
        assert_eq!(
            s.create_snapshot(i, 1).unwrap().evidence_state,
            EvidenceState::NotYetRealized
        );
        assert_eq!(
            s.create_snapshot(snapshot_input(), 2)
                .unwrap()
                .evidence_state,
            EvidenceState::InsufficientEvidence
        );
    }
    #[test]
    fn reports_and_decisions_are_user_scoped_and_append_only() {
        let s = store();
        let snap = s.create_snapshot(snapshot_input(), 1).unwrap();
        assert!(
            s.create_report(
                FeedbackReportInput {
                    user_id: "other".into(),
                    snapshot_id: snap.snapshot_id.clone(),
                    lens: FeedbackLens::Execution,
                    metrics: serde_json::json!({}),
                    comparable_evidence_id: None,
                },
                2,
            )
            .is_err()
        );
        let report = s
            .create_report(
                FeedbackReportInput {
                    user_id: "user".into(),
                    snapshot_id: snap.snapshot_id,
                    lens: FeedbackLens::Execution,
                    metrics: serde_json::json!({"rejectRate": 0}),
                    comparable_evidence_id: None,
                },
                2,
            )
            .unwrap();
        let decision = s
            .record_review_decision(ReviewDecisionInput {
                user_id: "user".into(),
                report_ids: vec![report.report_id],
                action: ReviewAction::NoChange,
                rationale: "Evidence remains below the frozen sample threshold".into(),
                decided_at_ms: 3,
            })
            .unwrap();
        assert!(!decision.decision_id.is_empty());
        assert!(
            s.record_review_decision(ReviewDecisionInput {
                user_id: "other".into(),
                report_ids: vec!["missing".into()],
                action: ReviewAction::NoChange,
                rationale: "x".into(),
                decided_at_ms: 4
            })
            .is_err()
        );
    }

    #[test]
    fn view_rehydrates_immutable_records_and_host_state() {
        let s = store();
        let snapshot = s
            .create_snapshot_with_state(snapshot_input(), 1, Some(EvidenceState::Unknown))
            .unwrap();
        let report = s
            .create_report(
                FeedbackReportInput {
                    user_id: "user".into(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    lens: FeedbackLens::Factor,
                    metrics: serde_json::json!({
                        "directionalConclusion": false,
                        "retainedCounts": {"decisionBatches": 0}
                    }),
                    comparable_evidence_id: None,
                },
                2,
            )
            .unwrap();
        let decision = s
            .record_review_decision(ReviewDecisionInput {
                user_id: "user".into(),
                report_ids: vec![report.report_id.clone()],
                action: ReviewAction::InvestigateOperations,
                rationale: "Reconcile the retained account before drawing a conclusion".into(),
                decided_at_ms: 3,
            })
            .unwrap();

        let view = s.view("user").unwrap();
        assert_eq!(view.snapshots.len(), 1);
        assert_eq!(view.snapshots[0].evidence_state, EvidenceState::Unknown);
        assert_eq!(view.reports[0].evidence_state, EvidenceState::Unknown);
        assert_eq!(view.decisions[0].decision_id, decision.decision_id);
        assert!(s.view("other").unwrap().snapshots.is_empty());
    }

    #[test]
    fn snapshots_reject_unbounded_cutoff_and_missing_host_evidence() {
        let s = store();
        let mut input = snapshot_input();
        input.realization_cutoff_ms = 19;
        assert!(s.create_snapshot(input, 1).is_err());

        let mut input = snapshot_input();
        input.evidence = Value::Null;
        assert!(s.create_snapshot(input, 1).is_err());

        let mut input = snapshot_input();
        input.required_observations = 1_000_001;
        assert!(s.create_snapshot(input, 1).is_err());
    }

    #[test]
    fn generated_reports_persist_insufficient_evidence_for_every_lens() {
        let s = store();
        let mut input = snapshot_input();
        input.realized_observations = input.required_observations;
        input.evidence = serde_json::json!({
            "runtime": {"decisionBatchCount": 1},
            "market": {"candidateRowCount": 3, "rows": [{
                "targetAvailable": true, "targetReturn": "1",
                "factorOutputs": [{"name": "score", "value": "1"}],
                "modelOutputs": [{"name": "forecast", "value": "1"}],
                "targetOutcomeAvailable": true, "targetOutcomeReturn": "1"
            }, {
                "targetAvailable": true, "targetReturn": "2",
                "factorOutputs": [], "modelOutputs": [], "targetOutcomeAvailable": false
            }, {
                "targetAvailable": true, "targetReturn": "3",
                "factorOutputs": [], "modelOutputs": [], "targetOutcomeAvailable": false
            }]},
            "paper": {
                "orderCount": 1, "fillCount": 0, "riskDecisionCount": 0,
                "orders": [{"orderId": "local-1", "filledQuantity": "0", "limitPrice": "100"}],
                "fills": [],
                "providerEvidence": [{"outcome": "accepted", "providerOrderId": "remote-1", "localOrderId": "local-1"}],
                "reconciliation": "reconciled", "restartRequired": false, "scopeCompatible": true
            }
        });
        let snapshot = s
            .create_snapshot_with_state(input, 1, Some(EvidenceState::Ready))
            .unwrap();
        for lens in [
            FeedbackLens::Factor,
            FeedbackLens::Model,
            FeedbackLens::Strategy,
            FeedbackLens::Execution,
        ] {
            let report = s
                .generate_report(
                    "user",
                    FeedbackReportRequest {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        lens,
                    },
                    serde_json::Map::new(),
                    None,
                    2,
                )
                .unwrap();
            assert_eq!(report.evidence_state, EvidenceState::InsufficientEvidence);
            let persisted = s
                .view("user")
                .unwrap()
                .reports
                .into_iter()
                .find(|value| value.report_id == report.report_id)
                .unwrap();
            assert_eq!(persisted.input.lens, lens);
            assert_eq!(persisted.input.metrics, report.input.metrics);
            assert_eq!(
                persisted.evidence_state,
                EvidenceState::InsufficientEvidence
            );
        }
        assert_eq!(s.view("user").unwrap().reports.len(), 4);
        assert!(s.view("other").unwrap().reports.is_empty());
        assert_eq!(
            s.snapshot_for_user("user", &snapshot.snapshot_id)
                .unwrap()
                .evidence_state,
            EvidenceState::Ready
        );
    }

    #[test]
    fn generated_reports_keep_snapshot_authority_and_metadata_separate() {
        let s = store();
        let snapshot = s
            .create_snapshot_with_state(snapshot_input(), 1, Some(EvidenceState::Ready))
            .unwrap();
        let request = || FeedbackReportRequest {
            snapshot_id: snapshot.snapshot_id.clone(),
            lens: FeedbackLens::Factor,
        };
        let metadata =
            serde_json::Map::from_iter([("experimentId".into(), serde_json::json!("experiment"))]);
        let report = s
            .generate_report(
                "user",
                request(),
                metadata,
                Some("experiment-report".into()),
                2,
            )
            .unwrap();
        assert_eq!(report.evidence_state, EvidenceState::Missing);
        assert_eq!(report.input.metrics["experimentId"], "experiment");
        assert_eq!(report.input.metrics["directionalConclusion"], false);
        assert_eq!(
            report.input.metrics["lensMetrics"]["factorOutputsAvailable"],
            false
        );
        assert_eq!(
            report.input.comparable_evidence_id.as_deref(),
            Some("experiment-report")
        );
        assert_eq!(
            s.view("user").unwrap().reports[0].input.metrics,
            report.input.metrics
        );
        assert!(
            s.generate_report("other", request(), serde_json::Map::new(), None, 2)
                .is_err()
        );
        assert!(
            s.generate_report(
                "user",
                request(),
                serde_json::Map::from_iter([("lensMetrics".into(), serde_json::json!({}))]),
                None,
                2,
            )
            .is_err()
        );
        assert_eq!(s.view("user").unwrap().reports.len(), 1);
        assert_eq!(
            s.snapshot_for_user("user", &snapshot.snapshot_id)
                .unwrap()
                .evidence_state,
            EvidenceState::Ready
        );
    }
}

fn metric_number(value: &serde_json::Value) -> Option<f64> {
    let value = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))?;
    value.is_finite().then_some(value)
}

fn metric_pairs(rows: &[serde_json::Value], output_key: &str) -> BTreeMap<String, Vec<(f64, f64)>> {
    let mut pairs = BTreeMap::new();
    for row in rows {
        if row["targetAvailable"] != serde_json::Value::Bool(true) {
            continue;
        }
        let Some(target) = metric_number(&row["targetReturn"]) else {
            continue;
        };
        for output in row[output_key].as_array().into_iter().flatten() {
            let Some(name) = output["name"].as_str() else {
                continue;
            };
            let Some(value) = metric_number(&output["value"]) else {
                continue;
            };
            pairs
                .entry(name.to_owned())
                .or_insert_with(Vec::new)
                .push((value, target));
        }
    }
    pairs
}

fn metric_row_count(rows: &[serde_json::Value], output_key: &str) -> u64 {
    rows.iter()
        .filter(|row| row["targetAvailable"] == serde_json::Value::Bool(true))
        .filter(|row| {
            row[output_key]
                .as_array()
                .into_iter()
                .flatten()
                .any(|output| metric_number(&output["value"]).is_some())
        })
        .count() as u64
}

fn metric_correlation(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let left_mean = pairs.iter().map(|(left, _)| *left).sum::<f64>() / pairs.len() as f64;
    let right_mean = pairs.iter().map(|(_, right)| *right).sum::<f64>() / pairs.len() as f64;
    let mut numerator = 0.0;
    let mut left_sum = 0.0;
    let mut right_sum = 0.0;
    for (left, right) in pairs {
        let left_delta = *left - left_mean;
        let right_delta = *right - right_mean;
        numerator += left_delta * right_delta;
        left_sum += left_delta * left_delta;
        right_sum += right_delta * right_delta;
    }
    let denominator = (left_sum * right_sum).sqrt();
    (denominator > 0.0)
        .then_some(numerator / denominator)
        .filter(|value| value.is_finite())
}

fn metric_rank(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        values[*left]
            .partial_cmp(&values[*right])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.cmp(right))
    });
    let mut ranks = vec![0.0; values.len()];
    let mut index = 0;
    while index < order.len() {
        let mut end = index + 1;
        while end < order.len() && values[order[index]] == values[order[end]] {
            end += 1;
        }
        let rank = (index + end - 1) as f64 / 2.0 + 1.0;
        for position in &order[index..end] {
            ranks[*position] = rank;
        }
        index = end;
    }
    ranks
}

fn metric_rank_correlation(pairs: &[(f64, f64)]) -> Option<f64> {
    let left = pairs.iter().map(|(left, _)| *left).collect::<Vec<_>>();
    let right = pairs.iter().map(|(_, right)| *right).collect::<Vec<_>>();
    let left_rank = metric_rank(&left);
    let right_rank = metric_rank(&right);
    metric_correlation(&left_rank.into_iter().zip(right_rank).collect::<Vec<_>>())
}

fn feedback_rows(snapshot: &FeedbackSnapshot) -> &[serde_json::Value] {
    snapshot.input.evidence["market"]["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn paper_feedback_report_state(
    snapshot: &FeedbackSnapshot,
    lens: FeedbackLens,
    metrics: &serde_json::Value,
) -> EvidenceState {
    if matches!(
        snapshot.evidence_state,
        EvidenceState::Unknown
            | EvidenceState::Missing
            | EvidenceState::Incompatible
            | EvidenceState::Failed
    ) {
        return snapshot.evidence_state;
    }
    let lens_metrics = &metrics["lensMetrics"];
    let (available, samples) = match lens {
        FeedbackLens::Factor => (
            lens_metrics["factorOutputsAvailable"] == serde_json::Value::Bool(true),
            lens_metrics["realizedFactorRows"].as_u64().unwrap_or(0),
        ),
        FeedbackLens::Model => (
            lens_metrics["predictionQualityAvailable"] == serde_json::Value::Bool(true),
            lens_metrics["realizedPredictionRows"].as_u64().unwrap_or(0),
        ),
        FeedbackLens::Strategy => (
            lens_metrics["targetOutcomeAvailable"] == serde_json::Value::Bool(true),
            lens_metrics["targetOutcomeSamples"].as_u64().unwrap_or(0),
        ),
        FeedbackLens::Execution => (
            lens_metrics["acknowledgementAndFillEvidenceAvailable"]
                == serde_json::Value::Bool(true),
            lens_metrics["executionObservationCount"]
                .as_u64()
                .unwrap_or(0),
        ),
    };
    if !available {
        return if snapshot.input.realized_observations == 0 {
            EvidenceState::NotYetRealized
        } else {
            EvidenceState::Missing
        };
    }
    if samples == 0 {
        EvidenceState::NotYetRealized
    } else if samples < snapshot.input.required_observations {
        EvidenceState::InsufficientEvidence
    } else {
        EvidenceState::Ready
    }
}

fn paper_feedback_execution_metrics(paper: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(paper) = paper else {
        return serde_json::json!({
            "acknowledgementAndFillEvidenceAvailable": false,
            "availabilityReason": "paper-account-evidence-missing",
        });
    };
    let orders = paper["orders"].as_array().cloned().unwrap_or_default();
    let fills = paper["fills"].as_array().cloned().unwrap_or_default();
    let provider_evidence = paper["providerEvidence"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let execution_observation_count = orders.len().max(provider_evidence.len()).max(fills.len());
    let acknowledged = provider_evidence
        .iter()
        .filter(|evidence| evidence["providerOrderId"].as_str().is_some())
        .count();
    let filled_orders = orders
        .iter()
        .filter(|order| metric_number(&order["filledQuantity"]).is_some_and(|value| value > 0.0))
        .count();
    let total_fees = fills
        .iter()
        .filter_map(|fill| metric_number(&fill["fee"]))
        .sum::<f64>();
    let order_by_id = orders
        .iter()
        .filter_map(|order| order["orderId"].as_str().map(|id| (id, order)))
        .collect::<BTreeMap<_, _>>();
    let slippages = fills
        .iter()
        .filter_map(|fill| {
            let order = order_by_id.get(fill["orderId"].as_str()?)?;
            let limit = metric_number(&order["limitPrice"])?;
            let price = metric_number(&fill["price"])?;
            (limit > 0.0).then_some(((price - limit).abs() / limit) * 10_000.0)
        })
        .collect::<Vec<_>>();
    let evidence_available = paper.get("providerEvidence").is_some()
        && paper["scopeCompatible"] != serde_json::Value::Bool(false)
        && paper["restartRequired"] != serde_json::Value::Bool(true)
        && paper["reconciliation"] == serde_json::json!("reconciled");
    serde_json::json!({
        "acknowledgementAndFillEvidenceAvailable": evidence_available,
        "providerEvidenceCount": provider_evidence.len(),
        "executionObservationCount": execution_observation_count,
        "acknowledgedOrders": acknowledged,
        "acknowledgementRate": if orders.is_empty() { serde_json::Value::Null } else { serde_json::json!(acknowledged as f64 / orders.len() as f64) },
        "filledOrders": filled_orders,
        "fillRate": if orders.is_empty() { serde_json::Value::Null } else { serde_json::json!(filled_orders as f64 / orders.len() as f64) },
        "totalFees": total_fees,
        "averageSlippageBps": if slippages.is_empty() { serde_json::Value::Null } else { serde_json::json!(slippages.iter().sum::<f64>() / slippages.len() as f64) },
    })
}

fn paper_feedback_metrics(snapshot: &FeedbackSnapshot, lens: FeedbackLens) -> serde_json::Value {
    let paper = snapshot
        .input
        .evidence
        .get("paper")
        .filter(|value| value.is_object());
    let counts = serde_json::json!({
        "decisionBatches": snapshot.input.evidence["runtime"]["decisionBatchCount"],
        "orders": paper.and_then(|value| value["orderCount"].as_u64()).unwrap_or(0),
        "fills": paper.and_then(|value| value["fillCount"].as_u64()).unwrap_or(0),
        "riskDecisions": paper.and_then(|value| value["riskDecisionCount"].as_u64()).unwrap_or(0),
    });
    let rows = feedback_rows(snapshot);
    let factor_pairs = metric_pairs(rows, "factorOutputs");
    let model_pairs = metric_pairs(rows, "modelOutputs");
    let factor_samples = factor_pairs.values().map(Vec::len).sum::<usize>() as u64;
    let model_samples = model_pairs.values().map(Vec::len).sum::<usize>() as u64;
    let factor_rows = metric_row_count(rows, "factorOutputs");
    let model_rows = metric_row_count(rows, "modelOutputs");
    let candidate_rows = snapshot.input.evidence["market"]["candidateRowCount"]
        .as_u64()
        .unwrap_or(0);
    let factor_metrics = factor_pairs
        .iter()
        .map(|(name, pairs)| {
            (
                name.clone(),
                serde_json::json!({
                    "samples": pairs.len(),
                    "coverage": if candidate_rows == 0 { serde_json::Value::Null } else { serde_json::json!(pairs.len() as f64 / candidate_rows as f64) },
                    "ic": metric_correlation(pairs),
                    "rankIc": metric_rank_correlation(pairs),
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let model_metrics = model_pairs
        .iter()
        .map(|(name, pairs)| {
            let errors = pairs
                .iter()
                .map(|(prediction, target)| prediction - target)
                .collect::<Vec<_>>();
            let mae = (!errors.is_empty())
                .then(|| errors.iter().map(|error| error.abs()).sum::<f64>() / errors.len() as f64);
            let rmse = (!errors.is_empty()).then(|| {
                (errors.iter().map(|error| error * error).sum::<f64>() / errors.len() as f64).sqrt()
            });
            (
                name.clone(),
                serde_json::json!({
                    "samples": pairs.len(),
                    "mae": mae,
                    "rmse": rmse,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let target_outcomes = rows
        .iter()
        .filter(|row| row["targetOutcomeAvailable"] == serde_json::Value::Bool(true))
        .filter_map(|row| metric_number(&row["targetOutcomeReturn"]))
        .collect::<Vec<_>>();
    let target_mean = (!target_outcomes.is_empty())
        .then(|| target_outcomes.iter().sum::<f64>() / target_outcomes.len() as f64);
    let execution = paper_feedback_execution_metrics(paper);
    let lens_metrics = match lens {
        FeedbackLens::Factor => serde_json::json!({
            "realizedFactorSamples": factor_samples,
            "realizedFactorRows": factor_rows,
            "factorOutputsAvailable": factor_rows > 0,
            "outputMetrics": factor_metrics,
            "availabilityReason": if factor_samples == 0 { Some("no-compatible-factor-output-pairs") } else { None },
        }),
        FeedbackLens::Model => serde_json::json!({
            "realizedPredictionSamples": model_samples,
            "realizedPredictionRows": model_rows,
            "predictionQualityAvailable": model_rows > 0,
            "outputMetrics": model_metrics,
            "availabilityReason": if model_samples == 0 { Some("no-compatible-model-target-pairs") } else { None },
        }),
        FeedbackLens::Strategy => serde_json::json!({
            "targetOutcomeSamples": target_outcomes.len(),
            "targetOutcomeAvailable": !target_outcomes.is_empty(),
            "targetOutcomeMean": target_mean,
            "returnAndDrawdownAvailable": false,
            "returnAndDrawdownReason": "historical-account-valuation-series-not-retained",
            "riskAndExecutionCounts": counts,
        }),
        FeedbackLens::Execution => {
            let mut metrics = execution;
            if let Some(object) = metrics.as_object_mut() {
                object.insert("executionCounts".into(), counts.clone());
            }
            metrics
        }
    };
    let reasons = rows
        .iter()
        .filter_map(|row| row["reason"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    serde_json::json!({
        "lens": lens,
        "evidenceState": snapshot.evidence_state,
        "realizedObservations": snapshot.input.realized_observations,
        "requiredObservations": snapshot.input.required_observations,
        "observationStartMs": snapshot.input.observation_start_ms,
        "observationEndMs": snapshot.input.observation_end_ms,
        "directionalConclusion": false,
        "retainedCounts": counts,
        "lensMetrics": lens_metrics,
        "evidenceReasons": reasons,
        "note": "Host-retained evidence is the only source for feedback metrics; unavailable rows remain non-directional.",
    })
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    fn snapshot(evidence: serde_json::Value) -> FeedbackSnapshot {
        FeedbackSnapshot {
            snapshot_id: "snapshot".into(),
            input: FeedbackSnapshotInput {
                user_id: "user".into(),
                bundle_id: "bundle".into(),
                bot_id: "bot".into(),
                attempt_id: "attempt".into(),
                observation_start_ms: 1,
                observation_end_ms: 10,
                realization_cutoff_ms: 20,
                realized_observations: 2,
                required_observations: 2,
                evidence,
            },
            evidence_state: EvidenceState::Ready,
            created_at_ms: 30,
        }
    }

    #[test]
    fn factor_and_model_metrics_use_only_realized_pairs() {
        let snapshot = snapshot(serde_json::json!({
            "runtime": {"decisionBatchCount": 2},
            "market": {
                "candidateRowCount": 2,
                "rows": [
                    {"targetAvailable": true, "targetReturn": "1", "factorOutputs": [{"name": "score", "value": "1"}], "modelOutputs": [{"name": "forecast", "value": "1"}]},
                    {"targetAvailable": true, "targetReturn": "2", "factorOutputs": [{"name": "score", "value": "2"}], "modelOutputs": [{"name": "forecast", "value": "2"}]},
                    {"targetAvailable": false, "factorOutputs": [{"name": "score", "value": "100"}], "modelOutputs": [{"name": "forecast", "value": "100"}]}
                ]
            },
            "paper": {"orderCount": 0, "fillCount": 0, "riskDecisionCount": 0}
        }));
        let factor = paper_feedback_metrics(&snapshot, FeedbackLens::Factor);
        assert_eq!(factor["lensMetrics"]["realizedFactorSamples"], 2);
        assert_eq!(factor["lensMetrics"]["outputMetrics"]["score"]["ic"], 1.0);
        assert_eq!(
            factor["lensMetrics"]["outputMetrics"]["score"]["rankIc"],
            1.0
        );
        let model = paper_feedback_metrics(&snapshot, FeedbackLens::Model);
        assert_eq!(model["lensMetrics"]["realizedPredictionSamples"], 2);
        assert_eq!(
            model["lensMetrics"]["outputMetrics"]["forecast"]["mae"],
            0.0
        );
        assert_eq!(
            model["lensMetrics"]["outputMetrics"]["forecast"]["rmse"],
            0.0
        );
    }

    #[test]
    fn report_state_is_lens_specific_and_keeps_missing_outputs_explicit() {
        let snapshot = snapshot(serde_json::json!({
            "runtime": {"decisionBatchCount": 2},
            "market": {"candidateRowCount": 2, "rows": [{"targetAvailable": true, "targetReturn": "1", "targetOutcomeAvailable": true, "targetOutcomeReturn": "1", "factorOutputs": [], "modelOutputs": []}]},
            "paper": {"orderCount": 0, "fillCount": 0, "riskDecisionCount": 0, "orders": [], "fills": []}
        }));
        let factor = paper_feedback_metrics(&snapshot, FeedbackLens::Factor);
        assert_eq!(
            paper_feedback_report_state(&snapshot, FeedbackLens::Factor, &factor),
            EvidenceState::Missing
        );
        let strategy = paper_feedback_metrics(&snapshot, FeedbackLens::Strategy);
        assert_eq!(
            paper_feedback_report_state(&snapshot, FeedbackLens::Strategy, &strategy),
            EvidenceState::InsufficientEvidence
        );
    }

    #[test]
    fn execution_metrics_use_retained_provider_and_fill_evidence() {
        let paper = serde_json::json!({
            "orders": [
                {"orderId": "local-1", "filledQuantity": "2", "limitPrice": "100"},
                {"orderId": "local-2", "filledQuantity": "0", "limitPrice": "100"}
            ],
            "fills": [
                {"orderId": "local-1", "quantity": "2", "price": "101", "fee": "0.5"}
            ],
            "providerEvidence": [
                {"outcome": "accepted", "providerOrderId": "remote-1", "localOrderId": "local-1"}
            ]
        });
        let metrics = paper_feedback_execution_metrics(Some(&paper));
        assert_eq!(metrics["providerEvidenceCount"], 1);
        assert_eq!(metrics["acknowledgedOrders"], 1);
        assert_eq!(metrics["filledOrders"], 1);
        assert_eq!(metrics["totalFees"], 0.5);
        assert_eq!(metrics["averageSlippageBps"], 100.0);
    }

    #[test]
    fn report_state_counts_unique_realized_rows_per_lens() {
        let snapshot = snapshot(serde_json::json!({
            "runtime": {"decisionBatchCount": 1},
            "market": {
                "candidateRowCount": 1,
                "rows": [{
                    "targetAvailable": true,
                    "targetReturn": "1",
                    "factorOutputs": [
                        {"name": "score", "value": "1"},
                        {"name": "confidence", "value": "2"}
                    ],
                    "modelOutputs": []
                }]
            },
            "paper": {
                "orderCount": 1,
                "fillCount": 0,
                "riskDecisionCount": 1,
                "reconciliation": "reconciled",
                "restartRequired": false,
                "scopeCompatible": true,
                "orders": [{"orderId": "local-1", "filledQuantity": "0", "limitPrice": "100"}],
                "fills": [],
                "providerEvidence": []
            }
        }));
        let factor = paper_feedback_metrics(&snapshot, FeedbackLens::Factor);
        assert_eq!(factor["lensMetrics"]["realizedFactorSamples"], 2);
        assert_eq!(factor["lensMetrics"]["realizedFactorRows"], 1);
        assert_eq!(
            paper_feedback_report_state(&snapshot, FeedbackLens::Factor, &factor),
            EvidenceState::InsufficientEvidence
        );
        let execution = paper_feedback_metrics(&snapshot, FeedbackLens::Execution);
        assert_eq!(execution["lensMetrics"]["executionObservationCount"], 1);
        assert_eq!(
            paper_feedback_report_state(&snapshot, FeedbackLens::Execution, &execution),
            EvidenceState::InsufficientEvidence
        );
    }
}
