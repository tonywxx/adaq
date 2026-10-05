//! Immutable Paper evidence and human-reviewed research feedback.
//!
//! This boundary intentionally stores references and evidence state, not mutable
//! projections from the running Bot. Reports and review decisions are append-only.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
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

    pub(crate) fn create_report_with_state(
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
    fn every_lens_inherits_the_snapshot_evidence_state() {
        let s = store();
        let snapshot = s
            .create_snapshot_with_state(
                snapshot_input(),
                1,
                Some(EvidenceState::InsufficientEvidence),
            )
            .unwrap();
        for lens in [
            FeedbackLens::Factor,
            FeedbackLens::Model,
            FeedbackLens::Strategy,
            FeedbackLens::Execution,
        ] {
            let report = s
                .create_report(
                    FeedbackReportInput {
                        user_id: "user".into(),
                        snapshot_id: snapshot.snapshot_id.clone(),
                        lens,
                        metrics: serde_json::json!({"directionalConclusion": false}),
                        comparable_evidence_id: None,
                    },
                    2,
                )
                .unwrap();
            assert_eq!(report.evidence_state, EvidenceState::InsufficientEvidence);
        }
    }

    #[test]
    fn report_state_can_be_overridden_per_lens_without_mutating_snapshot() {
        let s = store();
        let snapshot = s
            .create_snapshot_with_state(snapshot_input(), 1, Some(EvidenceState::Ready))
            .unwrap();
        let report = s
            .create_report_with_state(
                FeedbackReportInput {
                    user_id: "user".into(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    lens: FeedbackLens::Factor,
                    metrics: serde_json::json!({"lensMetrics": {"factorOutputsAvailable": false}}),
                    comparable_evidence_id: None,
                },
                2,
                EvidenceState::Missing,
            )
            .unwrap();
        assert_eq!(report.evidence_state, EvidenceState::Missing);
        assert_eq!(
            s.snapshot_for_user("user", &snapshot.snapshot_id)
                .unwrap()
                .evidence_state,
            EvidenceState::Ready
        );
    }
}
