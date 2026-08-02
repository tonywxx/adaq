use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const INCOMPATIBLE_SCHEMA: &str = "Incompatible pre-v1 Dataset Generation schema. Close AdaQ, remove its device-local app data directory, and reopen AdaQ. This deletes all Local Research Data for every User on this device.";
const MAX_DIAGNOSTIC_EVIDENCE_CHARS: usize = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TryFrom<&str> for AttemptStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!(
                "unknown Dataset Generation Attempt status: {value}"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DiagnosticCode {
    GenerationInterrupted,
    GenerationFailed,
    PublicationFailed,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Diagnostic {
    code: DiagnosticCode,
    details: String,
}

impl Diagnostic {
    fn generation_interrupted(previous: Option<String>) -> Self {
        let mut details = "application stopped before completion".to_owned();
        if let Some(previous) = previous {
            let previous = serde_json::from_str::<Self>(&previous)
                .map(Self::evidence)
                .unwrap_or(previous);
            details.push_str("; previous diagnostic: ");
            details.push_str(&previous);
        }
        Self {
            code: DiagnosticCode::GenerationInterrupted,
            details,
        }
    }

    pub(crate) fn generation_failed(details: impl Into<String>) -> Self {
        Self::bounded(DiagnosticCode::GenerationFailed, details)
    }

    pub(crate) fn publication_failed(details: impl Into<String>) -> Self {
        Self::bounded(DiagnosticCode::PublicationFailed, details)
    }

    fn bounded(code: DiagnosticCode, details: impl Into<String>) -> Self {
        let available =
            MAX_DIAGNOSTIC_EVIDENCE_CHARS.saturating_sub(code.persisted().chars().count() + 2);
        Self {
            code,
            details: details.into().chars().take(available).collect(),
        }
    }

    fn evidence(self) -> String {
        format!("{}: {}", self.code.persisted(), self.details)
    }
}

impl DiagnosticCode {
    fn persisted(self) -> &'static str {
        match self {
            Self::GenerationInterrupted => "generation-interrupted",
            Self::GenerationFailed => "generation-failed",
            Self::PublicationFailed => "publication-failed",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attempt {
    pub(crate) attempt_id: String,
    pub(crate) dataset_id: Option<String>,
    pub(crate) status: AttemptStatus,
    pub(crate) diagnostic_evidence: Option<String>,
    pub(crate) progress_completed: i64,
    pub(crate) progress_total: i64,
}

pub(crate) struct PreparedAttempt {
    pub(crate) attempt: Attempt,
    pub(crate) should_start: bool,
}

pub(crate) struct AttemptStore<'a> {
    database: &'a Connection,
}

impl<'a> AttemptStore<'a> {
    pub(crate) fn new(database: &'a Connection) -> Self {
        Self { database }
    }

    pub(crate) fn initialize(&self) -> Result<(), String> {
        self.database
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS dataset_generation_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    request_hash TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    dataset_id TEXT,
                    status TEXT NOT NULL,
                    request_json TEXT NOT NULL,
                    diagnostic_json TEXT,
                    progress_completed INTEGER NOT NULL DEFAULT 0,
                    progress_total INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );",
            )
            .map_err(string)?;
        let mut statement = self
            .database
            .prepare("PRAGMA table_info(dataset_generation_attempts)")
            .map_err(string)?;
        let actual = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let expected = vec![
            ("attempt_id".into(), "TEXT".into(), false, None, true),
            ("request_hash".into(), "TEXT".into(), true, None, false),
            ("user_id".into(), "TEXT".into(), true, None, false),
            ("dataset_id".into(), "TEXT".into(), false, None, false),
            ("status".into(), "TEXT".into(), true, None, false),
            ("request_json".into(), "TEXT".into(), true, None, false),
            ("diagnostic_json".into(), "TEXT".into(), false, None, false),
            (
                "progress_completed".into(),
                "INTEGER".into(),
                true,
                Some("0".into()),
                false,
            ),
            (
                "progress_total".into(),
                "INTEGER".into(),
                true,
                Some("0".into()),
                false,
            ),
            (
                "created_at".into(),
                "TEXT".into(),
                true,
                Some("CURRENT_TIMESTAMP".into()),
                false,
            ),
        ];
        if actual != expected {
            return Err(INCOMPATIBLE_SCHEMA.into());
        }
        let statuses = {
            let mut statement = self
                .database
                .prepare("SELECT DISTINCT status FROM dataset_generation_attempts")
                .map_err(string)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(string)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(string)?
        };
        for status in statuses {
            AttemptStatus::try_from(status.as_str())?;
        }
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        let unfinished = {
            let mut statement = transaction
                .prepare(
                    "SELECT attempt_id, diagnostic_json
                 FROM dataset_generation_attempts
                 WHERE status IN ('pending', 'running')",
                )
                .map_err(string)?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(string)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(string)?
        };
        for (attempt_id, previous) in unfinished {
            let diagnostic = serde_json::to_string(&Diagnostic::generation_interrupted(previous))
                .map_err(string)?;
            transaction
                .execute(
                    "UPDATE dataset_generation_attempts
                 SET status = 'failed', diagnostic_json = ?2
                 WHERE attempt_id = ?1 AND status IN ('pending', 'running')",
                    params![attempt_id, diagnostic],
                )
                .map_err(string)?;
        }
        transaction.commit().map_err(string)
    }

    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<Attempt>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id, dataset_id, status, diagnostic_json,
                    progress_completed, progress_total
             FROM dataset_generation_attempts
             WHERE user_id = ?1
             ORDER BY created_at DESC",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(string)?;
        rows.map(|row| {
            let (attempt_id, dataset_id, status, diagnostic, progress_completed, progress_total) =
                row.map_err(string)?;
            attempt_from_parts(
                attempt_id,
                dataset_id,
                status,
                diagnostic,
                progress_completed,
                progress_total,
            )
        })
        .collect()
    }

    pub(crate) fn prepare(
        &self,
        request_hash: &str,
        user_id: &str,
        request_json: &str,
        new_attempt_id: impl FnOnce() -> String,
    ) -> Result<PreparedAttempt, String> {
        if let Some(parts) = self
            .database
            .query_row(
                "SELECT attempt_id, dataset_id, status, diagnostic_json,
                    progress_completed, progress_total
             FROM dataset_generation_attempts
             WHERE request_hash = ?1 AND user_id = ?2
               AND status IN ('pending', 'running', 'completed')
             ORDER BY CASE WHEN status IN ('pending', 'running') THEN 0 ELSE 1 END,
                      created_at DESC
             LIMIT 1",
                params![request_hash, user_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(string)?
        {
            return Ok(PreparedAttempt {
                attempt: attempt_from_parts(parts.0, parts.1, parts.2, parts.3, parts.4, parts.5)?,
                should_start: false,
            });
        }
        let new_attempt_id = new_attempt_id();
        self.database
            .execute(
                "INSERT INTO dataset_generation_attempts
             (attempt_id, request_hash, user_id, status, request_json)
             VALUES (?1, ?2, ?3, 'pending', ?4)",
                params![new_attempt_id, request_hash, user_id, request_json],
            )
            .map_err(string)?;
        Ok(PreparedAttempt {
            attempt: Attempt {
                attempt_id: new_attempt_id,
                dataset_id: None,
                status: AttemptStatus::Pending,
                diagnostic_evidence: None,
                progress_completed: 0,
                progress_total: 0,
            },
            should_start: true,
        })
    }

    pub(crate) fn prepare_retry<T>(
        &self,
        attempt_id: &str,
        user_id: &str,
        new_attempt_id: impl FnOnce(&str) -> String,
        validate_request: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<(PreparedAttempt, T), String> {
        let (request_hash, request_json): (String, String) = self
            .database
            .query_row(
                "SELECT request_hash, request_json FROM dataset_generation_attempts
             WHERE attempt_id = ?1 AND user_id = ?2
               AND status IN ('failed', 'cancelled')",
                params![attempt_id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Dataset Generation Attempt cannot be retried".to_owned())?;
        let request = validate_request(&request_json)?;
        let retry_attempt_id = new_attempt_id(&request_hash);
        self.database
            .execute(
                "INSERT INTO dataset_generation_attempts
             (attempt_id, request_hash, user_id, status, request_json)
             VALUES (?1, ?2, ?3, 'pending', ?4)",
                params![retry_attempt_id, request_hash, user_id, request_json],
            )
            .map_err(string)?;
        Ok((
            PreparedAttempt {
                attempt: Attempt {
                    attempt_id: retry_attempt_id,
                    dataset_id: None,
                    status: AttemptStatus::Pending,
                    diagnostic_evidence: None,
                    progress_completed: 0,
                    progress_total: 0,
                },
                should_start: true,
            },
            request,
        ))
    }

    pub(crate) fn mark_running(&self, attempt_id: &str) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'running'
             WHERE attempt_id = ?1 AND status = 'pending'",
                [attempt_id],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn request_cancellation(
        &self,
        attempt_id: &str,
        user_id: &str,
    ) -> Result<bool, String> {
        if self
            .database
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'cancelled'
             WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'pending'",
                params![attempt_id, user_id],
            )
            .map_err(string)?
            == 1
        {
            return Ok(true);
        }
        self.database
            .query_row(
                "SELECT status FROM dataset_generation_attempts
             WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|status| matches!(status.as_deref(), Some("running" | "cancelled")))
            .map_err(string)
    }

    pub(crate) fn mark_cancelled_after_exit(
        &self,
        attempt_id: &str,
        user_id: &str,
    ) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'cancelled'
             WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'running'",
                params![attempt_id, user_id],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn mark_completed(
        &self,
        attempt_id: &str,
        dataset_id: &str,
    ) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE dataset_generation_attempts
             SET status = 'completed', dataset_id = ?2
             WHERE attempt_id = ?1 AND status = 'running'",
                params![attempt_id, dataset_id],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn reuse_completed_dataset(&self, attempt_id: &str) -> Result<bool, String> {
        let reused = self
            .database
            .query_row(
                "SELECT prior.dataset_id, prior.progress_completed, prior.progress_total
             FROM dataset_generation_attempts current
             JOIN dataset_generation_attempts prior
               ON prior.request_hash = current.request_hash
              AND prior.user_id = current.user_id
              AND prior.status = 'completed'
              AND prior.dataset_id IS NOT NULL
              AND prior.attempt_id != current.attempt_id
             JOIN signal_dataset_access access
               ON access.dataset_id = prior.dataset_id AND access.user_id = current.user_id
             WHERE current.attempt_id = ?1 AND current.status = 'running'
             ORDER BY prior.created_at DESC LIMIT 1",
                [attempt_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(string)?;
        let Some((dataset_id, progress_completed, progress_total)) = reused else {
            return Ok(false);
        };
        self.database
            .execute(
                "UPDATE dataset_generation_attempts
             SET status = 'completed', dataset_id = ?2,
                 progress_completed = ?3, progress_total = ?4
             WHERE attempt_id = ?1 AND status = 'running'",
                params![attempt_id, dataset_id, progress_completed, progress_total],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn record_failure(
        &self,
        attempt_id: &str,
        diagnostic: Diagnostic,
    ) -> Result<(), String> {
        let diagnostic = serde_json::to_string(&diagnostic).map_err(string)?;
        self.database
            .execute(
                "UPDATE dataset_generation_attempts
             SET status = 'failed', diagnostic_json = ?2
             WHERE attempt_id = ?1 AND status = 'running'",
                params![attempt_id, diagnostic],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn set_progress_total(&self, attempt_id: &str, total: i64) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE dataset_generation_attempts SET progress_total = ?2 WHERE attempt_id = ?1",
                params![attempt_id, total],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn set_progress_completed(
        &self,
        attempt_id: &str,
        completed: i64,
    ) -> Result<(), String> {
        self.database.execute(
            "UPDATE dataset_generation_attempts SET progress_completed = ?2 WHERE attempt_id = ?1",
            params![attempt_id, completed],
        ).map(|_| ()).map_err(string)
    }

    pub(crate) fn count_for_user(&self, user_id: &str) -> Result<u64, String> {
        self.database
            .query_row(
                "SELECT COUNT(*) FROM dataset_generation_attempts WHERE user_id = ?1",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as u64)
            .map_err(string)
    }

    pub(crate) fn active_ids_for_user(&self, user_id: &str) -> Result<Vec<String>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id FROM dataset_generation_attempts
             WHERE user_id = ?1 AND status IN ('pending', 'running')",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| row.get(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    pub(crate) fn delete_for_user(&self, user_id: &str) -> Result<(), String> {
        self.database
            .execute(
                "DELETE FROM dataset_generation_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map(|_| ())
            .map_err(string)
    }
}

fn attempt_from_parts(
    attempt_id: String,
    dataset_id: Option<String>,
    status: String,
    diagnostic: Option<String>,
    progress_completed: i64,
    progress_total: i64,
) -> Result<Attempt, String> {
    let diagnostic_evidence = diagnostic.map(|value| {
        serde_json::from_str::<Diagnostic>(&value)
            .map(Diagnostic::evidence)
            .unwrap_or(value)
    });
    Ok(Attempt {
        attempt_id,
        dataset_id,
        status: AttemptStatus::try_from(status.as_str())?,
        diagnostic_evidence,
        progress_completed,
        progress_total,
    })
}

fn string(error: impl ToString) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn database_path(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "adaq-attempt-store-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.join("adaq.db")
    }

    #[test]
    fn creates_current_schema_and_rejects_unknown_status() {
        let path = database_path("status");
        let database = Connection::open(&path).unwrap();
        let store = AttemptStore::new(&database);
        store.initialize().unwrap();
        database
            .execute(
                "INSERT INTO dataset_generation_attempts
             (attempt_id, request_hash, user_id, status, request_json)
             VALUES ('attempt', 'request', 'alice', 'unknown', '{}')",
                [],
            )
            .unwrap();

        let error = store.initialize().unwrap_err();
        assert!(error.contains("unknown Dataset Generation Attempt status"));
        drop(database);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn serializes_the_existing_list_contract_with_lowercase_status() {
        let path = database_path("contract");
        let database = Connection::open(&path).unwrap();
        let store = AttemptStore::new(&database);
        store.initialize().unwrap();
        store
            .prepare("request", "alice", "{}", || "attempt".into())
            .unwrap();

        assert!(store.list("bob").unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(store.list("alice").unwrap().pop().unwrap()).unwrap(),
            serde_json::json!({
                "attemptId": "attempt",
                "datasetId": null,
                "status": "pending",
                "diagnosticEvidence": null,
                "progressCompleted": 0,
                "progressTotal": 0
            })
        );
        drop(database);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn startup_recovery_fails_only_unfinished_attempts_and_retains_evidence() {
        let path = database_path("recovery");
        let database = Connection::open(&path).unwrap();
        let store = AttemptStore::new(&database);
        store.initialize().unwrap();
        let long_diagnostic = "x".repeat(MAX_DIAGNOSTIC_EVIDENCE_CHARS);
        for status in ["pending", "running", "completed", "failed", "cancelled"] {
            let diagnostic = if status == "running" {
                long_diagnostic.as_str()
            } else {
                "retained detail"
            };
            database.execute(
                "INSERT INTO dataset_generation_attempts
                 (attempt_id, request_hash, user_id, dataset_id, status, request_json,
                  diagnostic_json, progress_completed, progress_total)
                 VALUES (?1, 'request', 'alice', 'dataset', ?1,
                         '{\"snapshotId\":\"snapshot\",\"modelArchiveSha256\":\"package\",\"seed\":7}',
                         ?2, 3, 9)",
                params![status, diagnostic],
            ).unwrap();
        }

        store.initialize().unwrap();
        let attempts = store.list("alice").unwrap();
        for attempt in attempts {
            if attempt.attempt_id == "pending" || attempt.attempt_id == "running" {
                assert_eq!(attempt.status, AttemptStatus::Failed);
                let evidence = attempt.diagnostic_evidence.unwrap();
                assert!(evidence.starts_with("generation-interrupted:"));
                if attempt.attempt_id == "running" {
                    assert!(evidence.ends_with(&long_diagnostic));
                } else {
                    assert!(evidence.contains("retained detail"));
                }
            } else {
                let expected = match attempt.attempt_id.as_str() {
                    "completed" => AttemptStatus::Completed,
                    "failed" => AttemptStatus::Failed,
                    "cancelled" => AttemptStatus::Cancelled,
                    _ => unreachable!(),
                };
                assert_eq!(attempt.status, expected);
                assert_eq!(
                    attempt.diagnostic_evidence.as_deref(),
                    Some("retained detail")
                );
            }
            assert_eq!((attempt.progress_completed, attempt.progress_total), (3, 9));
        }
        let request: String = database
            .query_row(
                "SELECT request_json FROM dataset_generation_attempts WHERE attempt_id = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(request.contains("snapshot"));
        assert!(request.contains("package"));
        assert!(request.contains("\"seed\":7"));
        drop(database);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn incompatible_schema_is_not_altered_and_explains_device_reset_scope() {
        let path = database_path("incompatible");
        let database = Connection::open(&path).unwrap();
        database
            .execute_batch(
                "CREATE TABLE dataset_generation_attempts (
                attempt_id TEXT PRIMARY KEY,
                status TEXT NOT NULL
            );",
            )
            .unwrap();
        let before: String = database
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'dataset_generation_attempts'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let error = AttemptStore::new(&database).initialize().unwrap_err();
        assert!(error.contains("Close AdaQ"));
        assert!(error.contains("every User on this device"));
        let after: String = database
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'dataset_generation_attempts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, before);
        drop(database);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
