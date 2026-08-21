//! Host-owned operational evidence, health, alerts, and fail-closed actions.
//!
//! This module is deliberately independent of the GUI. Runtimes submit typed
//! observations; only this host boundary persists evidence and derives safety.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const MAX_METRICS: usize = 32;

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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertView {
    pub alert_id: String,
    pub user_id: String,
    pub entity_id: String,
    pub dimension: HealthDimension,
    pub condition: String,
    pub severity: AlertSeverity,
    pub state: AlertState,
    pub safety_action: SafetyAction,
    pub last_event_id: String,
}

#[derive(Clone)]
pub struct OperationsStore {
    database: Arc<Mutex<Connection>>,
}

impl OperationsStore {
    pub fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        database.lock().map_err(|e| e.to_string())?.execute_batch("\
            CREATE TABLE IF NOT EXISTS operational_events (\
                event_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, entity_id TEXT NOT NULL,\
                dimension TEXT NOT NULL, kind TEXT NOT NULL, observed_at_ms INTEGER NOT NULL,\
                evidence_json TEXT NOT NULL\
            );\
            CREATE INDEX IF NOT EXISTS operational_events_user_time ON operational_events(user_id, observed_at_ms);\
            CREATE TABLE IF NOT EXISTS operational_alerts (\
                alert_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, entity_id TEXT NOT NULL,\
                dimension TEXT NOT NULL, condition TEXT NOT NULL, severity TEXT NOT NULL,\
                state TEXT NOT NULL, safety_action TEXT NOT NULL, last_event_id TEXT NOT NULL,\
                UNIQUE(user_id, entity_id, dimension, condition)\
            );\
            CREATE TABLE IF NOT EXISTS operational_alert_lifecycle (\
                lifecycle_id TEXT PRIMARY KEY, alert_id TEXT NOT NULL, user_id TEXT NOT NULL,\
                state TEXT NOT NULL, event_id TEXT NOT NULL, occurred_at_ms INTEGER NOT NULL,\
                FOREIGN KEY(alert_id) REFERENCES operational_alerts(alert_id)\
            );\
        ").map_err(|e| e.to_string())?;
        Ok(Self { database })
    }

    pub fn observe(
        &self,
        observation: HealthObservation,
    ) -> Result<(OperationalEvent, Option<AlertView>, SafetyAction), String> {
        if observation.user_id.trim().is_empty()
            || observation.entity_id.trim().is_empty()
            || observation.condition.len() > 128
        {
            return Err("invalid operational observation".into());
        }
        let event_id = Uuid::new_v4().to_string();
        let mut evidence = observation.evidence;
        if let Value::Object(map) = &mut evidence {
            map.insert(
                "state".into(),
                Value::String(
                    serde_json::to_string(&observation.state)
                        .map_err(|e| e.to_string())?
                        .trim_matches('"')
                        .into(),
                ),
            );
            map.insert("required".into(), Value::Bool(observation.required));
        }
        let evidence = redact_and_bound(evidence);
        let event = OperationalEvent {
            event_id: event_id.clone(),
            user_id: observation.user_id.clone(),
            entity_id: observation.entity_id.clone(),
            dimension: observation.dimension,
            kind: "health.observed".into(),
            observed_at_ms: observation.observed_at_ms,
            evidence,
        };
        let dimension = serde_json::to_string(&event.dimension)
            .map_err(|e| e.to_string())?
            .trim_matches('"')
            .to_string();
        let action = safety_action(
            observation.dimension,
            observation.state,
            observation.required,
        );
        let action_name = serde_json::to_string(&action)
            .map_err(|e| e.to_string())?
            .trim_matches('"')
            .to_string();
        let evidence_json = serde_json::to_string(&event.evidence).map_err(|e| e.to_string())?;
        let severity = if observation.state >= HealthState::Critical {
            AlertSeverity::Critical
        } else if observation.state == HealthState::Degraded {
            AlertSeverity::Warning
        } else {
            AlertSeverity::Info
        };
        let severity_name = serde_json::to_string(&severity)
            .map_err(|e| e.to_string())?
            .trim_matches('"')
            .to_string();
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO operational_events VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                event.event_id,
                event.user_id,
                event.entity_id,
                dimension,
                event.kind,
                event.observed_at_ms,
                evidence_json
            ],
        )
        .map_err(|e| e.to_string())?;
        let existing: Option<String> = conn.query_row("SELECT alert_id FROM operational_alerts WHERE user_id=?1 AND entity_id=?2 AND dimension=?3 AND condition=?4", params![observation.user_id, observation.entity_id, dimension, observation.condition], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        let alert = if observation.state >= HealthState::Degraded {
            let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
            let active_state = serde_json::to_string(&AlertState::Active)
                .map_err(|e| e.to_string())?
                .trim_matches('"')
                .to_string();
            conn.execute("INSERT INTO operational_alerts VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(user_id,entity_id,dimension,condition) DO UPDATE SET severity=excluded.severity,state=excluded.state,safety_action=excluded.safety_action,last_event_id=excluded.last_event_id", params![id, observation.user_id, observation.entity_id, dimension, observation.condition, severity_name, active_state, action_name, event.event_id]).map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO operational_alert_lifecycle VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    observation.user_id,
                    active_state,
                    event.event_id,
                    observation.observed_at_ms
                ],
            )
            .map_err(|e| e.to_string())?;
            Some(AlertView {
                alert_id: id,
                user_id: observation.user_id,
                entity_id: observation.entity_id,
                dimension: observation.dimension,
                condition: observation.condition,
                severity,
                state: AlertState::Active,
                safety_action: action,
                last_event_id: event.event_id.clone(),
            })
        } else if let Some(id) = existing {
            let resolved = serde_json::to_string(&AlertState::Resolved)
                .map_err(|e| e.to_string())?
                .trim_matches('"')
                .to_string();
            let changed = conn.execute(
                "UPDATE operational_alerts SET state=?1,last_event_id=?2 WHERE alert_id=?3 AND user_id=?4 AND state != ?1",
                params![resolved, event.event_id, id, observation.user_id],
            ).map_err(|e| e.to_string())?;
            if changed == 1 {
                conn.execute(
                    "INSERT INTO operational_alert_lifecycle VALUES (?1,?2,?3,?4,?5,?6)",
                    params![Uuid::new_v4().to_string(), id, observation.user_id, resolved, event.event_id, observation.observed_at_ms],
                ).map_err(|e| e.to_string())?;
            }
            None
        } else {
            None
        };
        Ok((event, alert, action))
    }

    pub fn health_for_user(&self, user_id: &str) -> Result<Vec<HealthView>, String> {
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT e.entity_id,e.dimension,e.evidence_json,e.observed_at_ms,e.event_id FROM operational_events e JOIN (SELECT entity_id,dimension,MAX(observed_at_ms) t FROM operational_events WHERE user_id=?1 GROUP BY entity_id,dimension) latest ON latest.entity_id=e.entity_id AND latest.dimension=e.dimension AND latest.t=e.observed_at_ms WHERE e.user_id=?1 ORDER BY e.entity_id,e.dimension").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([user_id], |r| {
                let d: String = r.get(1)?;
                let evidence: Value =
                    serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null);
                Ok(HealthView {
                    entity_id: r.get(0)?,
                    dimension: serde_json::from_str(&format!("\"{d}\""))
                        .unwrap_or(HealthDimension::LocalSystem),
                    state: evidence
                        .get("state")
                        .and_then(Value::as_str)
                        .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
                        .unwrap_or(HealthState::Unknown),
                    required: evidence
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    observed_at_ms: r.get(3)?,
                    event_id: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn transition_alert(
        &self,
        user_id: &str,
        alert_id: &str,
        state: AlertState,
        event_id: &str,
        occurred_at_ms: i64,
    ) -> Result<(), String> {
        let state_name = serde_json::to_string(&state)
            .map_err(|e| e.to_string())?
            .trim_matches('"')
            .to_string();
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let current: Option<String> = conn
            .query_row(
                "SELECT state FROM operational_alerts WHERE alert_id=?1 AND user_id=?2",
                params![alert_id, user_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let current = current.ok_or_else(|| "operational alert was not found for User".to_string())?;
        let event_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operational_events WHERE event_id=?1 AND user_id=?2)",
                params![event_id, user_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !event_exists {
            return Err("operational transition must reference User evidence".into());
        }
        let legal = matches!((current.as_str(), state), ("active", AlertState::Acknowledged | AlertState::Resolved) | ("acknowledged", AlertState::Resolved));
        if !legal {
            return Err("invalid operational alert lifecycle transition".into());
        }
        conn.execute(
            "UPDATE operational_alerts SET state=?1 WHERE alert_id=?2 AND user_id=?3",
            params![state_name, alert_id, user_id],
        ).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO operational_alert_lifecycle VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                Uuid::new_v4().to_string(),
                alert_id,
                user_id,
                state_name,
                event_id,
                occurred_at_ms
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn alerts_for_user(&self, user_id: &str) -> Result<Vec<AlertView>, String> {
        let conn = self.database.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT alert_id,user_id,entity_id,dimension,condition,severity,state,safety_action,last_event_id FROM operational_alerts WHERE user_id=?1 ORDER BY alert_id").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([user_id], |r| {
                Ok(AlertView {
                    alert_id: r.get(0)?,
                    user_id: r.get(1)?,
                    entity_id: r.get(2)?,
                    dimension: parse(r.get(3)?)?,
                    condition: r.get(4)?,
                    severity: parse(r.get(5)?)?,
                    state: parse(r.get(6)?)?,
                    safety_action: parse(r.get(7)?)?,
                    last_event_id: r.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

fn parse<T: for<'de> Deserialize<'de>>(s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{s}\"")).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn safety_action(d: HealthDimension, s: HealthState, required: bool) -> SafetyAction {
    if !required || s < HealthState::Critical && s != HealthState::Unknown {
        return if s == HealthState::Degraded {
            SafetyAction::SkipDecision
        } else {
            SafetyAction::None
        };
    }
    if s == HealthState::Unknown || s == HealthState::Critical {
        match d {
            HealthDimension::Worker | HealthDimension::FeatureModelStrategy => {
                SafetyAction::FaultAndReconcile
            }
            HealthDimension::LocalSystem => SafetyAction::FreezeAll,
            HealthDimension::PaperAccount
            | HealthDimension::RiskOms
            | HealthDimension::ExecutionAdapter => SafetyAction::Pause,
            _ => SafetyAction::SkipDecision,
        }
    } else {
        SafetyAction::None
    }
}
fn redact_and_bound(mut value: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if [
                        "password",
                        "secret",
                        "token",
                        "apiKey",
                        "credential",
                        "passphrase",
                    ]
                    .iter()
                    .any(|s| k.to_ascii_lowercase().contains(&s.to_ascii_lowercase()))
                    {
                        *x = Value::String("[REDACTED]".into());
                    } else {
                        walk(x);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            Value::String(s) => {
                if s.len() > MAX_DIAGNOSTIC_BYTES {
                    s.truncate(MAX_DIAGNOSTIC_BYTES);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    let mut metrics = 0;
    if let Value::Object(m) = &mut value {
        m.retain(|k, _| {
            if k.to_ascii_lowercase().contains("metric") {
                metrics += 1;
                metrics <= MAX_METRICS
            } else {
                true
            }
        });
        m.insert("_bounded".into(), Value::Bool(true));
    }
    value
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
        }
    }
    #[test]
    fn critical_worker_faults_and_redacts() {
        let s = store();
        let (_, a, action) = s.observe(obs(HealthState::Critical, true)).unwrap();
        assert_eq!(action, SafetyAction::FaultAndReconcile);
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
        s.observe(obs(HealthState::Healthy, true)).unwrap();
        assert_eq!(s.alerts_for_user("u").unwrap()[0].state, AlertState::Resolved);
        let conn = s.database.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM operational_alert_lifecycle WHERE alert_id=?1",
            [&alert.alert_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2);
        drop(conn);
        assert!(s.transition_alert("u", &alert.alert_id, AlertState::Acknowledged, &event.event_id, 2).is_err());
    }
}
