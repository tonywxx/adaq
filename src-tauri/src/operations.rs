//! Host-owned operational evidence, health, alerts, and fail-closed actions.
//!
//! This module is deliberately independent of the GUI. Runtimes submit typed
//! observations; only this host boundary persists evidence and derives safety.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const MAX_METRICS: usize = 32;
const MAX_METRIC_KEY_BYTES: usize = 64;
const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_EVENT_HISTORY: usize = 256;
const OPERATIONS_POLICY_ID: &str = "adaq:operations-policy@1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum HealthDimension {
    MarketData,
    Worker,
    FeatureModelStrategy,
    PaperAccount,
    RiskOms,
    ExecutionAdapter,
    LocalSystem,
    ResearchFeedback,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum HealthState {
    Healthy,
    Degraded,
    Critical,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AlertState {
    Active,
    Acknowledged,
    Resolved,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SafetyAction {
    None,
    SkipDecision,
    Pause,
    FaultAndReconcile,
    FreezeAll,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthObservation {
    pub user_id: String,
    pub entity_id: String,
    pub dimension: HealthDimension,
    pub state: HealthState,
    pub condition: String,
    pub evidence: Value,
    pub required: bool,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub event_kind: Option<String>,
    #[serde(default)]
    pub evidence_id: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub causation_id: Option<String>,
    #[serde(default)]
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub metrics: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalEvent {
    pub event_id: String,
    pub user_id: String,
    pub entity_id: String,
    pub dimension: HealthDimension,
    pub kind: String,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    pub evidence_id: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub diagnostic: Option<String>,
    pub metrics: BTreeMap<String, f64>,
    pub evidence: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    pub entity_id: String,
    pub dimension: HealthDimension,
    pub state: HealthState,
    pub required: bool,
    pub observed_at_ms: i64,
    pub event_id: String,
    pub condition: String,
    pub evidence_id: Option<String>,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertView {
    pub alert_id: String,
    pub user_id: String,
    pub entity_id: String,
    pub dimension: HealthDimension,
    pub condition: String,
    pub policy_id: String,
    pub severity: AlertSeverity,
    pub state: AlertState,
    pub safety_action: SafetyAction,
    pub first_event_id: String,
    pub first_critical_event_id: Option<String>,
    pub first_observed_at_ms: i64,
    pub occurrence_count: i64,
    pub last_observed_at_ms: i64,
    pub last_event_id: String,
    pub evidence_id: Option<String>,
    pub correlation_id: Option<String>,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertLifecycleView {
    pub lifecycle_id: String,
    pub alert_id: String,
    pub user_id: String,
    pub state: AlertState,
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub actor: String,
}

#[derive(Clone)]
pub struct OperationsStore {
    database: Arc<Mutex<Connection>>,
}

impl OperationsStore {
    pub fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let store = Self { database };
        {
            let connection = store.database.lock().map_err(|e| e.to_string())?;
            connection
                .execute_batch(
                    "PRAGMA foreign_keys = ON;
                     CREATE TABLE IF NOT EXISTS operational_events (
                        event_id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        entity_id TEXT NOT NULL,
                        dimension TEXT NOT NULL,
                        kind TEXT NOT NULL,
                        observed_at_ms INTEGER NOT NULL,
                        evidence_json TEXT NOT NULL,
                        evidence_id TEXT,
                        correlation_id TEXT,
                        causation_id TEXT,
                        diagnostic TEXT,
                        metrics_json TEXT NOT NULL DEFAULT '{}',
                        recorded_at_ms INTEGER NOT NULL DEFAULT 0
                     );
                     CREATE INDEX IF NOT EXISTS operational_events_user_time
                        ON operational_events(user_id, observed_at_ms DESC);
                     CREATE TABLE IF NOT EXISTS operational_alerts (
                        alert_id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        entity_id TEXT NOT NULL,
                        dimension TEXT NOT NULL,
                        condition TEXT NOT NULL,
                        policy_id TEXT NOT NULL DEFAULT 'adaq:operations-policy@1',
                        severity TEXT NOT NULL,
                        state TEXT NOT NULL,
                        safety_action TEXT NOT NULL,
                        first_event_id TEXT NOT NULL DEFAULT '',
                        first_critical_event_id TEXT,
                        first_observed_at_ms INTEGER NOT NULL DEFAULT 0,
                        occurrence_count INTEGER NOT NULL DEFAULT 1,
                        last_observed_at_ms INTEGER NOT NULL DEFAULT 0,
                        last_event_id TEXT NOT NULL,
                        evidence_id TEXT,
                        correlation_id TEXT,
                        diagnostic TEXT,
                        UNIQUE(user_id, entity_id, dimension, condition, policy_id)
                     );
                     CREATE TABLE IF NOT EXISTS operational_alert_lifecycle (
                        lifecycle_id TEXT PRIMARY KEY,
                        alert_id TEXT NOT NULL,
                        user_id TEXT NOT NULL,
                        state TEXT NOT NULL,
                        event_id TEXT NOT NULL,
                        occurred_at_ms INTEGER NOT NULL,
                        actor TEXT NOT NULL DEFAULT 'host',
                        FOREIGN KEY(alert_id) REFERENCES operational_alerts(alert_id)
                     );
                     CREATE TABLE IF NOT EXISTS operational_safety_actions (
                        action_id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        alert_id TEXT NOT NULL,
                        event_id TEXT NOT NULL,
                        action TEXT NOT NULL,
                        status TEXT NOT NULL,
                        occurred_at_ms INTEGER NOT NULL,
                        detail TEXT NOT NULL,
                        UNIQUE(user_id, event_id, action),
                        FOREIGN KEY(alert_id) REFERENCES operational_alerts(alert_id)
                     );",
                )
                .map_err(|e| e.to_string())?;
            for (column, definition) in [
                ("evidence_id", "TEXT"),
                ("correlation_id", "TEXT"),
                ("causation_id", "TEXT"),
                ("diagnostic", "TEXT"),
                ("metrics_json", "TEXT NOT NULL DEFAULT '{}'"),
                ("recorded_at_ms", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                ensure_column(&connection, "operational_events", column, definition)?;
            }
            for (column, definition) in [
                (
                    "policy_id",
                    "TEXT NOT NULL DEFAULT 'adaq:operations-policy@1'",
                ),
                ("first_event_id", "TEXT NOT NULL DEFAULT ''"),
                ("first_critical_event_id", "TEXT"),
                ("first_observed_at_ms", "INTEGER NOT NULL DEFAULT 0"),
                ("occurrence_count", "INTEGER NOT NULL DEFAULT 1"),
                ("last_observed_at_ms", "INTEGER NOT NULL DEFAULT 0"),
                ("evidence_id", "TEXT"),
                ("correlation_id", "TEXT"),
                ("diagnostic", "TEXT"),
            ] {
                ensure_column(&connection, "operational_alerts", column, definition)?;
            }
            ensure_column(
                &connection,
                "operational_alert_lifecycle",
                "actor",
                "TEXT NOT NULL DEFAULT 'host'",
            )?;
            connection
                .execute(
                    "UPDATE operational_alerts
                     SET first_event_id = last_event_id
                     WHERE first_event_id = ''",
                    [],
                )
                .map_err(|e| e.to_string())?;
            connection
                .execute(
                    "UPDATE operational_alerts
                     SET first_critical_event_id = first_event_id
                     WHERE first_critical_event_id IS NULL AND severity='critical'",
                    [],
                )
                .map_err(|e| e.to_string())?;
        }
        store.rebuild_missing_alert_projection()?;
        Ok(store)
    }

    pub fn observe(
        &self,
        observation: HealthObservation,
    ) -> Result<(OperationalEvent, Option<AlertView>, SafetyAction), String> {
        validate_observation(&observation)?;
        let dimension = enum_name(&observation.dimension)?;
        let mut evidence = redact_and_bound(observation.evidence.clone())?;
        validate_evidence_identity(&observation, &evidence)?;
        let metrics = normalize_metrics(&observation.metrics, &evidence)?;
        let evidence_id = observation.evidence_id.clone().or_else(|| {
            extract_identity(
                &evidence,
                &[
                    "evidenceId",
                    "observationId",
                    "reportId",
                    "attemptId",
                    "operationId",
                    "snapshotId",
                    "datasetId",
                    "botId",
                    "profileId",
                ],
            )
        });
        let correlation_id = observation
            .correlation_id
            .clone()
            .or_else(|| extract_identity(&evidence, &["correlationId", "requestId"]));
        let causation_id = observation
            .causation_id
            .clone()
            .or_else(|| extract_identity(&evidence, &["causationId", "eventId"]));
        let diagnostic = observation
            .diagnostic
            .clone()
            .or_else(|| {
                evidence
                    .get("detail")
                    .or_else(|| evidence.get("diagnostic"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .map(|value| bounded_redacted(&value));
        let kind = observation
            .event_kind
            .clone()
            .unwrap_or_else(|| "health.observed".into());
        validate_kind(&kind)?;
        insert_health_metadata(
            &mut evidence,
            observation.state,
            observation.required,
            &observation.condition,
        )?;
        let evidence_json = serde_json::to_string(&evidence).map_err(|e| e.to_string())?;
        if evidence_json.len() > MAX_EVENT_BYTES {
            return Err("operational evidence exceeds the Host retention bound".into());
        }
        let event = OperationalEvent {
            event_id: Uuid::new_v4().to_string(),
            user_id: observation.user_id.clone(),
            entity_id: observation.entity_id.clone(),
            dimension: observation.dimension,
            kind,
            observed_at_ms: observation.observed_at_ms,
            recorded_at_ms: now_ms(),
            evidence_id: evidence_id.map(|value| bounded_redacted(&value)),
            correlation_id: correlation_id.map(|value| bounded_redacted(&value)),
            causation_id: causation_id.map(|value| bounded_redacted(&value)),
            diagnostic,
            metrics,
            evidence,
        };
        let mut connection = self.database.lock().map_err(|e| e.to_string())?;
        let latest: Option<i64> = connection
            .query_row(
                "SELECT MAX(observed_at_ms) FROM operational_events
                 WHERE user_id=?1 AND entity_id=?2 AND dimension=?3",
                params![event.user_id, event.entity_id, dimension],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if latest.is_some_and(|latest| event.observed_at_ms < latest) {
            return Err("stale operational observation was rejected".into());
        }
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO operational_events
                 (event_id,user_id,entity_id,dimension,kind,observed_at_ms,evidence_json,
                  evidence_id,correlation_id,causation_id,diagnostic,metrics_json,recorded_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    event.event_id,
                    event.user_id,
                    event.entity_id,
                    dimension,
                    event.kind,
                    event.observed_at_ms,
                    evidence_json,
                    event.evidence_id,
                    event.correlation_id,
                    event.causation_id,
                    event.diagnostic,
                    serde_json::to_string(&event.metrics).map_err(|e| e.to_string())?,
                    event.recorded_at_ms,
                ],
            )
            .map_err(|e| e.to_string())?;
        let projection = project_event_locked(&transaction, &event)?;
        transaction.commit().map_err(|e| e.to_string())?;
        drop(connection);
        #[cfg(not(test))]
        if projection.notify
            && let Some(alert) = &projection.alert
        {
            notify_native(alert);
        }
        Ok((event, projection.alert, projection.action))
    }

    pub fn health_for_user(&self, user_id: &str) -> Result<Vec<HealthView>, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        let rows = load_event_rows(&connection, Some(user_id), true)?;
        let mut latest_by_condition = BTreeMap::new();
        for row in rows {
            let event = row.event()?;
            let (state, required, condition) = health_metadata(&event);
            latest_by_condition
                .entry((
                    event.entity_id.clone(),
                    enum_name(&event.dimension)?,
                    condition,
                ))
                .or_insert((event, state, required));
        }
        let mut health_by_dimension = BTreeMap::new();
        for (_, (event, state, required)) in latest_by_condition {
            let (_, _, condition) = health_metadata(&event);
            let view = HealthView {
                entity_id: event.entity_id.clone(),
                dimension: event.dimension,
                state,
                required,
                observed_at_ms: event.observed_at_ms,
                event_id: event.event_id,
                condition,
                evidence_id: event.evidence_id,
                diagnostic: event.diagnostic,
            };
            let key = (view.entity_id.clone(), enum_name(&view.dimension)?);
            let replace = health_by_dimension
                .get(&key)
                .is_none_or(|current: &HealthView| {
                    health_priority(view.state, view.required)
                        .cmp(&health_priority(current.state, current.required))
                        .then_with(|| view.observed_at_ms.cmp(&current.observed_at_ms))
                        .then_with(|| view.event_id.cmp(&current.event_id))
                        .is_gt()
                });
            if replace {
                health_by_dimension.insert(key, view);
            }
        }
        Ok(health_by_dimension.into_values().collect())
    }

    pub fn events_for_user(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<OperationalEvent>, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        load_event_rows(&connection, Some(user_id), true)?
            .into_iter()
            .take(limit.clamp(1, MAX_EVENT_HISTORY))
            .map(EventRow::event)
            .collect()
    }

    pub fn transition_alert(
        &self,
        user_id: &str,
        alert_id: &str,
        state: AlertState,
        event_id: &str,
        occurred_at_ms: i64,
    ) -> Result<(), String> {
        if state != AlertState::Acknowledged {
            return Err("Alerts resolve only from validated Host recovery evidence".into());
        }
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        let alert = load_alert_by_id(&connection, user_id, alert_id)?
            .ok_or_else(|| "operational alert was not found for User".to_string())?;
        if alert.state == AlertState::Acknowledged {
            return Ok(());
        }
        if alert.state != AlertState::Active || alert.last_event_id != event_id {
            return Err(
                "operational acknowledgement must reference the current active evidence".into(),
            );
        }
        let valid_event: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM operational_events
                    WHERE event_id=?1 AND user_id=?2 AND entity_id=?3 AND dimension=?4
                )",
                params![
                    event_id,
                    user_id,
                    alert.entity_id,
                    enum_name(&alert.dimension)?
                ],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !valid_event {
            return Err("operational transition must reference matching User evidence".into());
        }
        if occurred_at_ms <= 0 {
            return Err("operational acknowledgement timestamp is invalid".into());
        }
        connection
            .execute(
                "UPDATE operational_alerts SET state=?1 WHERE alert_id=?2 AND user_id=?3",
                params![enum_name(&AlertState::Acknowledged)?, alert_id, user_id],
            )
            .map_err(|e| e.to_string())?;
        append_lifecycle(
            &connection,
            alert_id,
            user_id,
            AlertState::Acknowledged,
            event_id,
            occurred_at_ms,
            "user",
        )
    }

    pub fn acknowledge(
        &self,
        user_id: &str,
        alert_id: &str,
        occurred_at_ms: i64,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        let alert = load_alert_by_id(&connection, user_id, alert_id)?
            .ok_or_else(|| "operational alert was not found for User".to_string())?;
        drop(connection);
        self.transition_alert(
            user_id,
            alert_id,
            AlertState::Acknowledged,
            &alert.last_event_id,
            occurred_at_ms,
        )
    }

    pub fn alerts_for_user(&self, user_id: &str) -> Result<Vec<AlertView>, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        let mut alerts = load_alerts(&connection, user_id)?
            .into_iter()
            .map(AlertRecord::view)
            .collect::<Vec<_>>();
        alerts.sort_by(|left, right| {
            right
                .last_observed_at_ms
                .cmp(&left.last_observed_at_ms)
                .then_with(|| left.alert_id.cmp(&right.alert_id))
        });
        Ok(alerts)
    }

    pub fn is_user_frozen(&self, user_id: &str) -> Result<bool, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM operational_alerts
                    WHERE user_id=?1 AND safety_action='freezeAll' AND state <> 'resolved'
                )",
                [user_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub fn blocks_new_risk(&self, user_id: &str) -> Result<bool, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM operational_alerts
                    WHERE user_id=?1 AND state <> 'resolved' AND safety_action <> 'none'
                )",
                [user_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub fn alert_history_for_user(
        &self,
        user_id: &str,
        alert_id: &str,
    ) -> Result<Vec<AlertLifecycleView>, String> {
        validate_user(user_id)?;
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        if load_alert_by_id(&connection, user_id, alert_id)?.is_none() {
            return Err("operational alert was not found for User".into());
        }
        let mut statement = connection
            .prepare(
                "SELECT lifecycle_id,alert_id,user_id,state,event_id,occurred_at_ms,actor
                 FROM operational_alert_lifecycle
                 WHERE alert_id=?1 AND user_id=?2
                 ORDER BY occurred_at_ms,lifecycle_id",
            )
            .map_err(|e| e.to_string())?;
        statement
            .query_map(params![alert_id, user_id], |row| {
                Ok(AlertLifecycleView {
                    lifecycle_id: row.get(0)?,
                    alert_id: row.get(1)?,
                    user_id: row.get(2)?,
                    state: parse(row.get(3)?)?,
                    event_id: row.get(4)?,
                    occurred_at_ms: row.get(5)?,
                    actor: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn complete_safety_action(
        &self,
        user_id: &str,
        event_id: &str,
        status: &str,
        detail: &str,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        if !bounded_nonempty(status, 32) {
            return Err("invalid safety action status".into());
        }
        let detail = bounded_redacted(detail);
        let changed = self
            .database
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE operational_safety_actions
                 SET status=?1, detail=?2
                 WHERE user_id=?3 AND event_id=?4",
                params![status, detail, user_id, event_id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("operational safety action was not found for User".into());
        }
        Ok(())
    }

    fn rebuild_missing_alert_projection(&self) -> Result<(), String> {
        let connection = self.database.lock().map_err(|e| e.to_string())?;
        let events = load_event_rows(&connection, None, false)?
            .into_iter()
            .map(EventRow::event)
            .collect::<Result<Vec<_>, _>>()?;
        for event in &events {
            let (state, _, condition) = health_metadata(event);
            if state == HealthState::Healthy
                || load_alert_for_key(
                    &connection,
                    &event.user_id,
                    &event.entity_id,
                    event.dimension,
                    &condition,
                )?
                .is_some()
            {
                continue;
            }
            // Replay only an absent deduplication key. Existing projections
            // must not have their occurrence counts doubled on restart.
            for candidate in events.iter().filter(|candidate| {
                let (_, _, candidate_condition) = health_metadata(candidate);
                candidate.user_id == event.user_id
                    && candidate.entity_id == event.entity_id
                    && candidate.dimension == event.dimension
                    && candidate_condition == condition
            }) {
                project_event_locked(&connection, candidate)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct EventRow {
    event_id: String,
    user_id: String,
    entity_id: String,
    dimension: String,
    kind: String,
    observed_at_ms: i64,
    recorded_at_ms: i64,
    evidence_json: String,
    evidence_id: Option<String>,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    diagnostic: Option<String>,
    metrics_json: String,
}

impl EventRow {
    fn event(self) -> Result<OperationalEvent, String> {
        Ok(OperationalEvent {
            event_id: self.event_id,
            user_id: self.user_id,
            entity_id: self.entity_id,
            dimension: parse_json(&self.dimension)?,
            kind: self.kind,
            observed_at_ms: self.observed_at_ms,
            recorded_at_ms: self.recorded_at_ms,
            evidence_id: self.evidence_id,
            correlation_id: self.correlation_id,
            causation_id: self.causation_id,
            diagnostic: self.diagnostic,
            metrics: serde_json::from_str(&self.metrics_json).unwrap_or_default(),
            evidence: serde_json::from_str(&self.evidence_json).unwrap_or(Value::Null),
        })
    }
}

#[derive(Clone)]
struct AlertRecord {
    alert_id: String,
    user_id: String,
    entity_id: String,
    dimension: HealthDimension,
    condition: String,
    policy_id: String,
    severity: AlertSeverity,
    state: AlertState,
    safety_action: SafetyAction,
    first_event_id: String,
    first_critical_event_id: Option<String>,
    first_observed_at_ms: i64,
    occurrence_count: i64,
    last_observed_at_ms: i64,
    last_event_id: String,
    evidence_id: Option<String>,
    correlation_id: Option<String>,
    diagnostic: Option<String>,
}

impl AlertRecord {
    fn view(self) -> AlertView {
        AlertView {
            alert_id: self.alert_id,
            user_id: self.user_id,
            entity_id: self.entity_id,
            dimension: self.dimension,
            condition: self.condition,
            policy_id: self.policy_id,
            severity: self.severity,
            state: self.state,
            safety_action: self.safety_action,
            first_event_id: self.first_event_id,
            first_critical_event_id: self.first_critical_event_id,
            first_observed_at_ms: self.first_observed_at_ms,
            occurrence_count: self.occurrence_count,
            last_observed_at_ms: self.last_observed_at_ms,
            last_event_id: self.last_event_id,
            evidence_id: self.evidence_id,
            correlation_id: self.correlation_id,
            diagnostic: self.diagnostic,
        }
    }
}

struct ProjectionResult {
    alert: Option<AlertView>,
    action: SafetyAction,
    notify: bool,
}

fn project_event_locked(
    connection: &Connection,
    event: &OperationalEvent,
) -> Result<ProjectionResult, String> {
    let (state, required, condition) = health_metadata(event);
    let action = safety_action_for_event(event.dimension, state, required, &condition);
    let severity = severity_for(state, required);
    let existing = load_alert_for_key(
        connection,
        &event.user_id,
        &event.entity_id,
        event.dimension,
        &condition,
    )?;
    let should_alert = !matches!(state, HealthState::Healthy);
    if should_alert {
        let alert_id = existing
            .as_ref()
            .map(|alert| alert.alert_id.clone())
            .unwrap_or_else(|| {
                stable_alert_id(
                    &event.user_id,
                    &event.entity_id,
                    event.dimension,
                    &condition,
                )
            });
        let (notify, projected_action) = match existing.as_ref() {
            None => {
                connection
                    .execute(
                        "INSERT INTO operational_alerts
                         (alert_id,user_id,entity_id,dimension,condition,policy_id,severity,state,
                          safety_action,first_event_id,first_critical_event_id,first_observed_at_ms,
                          occurrence_count,last_observed_at_ms,last_event_id,evidence_id,
                          correlation_id,diagnostic)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?12,?13,?14,?15,?16)",
                        params![
                            alert_id,
                            event.user_id,
                            event.entity_id,
                            enum_name(&event.dimension)?,
                            condition,
                            OPERATIONS_POLICY_ID,
                            enum_name(&severity)?,
                            enum_name(&AlertState::Active)?,
                            enum_name(&action)?,
                            event.event_id,
                            (severity == AlertSeverity::Critical).then(|| event.event_id.clone()),
                            event.observed_at_ms,
                            event.event_id,
                            event.evidence_id,
                            event.correlation_id,
                            event.diagnostic,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                append_lifecycle(
                    connection,
                    &alert_id,
                    &event.user_id,
                    AlertState::Active,
                    &event.event_id,
                    event.observed_at_ms,
                    "host",
                )?;
                (severity == AlertSeverity::Critical, action)
            }
            Some(alert) => {
                let next_state = if alert.state == AlertState::Resolved {
                    AlertState::Active
                } else {
                    alert.state
                };
                // Keep a fail-closed action active until the condition has a
                // validated Healthy recovery. A lower-severity follow-up
                // observation must not release a previously applied Freeze or
                // Fault action.
                let next_action =
                    if action == SafetyAction::None && alert.state != AlertState::Resolved {
                        alert.safety_action
                    } else {
                        action
                    };
                let next_severity = higher_severity(alert.severity, severity);
                let first_event_id = if alert.first_event_id.is_empty() {
                    event.event_id.clone()
                } else {
                    alert.first_event_id.clone()
                };
                let first_critical_event_id = alert.first_critical_event_id.clone().or_else(|| {
                    (severity == AlertSeverity::Critical).then(|| event.event_id.clone())
                });
                let first_observed_at_ms = if alert.first_observed_at_ms == 0 {
                    event.observed_at_ms
                } else {
                    alert.first_observed_at_ms
                };
                let occurrence_count = alert.occurrence_count.saturating_add(1);
                connection
                    .execute(
                        "UPDATE operational_alerts
                         SET severity=?1,state=?2,safety_action=?3,first_event_id=?4,
                             first_critical_event_id=?5,first_observed_at_ms=?6,
                             occurrence_count=?7,last_observed_at_ms=?8,last_event_id=?9,
                             evidence_id=?10,correlation_id=?11,diagnostic=?12
                         WHERE alert_id=?13 AND user_id=?14",
                        params![
                            enum_name(&next_severity)?,
                            enum_name(&next_state)?,
                            enum_name(&next_action)?,
                            first_event_id,
                            first_critical_event_id,
                            first_observed_at_ms,
                            occurrence_count,
                            event.observed_at_ms,
                            event.event_id,
                            event.evidence_id,
                            event.correlation_id,
                            event.diagnostic,
                            alert_id,
                            event.user_id,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                if alert.state == AlertState::Resolved {
                    append_lifecycle(
                        connection,
                        &alert_id,
                        &event.user_id,
                        AlertState::Active,
                        &event.event_id,
                        event.observed_at_ms,
                        "host",
                    )?;
                }
                (
                    // One native notification per active incident is the
                    // frozen cooldown; recurrence after Healthy recovery
                    // opens a new notification window.
                    alert.state == AlertState::Resolved && next_severity == AlertSeverity::Critical,
                    next_action,
                )
            }
        };
        if projected_action != SafetyAction::None {
            let action_id = format!("safety-{}", event.event_id);
            connection
                .execute(
                    "INSERT OR IGNORE INTO operational_safety_actions
                     (action_id,user_id,alert_id,event_id,action,status,occurred_at_ms,detail)
                     VALUES (?1,?2,?3,?4,?5,'required',?6,?7)",
                    params![
                        action_id,
                        event.user_id,
                        alert_id,
                        event.event_id,
                        enum_name(&projected_action)?,
                        event.observed_at_ms,
                        event.diagnostic.as_deref().unwrap_or(condition.as_str()),
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        let alert = load_alert_by_id(connection, &event.user_id, &alert_id)?
            .ok_or_else(|| "operational alert projection was not retained".to_string())?
            .view();
        Ok(ProjectionResult {
            notify,
            action: projected_action,
            alert: Some(alert),
        })
    } else {
        if let Some(alert) = existing {
            if alert.state != AlertState::Resolved {
                connection
                    .execute(
                        "UPDATE operational_alerts
                         SET state=?1,last_observed_at_ms=?2,last_event_id=?3,evidence_id=?4,
                             correlation_id=?5,diagnostic=?6
                         WHERE alert_id=?7 AND user_id=?8",
                        params![
                            enum_name(&AlertState::Resolved)?,
                            event.observed_at_ms,
                            event.event_id,
                            event.evidence_id,
                            event.correlation_id,
                            event.diagnostic,
                            alert.alert_id,
                            event.user_id,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                append_lifecycle(
                    connection,
                    &alert.alert_id,
                    &event.user_id,
                    AlertState::Resolved,
                    &event.event_id,
                    event.observed_at_ms,
                    "host",
                )?;
                connection
                    .execute(
                        "UPDATE operational_safety_actions
                         SET status='resolved'
                         WHERE user_id=?1 AND alert_id=?2 AND status='required'",
                        params![event.user_id, alert.alert_id],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(ProjectionResult {
            alert: None,
            action,
            notify: false,
        })
    }
}

fn load_event_rows(
    connection: &Connection,
    user_id: Option<&str>,
    descending: bool,
) -> Result<Vec<EventRow>, String> {
    let order = if descending { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT event_id,user_id,entity_id,dimension,kind,observed_at_ms,evidence_json,
                evidence_id,correlation_id,causation_id,diagnostic,metrics_json,recorded_at_ms
         FROM operational_events {}
         ORDER BY observed_at_ms {},recorded_at_ms {},event_id {}",
        if user_id.is_some() {
            "WHERE user_id=?1"
        } else {
            ""
        },
        order,
        order,
        order
    );
    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let map = |row: &Row<'_>| {
        Ok(EventRow {
            event_id: row.get(0)?,
            user_id: row.get(1)?,
            entity_id: row.get(2)?,
            dimension: row.get(3)?,
            kind: row.get(4)?,
            observed_at_ms: row.get(5)?,
            evidence_json: row.get(6)?,
            evidence_id: row.get(7)?,
            correlation_id: row.get(8)?,
            causation_id: row.get(9)?,
            diagnostic: row.get(10)?,
            metrics_json: row.get(11)?,
            recorded_at_ms: row.get(12)?,
        })
    };
    if let Some(user_id) = user_id {
        statement
            .query_map([user_id], map)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    } else {
        statement
            .query_map([], map)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

fn load_alerts(connection: &Connection, user_id: &str) -> Result<Vec<AlertRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT alert_id,user_id,entity_id,dimension,condition,policy_id,severity,state,
                    safety_action,first_event_id,first_critical_event_id,first_observed_at_ms,
                    occurrence_count,last_observed_at_ms,last_event_id,evidence_id,
                    correlation_id,diagnostic
             FROM operational_alerts WHERE user_id=?1",
        )
        .map_err(|e| e.to_string())?;
    statement
        .query_map([user_id], decode_alert)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn load_alert_for_key(
    connection: &Connection,
    user_id: &str,
    entity_id: &str,
    dimension: HealthDimension,
    condition: &str,
) -> Result<Option<AlertRecord>, String> {
    connection
        .query_row(
            "SELECT alert_id,user_id,entity_id,dimension,condition,policy_id,severity,state,
                    safety_action,first_event_id,first_critical_event_id,first_observed_at_ms,
                    occurrence_count,last_observed_at_ms,last_event_id,evidence_id,
                    correlation_id,diagnostic
             FROM operational_alerts
             WHERE user_id=?1 AND entity_id=?2 AND dimension=?3 AND condition=?4
               AND policy_id=?5",
            params![
                user_id,
                entity_id,
                enum_name(&dimension)?,
                condition,
                OPERATIONS_POLICY_ID
            ],
            decode_alert,
        )
        .optional()
        .map_err(|e| e.to_string())
}

fn load_alert_by_id(
    connection: &Connection,
    user_id: &str,
    alert_id: &str,
) -> Result<Option<AlertRecord>, String> {
    connection
        .query_row(
            "SELECT alert_id,user_id,entity_id,dimension,condition,policy_id,severity,state,
                    safety_action,first_event_id,first_critical_event_id,first_observed_at_ms,
                    occurrence_count,last_observed_at_ms,last_event_id,evidence_id,
                    correlation_id,diagnostic
             FROM operational_alerts WHERE alert_id=?1 AND user_id=?2",
            params![alert_id, user_id],
            decode_alert,
        )
        .optional()
        .map_err(|e| e.to_string())
}

fn decode_alert(row: &Row<'_>) -> rusqlite::Result<AlertRecord> {
    Ok(AlertRecord {
        alert_id: row.get(0)?,
        user_id: row.get(1)?,
        entity_id: row.get(2)?,
        dimension: parse(row.get(3)?)?,
        condition: row.get(4)?,
        policy_id: row.get(5)?,
        severity: parse(row.get(6)?)?,
        state: parse(row.get(7)?)?,
        safety_action: parse(row.get(8)?)?,
        first_event_id: row.get(9)?,
        first_critical_event_id: row.get(10)?,
        first_observed_at_ms: row.get(11)?,
        occurrence_count: row.get(12)?,
        last_observed_at_ms: row.get(13)?,
        last_event_id: row.get(14)?,
        evidence_id: row.get(15)?,
        correlation_id: row.get(16)?,
        diagnostic: row.get(17)?,
    })
}

fn append_lifecycle(
    connection: &Connection,
    alert_id: &str,
    user_id: &str,
    state: AlertState,
    event_id: &str,
    occurred_at_ms: i64,
    actor: &str,
) -> Result<(), String> {
    let state_name = enum_name(&state)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM operational_alert_lifecycle
                WHERE alert_id=?1 AND user_id=?2 AND state=?3 AND event_id=?4
            )",
            params![alert_id, user_id, state_name, event_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        connection
            .execute(
                "INSERT INTO operational_alert_lifecycle
                 (lifecycle_id,alert_id,user_id,state,event_id,occurred_at_ms,actor)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    Uuid::new_v4().to_string(),
                    alert_id,
                    user_id,
                    state_name,
                    event_id,
                    occurred_at_ms,
                    actor,
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn health_metadata(event: &OperationalEvent) -> (HealthState, bool, String) {
    let state = event
        .evidence
        .get("state")
        .and_then(Value::as_str)
        .and_then(|value| parse_json(value).ok())
        .unwrap_or(HealthState::Unknown);
    let required = event
        .evidence
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let condition = event
        .evidence
        .get("condition")
        .and_then(Value::as_str)
        .unwrap_or(&event.kind)
        .to_owned();
    (state, required, condition)
}

fn insert_health_metadata(
    evidence: &mut Value,
    state: HealthState,
    required: bool,
    condition: &str,
) -> Result<(), String> {
    if let Value::Object(map) = evidence {
        map.insert("state".into(), Value::String(enum_name(&state)?));
        map.insert("required".into(), Value::Bool(required));
        map.insert(
            "condition".into(),
            Value::String(bounded_redacted(condition)),
        );
        map.insert(
            "policyId".into(),
            Value::String(OPERATIONS_POLICY_ID.into()),
        );
        Ok(())
    } else {
        Err("operational evidence must be a JSON object".into())
    }
}

fn severity_for(state: HealthState, required: bool) -> AlertSeverity {
    match state {
        HealthState::Critical => AlertSeverity::Critical,
        HealthState::Unknown if required => AlertSeverity::Critical,
        HealthState::Degraded | HealthState::Unknown => AlertSeverity::Warning,
        HealthState::Healthy => AlertSeverity::Info,
    }
}

fn health_priority(state: HealthState, required: bool) -> u8 {
    match state {
        HealthState::Healthy => 0,
        HealthState::Degraded => 1,
        HealthState::Unknown if required => 3,
        HealthState::Critical => 4,
        HealthState::Unknown => 2,
    }
}

fn higher_severity(left: AlertSeverity, right: AlertSeverity) -> AlertSeverity {
    if matches!(left, AlertSeverity::Critical) || matches!(right, AlertSeverity::Critical) {
        AlertSeverity::Critical
    } else if matches!(left, AlertSeverity::Warning) || matches!(right, AlertSeverity::Warning) {
        AlertSeverity::Warning
    } else {
        AlertSeverity::Info
    }
}

fn safety_action(dimension: HealthDimension, state: HealthState, required: bool) -> SafetyAction {
    if !required {
        return SafetyAction::None;
    }
    if state == HealthState::Degraded {
        return SafetyAction::SkipDecision;
    }
    if matches!(state, HealthState::Unknown | HealthState::Critical) {
        return match dimension {
            HealthDimension::Worker | HealthDimension::FeatureModelStrategy => {
                SafetyAction::FaultAndReconcile
            }
            HealthDimension::LocalSystem => SafetyAction::FreezeAll,
            HealthDimension::PaperAccount
            | HealthDimension::RiskOms
            | HealthDimension::ExecutionAdapter => SafetyAction::Pause,
            HealthDimension::MarketData | HealthDimension::ResearchFeedback => {
                SafetyAction::SkipDecision
            }
        };
    }
    SafetyAction::None
}

fn safety_action_for_event(
    dimension: HealthDimension,
    state: HealthState,
    required: bool,
    condition: &str,
) -> SafetyAction {
    if state == HealthState::Degraded {
        if !required {
            return SafetyAction::None;
        }
        return match dimension {
            HealthDimension::MarketData if condition.contains("stale") => SafetyAction::Pause,
            HealthDimension::PaperAccount
            | HealthDimension::RiskOms
            | HealthDimension::ExecutionAdapter
                if condition.contains("uncertain") || condition.contains("stale") =>
            {
                SafetyAction::Pause
            }
            _ => SafetyAction::None,
        };
    }
    safety_action(dimension, state, required)
}

fn validate_observation(observation: &HealthObservation) -> Result<(), String> {
    validate_user(&observation.user_id)?;
    if !bounded_nonempty(&observation.entity_id, 256)
        || !bounded_nonempty(&observation.condition, 128)
        || observation.observed_at_ms <= 0
    {
        return Err("invalid operational observation".into());
    }
    if let Some(kind) = &observation.event_kind {
        validate_kind(kind)?;
    }
    for id in [
        observation.evidence_id.as_deref(),
        observation.correlation_id.as_deref(),
        observation.causation_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !bounded_nonempty(id, 256) {
            return Err("operational evidence identity is invalid".into());
        }
    }
    Ok(())
}

fn normalize_metrics(
    explicit: &BTreeMap<String, f64>,
    evidence: &Value,
) -> Result<BTreeMap<String, f64>, String> {
    let mut metrics = explicit.clone();
    if let Some(Value::Object(values)) = evidence.get("metrics") {
        for (key, value) in values {
            let number = value
                .as_f64()
                .ok_or_else(|| "operational metrics must be numeric".to_string())?;
            metrics.entry(key.clone()).or_insert(number);
        }
    }
    if metrics.len() > MAX_METRICS
        || metrics.iter().any(|(key, value)| {
            !bounded_nonempty(key, MAX_METRIC_KEY_BYTES) || sensitive_key(key) || !value.is_finite()
        })
    {
        return Err("operational metrics exceed the Host bound".into());
    }
    Ok(metrics)
}

fn redact_and_bound(mut value: Value) -> Result<Value, String> {
    redact_value(&mut value);
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_EVENT_BYTES {
        return Err("operational evidence exceeds the Host retention bound".into());
    }
    if !value.is_object() {
        return Err("operational evidence must be a JSON object".into());
    }
    Ok(value)
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if sensitive_key(key) {
                    *value = Value::String("[REDACTED]".into());
                } else {
                    redact_value(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        Value::String(value) => *value = bounded_redacted(value),
        _ => {}
    }
}

fn bounded_redacted(value: &str) -> String {
    let mut redacted = redact_text(value);
    if redacted.len() > MAX_DIAGNOSTIC_BYTES {
        let mut end = MAX_DIAGNOSTIC_BYTES;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        redacted.truncate(end);
    }
    redacted
}

fn redact_text(value: &str) -> String {
    let mut output = value.to_owned();
    redact_after_marker(&mut output, "Bearer ", "[REDACTED]");
    redact_after_marker(&mut output, "Basic ", "[REDACTED]");
    for marker in [
        "api_key=",
        "api-key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "passphrase=",
        "authorization=",
        "authorization:",
    ] {
        redact_after_marker(&mut output, marker, "[REDACTED]");
    }
    while let Some(start) = output.find("-----BEGIN") {
        let Some(end_offset) = output[start..].find("-----END") else {
            output.replace_range(start.., "[REDACTED_KEY]");
            break;
        };
        let end_start = start + end_offset;
        let end_marker = end_start + "-----END".len();
        let end = output[end_marker..]
            .find("-----")
            .map(|offset| end_marker + offset + "-----".len())
            .unwrap_or(output.len());
        output.replace_range(start..end, "[REDACTED_KEY]");
    }
    for marker in ["/Users/", "/home/", "C:\\Users\\", "/private/var/"] {
        while let Some(start) = output.find(marker) {
            let end = output[start..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'')
                })
                .map(|offset| start + offset)
                .unwrap_or(output.len());
            output.replace_range(start..end, "[REDACTED_PATH]");
        }
    }
    output
}

fn redact_after_marker(value: &mut String, marker: &str, replacement: &str) {
    let marker = marker.to_ascii_lowercase();
    while let Some(start) = value.to_ascii_lowercase().find(&marker) {
        let token_start = start + marker.len();
        let token_end = value[token_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '"' | '\'' | ',' | ';' | '&' | '}' | ']')
            })
            .map(|offset| token_start + offset)
            .unwrap_or(value.len());
        value.replace_range(start..token_end, replacement);
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "secret",
        "token",
        "apikey",
        "api_key",
        "api-key",
        "credential",
        "passphrase",
        "authorization",
        "privatekey",
        "private_key",
        "signing",
        "path",
        "directory",
    ]
    .iter()
    .any(|part| key.contains(part))
}

fn extract_identity(evidence: &Value, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = evidence else {
        return None;
    };
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn validate_evidence_identity(
    observation: &HealthObservation,
    evidence: &Value,
) -> Result<(), String> {
    let expected_ids = [
        (
            observation.evidence_id.as_deref(),
            &["evidenceId", "observationId", "reportId", "attemptId"][..],
        ),
        (
            observation.correlation_id.as_deref(),
            &["correlationId", "requestId"][..],
        ),
        (
            observation.causation_id.as_deref(),
            &["causationId", "eventId"][..],
        ),
    ];
    for (expected, keys) in expected_ids {
        let Some(expected) = expected else {
            continue;
        };
        if let Some(actual) = extract_identity(evidence, keys)
            && actual != expected
        {
            return Err("operational evidence identity does not match the Host observation".into());
        }
    }
    let entity_key = match observation.dimension {
        HealthDimension::Worker | HealthDimension::FeatureModelStrategy => Some("botId"),
        HealthDimension::PaperAccount => Some("accountId"),
        HealthDimension::MarketData => Some("snapshotId"),
        HealthDimension::ResearchFeedback => Some("reportId"),
        HealthDimension::RiskOms
        | HealthDimension::ExecutionAdapter
        | HealthDimension::LocalSystem => None,
    };
    if let Some(key) = entity_key
        && let Some(actual) = evidence.get(key).and_then(Value::as_str)
        && actual != observation.entity_id
    {
        return Err("operational evidence entity does not match the Host observation".into());
    }
    if let Some(actual) = evidence.get("userId").and_then(Value::as_str)
        && actual != observation.user_id
    {
        return Err("operational evidence User does not match the Host session".into());
    }
    Ok(())
}

fn validate_kind(kind: &str) -> Result<(), String> {
    if !bounded_nonempty(kind, 64)
        || !kind.bytes().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || b"._-".contains(&character)
        })
    {
        return Err("operational event kind is invalid".into());
    }
    Ok(())
}

fn bounded_nonempty(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max
        && value.chars().all(|character| !character.is_control())
}

fn stable_alert_id(
    user_id: &str,
    entity_id: &str,
    dimension: HealthDimension,
    condition: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update([0]);
    hasher.update(entity_id.as_bytes());
    hasher.update([0]);
    hasher.update(enum_name(&dimension).unwrap_or_default().as_bytes());
    hasher.update([0]);
    hasher.update(condition.as_bytes());
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("alert-{hex}")
}

use crate::user::validate_user;

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .any(|name| name == column);
    if !exists {
        connection
            .execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn enum_name<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|e| e.to_string())
        .map(|value| value.trim_matches('"').to_owned())
}

fn parse<T: for<'de> Deserialize<'de>>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_json<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, String> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(not(test))]
use std::process::Command;

#[cfg(not(test))]
fn notify_native(alert: &AlertView) {
    let title = "AdaQ Operational Alert";
    let body = bounded_redacted(&format!(
        "{}: {} ({})",
        alert.condition,
        alert.entity_id,
        enum_name(&alert.severity).unwrap_or_else(|_| "critical".into())
    ));
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('\\', "\\\\").replace('"', "\\\""),
            title
        );
        let _ = Command::new("osascript").args(["-e", &script]).status();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send").args([title, &body]).status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("msg").args(["*", &body]).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> OperationsStore {
        OperationsStore::open(Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))).unwrap()
    }
    fn obs(state: HealthState, required: bool) -> HealthObservation {
        HealthObservation {
            user_id: "u".into(),
            entity_id: "bot".into(),
            dimension: HealthDimension::Worker,
            state,
            condition: "heartbeat".into(),
            evidence: serde_json::json!({"state":state,"required":required,"token":"secret"}),
            required,
            observed_at_ms: 1,
            event_kind: None,
            evidence_id: None,
            correlation_id: None,
            causation_id: None,
            diagnostic: None,
            metrics: BTreeMap::new(),
        }
    }
    #[test]
    fn critical_worker_faults_and_redacts() {
        let s = store();
        let (_, a, action) = s.observe(obs(HealthState::Critical, true)).unwrap();
        assert_eq!(action, SafetyAction::FaultAndReconcile);
        assert!(s.blocks_new_risk("u").unwrap());
        assert_eq!(a.unwrap().severity, AlertSeverity::Critical);
        let event = s.health_for_user("u").unwrap();
        assert_eq!(event.len(), 1);
        assert_eq!(event[0].state, HealthState::Critical);
        let alerts = s.alerts_for_user("u").unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].state, AlertState::Active);
        s.transition_alert(
            "u",
            &alerts[0].alert_id,
            AlertState::Acknowledged,
            &event[0].event_id,
            2,
        )
        .unwrap();
        assert_eq!(
            s.alerts_for_user("u").unwrap()[0].state,
            AlertState::Acknowledged
        );
    }
    #[test]
    fn unknown_required_fails_closed() {
        let s = store();
        let (_, _, action) = s.observe(obs(HealthState::Unknown, true)).unwrap();
        assert_eq!(action, SafetyAction::FaultAndReconcile);
    }
    #[test]
    fn healthy_does_not_create_alert() {
        let s = store();
        let (_, a, action) = s.observe(obs(HealthState::Healthy, true)).unwrap();
        assert!(a.is_none());
        assert_eq!(action, SafetyAction::None);
    }

    #[test]
    fn recovery_resolves_existing_alert_and_keeps_history() {
        let s = store();
        let (event, alert, _) = s.observe(obs(HealthState::Critical, true)).unwrap();
        let alert = alert.unwrap();
        let mut recovery = obs(HealthState::Healthy, true);
        recovery.observed_at_ms = 2;
        s.observe(recovery).unwrap();
        assert_eq!(
            s.alerts_for_user("u").unwrap()[0].state,
            AlertState::Resolved
        );
        let history = s.alert_history_for_user("u", &alert.alert_id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].state, AlertState::Resolved);
        assert!(
            s.transition_alert(
                "u",
                &alert.alert_id,
                AlertState::Acknowledged,
                &event.event_id,
                2
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_faults_preserve_acknowledgement_and_first_critical_evidence() {
        let s = store();
        let mut first = obs(HealthState::Critical, true);
        first.observed_at_ms = 10;
        first.evidence_id = Some("worker-fault-1".into());
        let (event, alert, _) = s.observe(first).unwrap();
        let alert = alert.unwrap();
        assert_eq!(alert.first_event_id, event.event_id);
        s.acknowledge("u", &alert.alert_id, 11).unwrap();

        let mut repeated = obs(HealthState::Critical, true);
        repeated.observed_at_ms = 12;
        repeated.evidence_id = Some("worker-fault-2".into());
        let (repeated_event, repeated_alert, _) = s.observe(repeated).unwrap();
        assert!(repeated_alert.is_some());
        let current = &s.alerts_for_user("u").unwrap()[0];
        assert_eq!(current.state, AlertState::Acknowledged);
        assert_eq!(current.first_event_id, event.event_id);
        assert_eq!(current.last_event_id, repeated_event.event_id);
        assert_eq!(current.occurrence_count, 2);
        assert_eq!(current.first_critical_event_id, Some(event.event_id));
    }

    #[test]
    fn first_critical_event_is_retained_after_a_warning() {
        let s = store();
        let mut warning = obs(HealthState::Degraded, true);
        warning.condition = "worker_latency".into();
        warning.observed_at_ms = 1;
        let (_, warning_alert, _) = s.observe(warning).unwrap();
        let warning_alert = warning_alert.unwrap();

        let mut critical = obs(HealthState::Critical, true);
        critical.condition = "worker_latency".into();
        critical.observed_at_ms = 2;
        let (critical_event, alert, _) = s.observe(critical).unwrap();
        let alert = alert.unwrap();
        assert_eq!(alert.first_event_id, warning_alert.first_event_id);
        assert_eq!(alert.first_critical_event_id, Some(critical_event.event_id));
    }

    #[test]
    fn health_projection_keeps_unresolved_conditions_visible() {
        let s = store();
        let mut fault = obs(HealthState::Critical, true);
        fault.condition = "worker_fault".into();
        fault.observed_at_ms = 1;
        s.observe(fault).unwrap();

        let mut heartbeat = obs(HealthState::Healthy, true);
        heartbeat.condition = "worker_heartbeat".into();
        heartbeat.observed_at_ms = 2;
        s.observe(heartbeat).unwrap();

        let health = s.health_for_user("u").unwrap();
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].state, HealthState::Critical);
        assert_eq!(health[0].condition, "worker_fault");
    }

    #[test]
    fn fail_closed_action_survives_lower_severity_until_recovery() {
        let s = store();
        let (_, critical, action) = s.observe(obs(HealthState::Critical, true)).unwrap();
        let critical = critical.unwrap();
        assert_eq!(action, SafetyAction::FaultAndReconcile);

        let mut degraded = obs(HealthState::Degraded, true);
        degraded.observed_at_ms = 2;
        let (_, follow_up, action) = s.observe(degraded).unwrap();
        assert_eq!(action, SafetyAction::FaultAndReconcile);
        assert_eq!(
            follow_up.unwrap().safety_action,
            SafetyAction::FaultAndReconcile
        );
        assert_eq!(
            s.alerts_for_user("u").unwrap()[0].safety_action,
            SafetyAction::FaultAndReconcile
        );
        assert_ne!(
            s.alert_history_for_user("u", &critical.alert_id)
                .unwrap()
                .last()
                .unwrap()
                .state,
            AlertState::Resolved
        );
    }

    #[test]
    fn stale_observations_and_foreign_users_are_rejected() {
        let s = store();
        let mut current = obs(HealthState::Healthy, true);
        current.observed_at_ms = 20;
        s.observe(current).unwrap();

        let mut stale = obs(HealthState::Critical, true);
        stale.observed_at_ms = 19;
        assert!(s.observe(stale).unwrap_err().contains("stale"));
        assert!(s.alerts_for_user("other").unwrap().is_empty());

        let mut mismatched = obs(HealthState::Healthy, true);
        mismatched.observed_at_ms = 21;
        mismatched.evidence = serde_json::json!({ "botId": "other" });
        assert!(s.observe(mismatched).unwrap_err().contains("entity"));

        let mut sensitive_metric = obs(HealthState::Healthy, true);
        sensitive_metric.observed_at_ms = 22;
        sensitive_metric.metrics.insert("api_key".into(), 1.0);
        assert!(
            sensitive_metric
                .metrics
                .keys()
                .any(|key| sensitive_key(key))
        );
        assert!(s.observe(sensitive_metric).is_err());
    }

    #[test]
    fn typed_metadata_is_retained_while_diagnostics_are_redacted_and_bounded() {
        let s = store();
        let mut metrics = BTreeMap::new();
        metrics.insert("latency_ms".into(), 12.5);
        let observation = HealthObservation {
            user_id: "u".into(),
            entity_id: "worker".into(),
            dimension: HealthDimension::Worker,
            state: HealthState::Degraded,
            condition: "heartbeat".into(),
            evidence: serde_json::json!({
                "token": "secret",
            "detail": "Bearer abcdef apiKey=private-token /Users/private/secret.txt -----BEGIN PRIVATE KEY----- private -----END PRIVATE KEY-----"
            }),
            required: true,
            observed_at_ms: 1,
            event_kind: Some("worker.diagnostic".into()),
            evidence_id: Some("worker-evidence-1".into()),
            correlation_id: Some("run-1".into()),
            causation_id: Some("heartbeat-1".into()),
            diagnostic: Some(
                "Bearer abcdef apiKey=private-token /Users/private/secret.txt -----BEGIN PRIVATE KEY----- private -----END PRIVATE KEY-----"
                    .into(),
            ),
            metrics,
        };
        let (event, _, _) = s.observe(observation).unwrap();
        assert_eq!(event.kind, "worker.diagnostic");
        assert_eq!(event.evidence_id.as_deref(), Some("worker-evidence-1"));
        assert_eq!(event.correlation_id.as_deref(), Some("run-1"));
        assert_eq!(event.causation_id.as_deref(), Some("heartbeat-1"));
        assert_eq!(event.metrics.get("latency_ms"), Some(&12.5));
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains("abcdef"));
        assert!(!serialized.contains("private-token"));
        assert!(!serialized.contains("/Users/private"));
        assert!(!serialized.contains("PRIVATE KEY"));
        assert!(serialized.contains("[REDACTED]"));
    }

    #[test]
    fn restarting_the_store_rebuilds_missing_alert_projection_from_events() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let s = OperationsStore::open(database.clone()).unwrap();
        let (_, first_alert, _) = s.observe(obs(HealthState::Critical, true)).unwrap();
        let first_alert = first_alert.unwrap();
        let mut second = obs(HealthState::Critical, true);
        second.entity_id = "bot-2".into();
        s.observe(second).unwrap();
        let connection = database.lock().unwrap();
        connection
            .execute(
                "DELETE FROM operational_safety_actions WHERE alert_id=?1",
                [&first_alert.alert_id],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM operational_alert_lifecycle WHERE alert_id=?1",
                [&first_alert.alert_id],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM operational_alerts WHERE alert_id=?1",
                [&first_alert.alert_id],
            )
            .unwrap();
        drop(connection);
        drop(s);

        let restarted = OperationsStore::open(database).unwrap();
        let alerts = restarted.alerts_for_user("u").unwrap();
        assert_eq!(alerts.len(), 2);
        assert!(alerts.iter().all(|alert| alert.state == AlertState::Active));
    }

    #[test]
    fn required_health_states_map_to_frozen_fail_closed_actions() {
        let cases = [
            (HealthDimension::MarketData, SafetyAction::SkipDecision),
            (HealthDimension::Worker, SafetyAction::FaultAndReconcile),
            (
                HealthDimension::FeatureModelStrategy,
                SafetyAction::FaultAndReconcile,
            ),
            (HealthDimension::PaperAccount, SafetyAction::Pause),
            (HealthDimension::RiskOms, SafetyAction::Pause),
            (HealthDimension::ExecutionAdapter, SafetyAction::Pause),
            (HealthDimension::LocalSystem, SafetyAction::FreezeAll),
            (
                HealthDimension::ResearchFeedback,
                SafetyAction::SkipDecision,
            ),
        ];
        for (dimension, expected) in cases {
            assert_eq!(
                safety_action(dimension, HealthState::Critical, true),
                expected
            );
        }
        assert_eq!(
            safety_action_for_event(
                HealthDimension::MarketData,
                HealthState::Degraded,
                true,
                "latency_excursion"
            ),
            SafetyAction::None
        );
        assert_eq!(
            safety_action_for_event(
                HealthDimension::MarketData,
                HealthState::Degraded,
                true,
                "market_data_stale"
            ),
            SafetyAction::Pause
        );
    }

    #[test]
    fn freeze_all_alert_stays_active_until_host_recovery_evidence() {
        let s = store();
        let critical = HealthObservation {
            user_id: "u".into(),
            entity_id: "host".into(),
            dimension: HealthDimension::LocalSystem,
            state: HealthState::Critical,
            condition: "local_system_integrity".into(),
            evidence: serde_json::json!({ "sqliteIntegrity": "failed" }),
            required: true,
            observed_at_ms: 1,
            event_kind: Some("local.system-health".into()),
            evidence_id: Some("sqlite-check-1".into()),
            correlation_id: None,
            causation_id: None,
            diagnostic: Some("SQLite integrity failed".into()),
            metrics: BTreeMap::new(),
        };
        let (_, alert, action) = s.observe(critical).unwrap();
        let alert = alert.unwrap();
        assert_eq!(action, SafetyAction::FreezeAll);
        assert!(s.is_user_frozen("u").unwrap());
        s.acknowledge("u", &alert.alert_id, 2).unwrap();
        assert!(s.is_user_frozen("u").unwrap());

        s.observe(HealthObservation {
            user_id: "u".into(),
            entity_id: "host".into(),
            dimension: HealthDimension::LocalSystem,
            state: HealthState::Healthy,
            condition: "local_system_integrity".into(),
            evidence: serde_json::json!({ "sqliteIntegrity": "ok" }),
            required: true,
            observed_at_ms: 3,
            event_kind: Some("local.system-health".into()),
            evidence_id: Some("sqlite-check-3".into()),
            correlation_id: None,
            causation_id: None,
            diagnostic: Some("SQLite integrity check passed".into()),
            metrics: BTreeMap::new(),
        })
        .unwrap();
        assert!(!s.is_user_frozen("u").unwrap());
        assert!(!s.blocks_new_risk("u").unwrap());
    }
}
