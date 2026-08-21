//! Immutable Paper evidence and human-reviewed research feedback.
//!
//! This boundary intentionally stores references and evidence state, not mutable
//! projections from the running Bot. Reports and review decisions are append-only.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSnapshotInput {
    pub user_id: String,
    pub bundle_id: String,
    pub bot_id: String,
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

#[derive(Clone)]
pub struct PaperFeedbackStore {
    database: Arc<Mutex<Connection>>,
}

impl PaperFeedbackStore {
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
        ").map_err(|e| e.to_string())?;
        Ok(Self { database })
    }

    pub fn create_snapshot(
        &self,
        input: FeedbackSnapshotInput,
        created_at_ms: i64,
    ) -> Result<FeedbackSnapshot, String> {
        validate_user_and_range(
            &input.user_id,
            input.observation_start_ms,
            input.observation_end_ms,
        )?;
        if input.bundle_id.trim().is_empty()
            || input.bot_id.trim().is_empty()
            || input.realization_cutoff_ms < input.observation_start_ms
        {
            return Err("invalid Paper Feedback Snapshot binding".into());
        }
        let state = if input.realized_observations == 0 {
            EvidenceState::NotYetRealized
        } else if input.realized_observations < input.required_observations {
            EvidenceState::InsufficientEvidence
        } else {
            EvidenceState::Ready
        };
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

    pub fn create_report(
        &self,
        input: FeedbackReportInput,
        created_at_ms: i64,
    ) -> Result<FeedbackReport, String> {
        validate_user(&input.user_id)?;
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let snapshot_state: Option<String> = conn.query_row(
            "SELECT evidence_state FROM paper_feedback_snapshots WHERE snapshot_id=?1 AND user_id=?2",
            params![input.snapshot_id, input.user_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        let snapshot_state = snapshot_state
            .ok_or_else(|| "Paper Feedback Snapshot was not found for User".to_string())?;
        let state: EvidenceState =
            serde_json::from_str(&format!("\"{snapshot_state}\"")).map_err(|e| e.to_string())?;
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

    pub fn record_review_decision(
        &self,
        input: ReviewDecisionInput,
    ) -> Result<ReviewDecision, String> {
        validate_user(&input.user_id)?;
        if input.report_ids.is_empty() || input.rationale.trim().is_empty() {
            return Err("a review decision requires reports and rationale".into());
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
            observation_start_ms: 10,
            observation_end_ms: 20,
            realization_cutoff_ms: 20,
            realized_observations: 2,
            required_observations: 3,
            evidence: serde_json::json!({"orders": 2}),
        }
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
}
