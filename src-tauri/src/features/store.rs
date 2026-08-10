//! Private SQLite ownership for Feature Definitions, Fitting Protocols,
//! Fitting Attempts, and Fitted Transformation Artifacts. Materialization
//! Attempts and Feature Datasets stay inside the Tauri-independent
//! `FeatureMaterializationStore`; this store never touches their tables.

use rusqlite::{Connection, OptionalExtension, params};

use super::{DefinitionRecord, FeatureAttemptStatus, FittingAttemptRecord, INCOMPATIBLE_SCHEMA};

pub(crate) const FEATURE_WORKSPACE_SCHEMA_VERSION: &str = "1.0.0";

pub(crate) struct FeatureStore<'a> {
    database: &'a Connection,
}

impl<'a> FeatureStore<'a> {
    pub(crate) fn new(database: &'a Connection) -> Self {
        Self { database }
    }

    pub(crate) fn initialize(&self) -> Result<(), String> {
        let schema_exists = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'feature_workspace_schema'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            != 0;
        let tables = [
            "feature_definitions",
            "feature_definition_access",
            "feature_definition_presentation",
            "feature_fitting_protocols",
            "feature_fitting_attempts",
            "feature_fitted_artifacts",
            "feature_artifact_access",
            "feature_artifact_references",
        ];
        if schema_exists {
            let version: Option<String> = self
                .database
                .query_row(
                    "SELECT schema_version FROM feature_workspace_schema LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(string)?;
            if version.as_deref() != Some(FEATURE_WORKSPACE_SCHEMA_VERSION) {
                return Err(INCOMPATIBLE_SCHEMA.into());
            }
            for table in tables {
                if !table_exists(self.database, table)? {
                    return Err(INCOMPATIBLE_SCHEMA.into());
                }
            }
        } else {
            for table in tables {
                if table_exists(self.database, table)? {
                    return Err(INCOMPATIBLE_SCHEMA.into());
                }
            }
        }
        let required_columns = [
            (
                "feature_definitions",
                [
                    "definition_hash",
                    "definition_id",
                    "revision",
                    "definition_json",
                    "created_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_definition_access",
                ["user_id", "definition_hash"].as_slice(),
            ),
            (
                "feature_definition_presentation",
                [
                    "user_id",
                    "definition_hash",
                    "name",
                    "description",
                    "tags_json",
                    "updated_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_fitting_protocols",
                ["protocol_hash", "protocol_json", "created_at_ms"].as_slice(),
            ),
            (
                "feature_fitting_attempts",
                [
                    "queue_sequence",
                    "attempt_id",
                    "user_id",
                    "protocol_hash",
                    "plan_hash",
                    "plan_json",
                    "status",
                    "source_attempt_id",
                    "artifact_id",
                    "failure_code",
                    "diagnostic",
                    "progress_completed",
                    "progress_total",
                    "created_at_ms",
                    "updated_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_fitted_artifacts",
                [
                    "artifact_id",
                    "protocol_hash",
                    "artifact_json",
                    "created_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_artifact_access",
                ["user_id", "artifact_id"].as_slice(),
            ),
            (
                "feature_artifact_references",
                ["artifact_id", "referencing_user_id", "reference_id"].as_slice(),
            ),
        ];
        for (table, columns) in required_columns {
            if table_exists(self.database, table)? {
                let actual = table_columns(self.database, table)?;
                if columns
                    .iter()
                    .any(|column| !actual.contains(&(*column).to_owned()))
                {
                    return Err(INCOMPATIBLE_SCHEMA.into());
                }
            }
        }
        self.database
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS feature_workspace_schema (
                    schema_version TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_definitions (
                    definition_hash TEXT PRIMARY KEY,
                    definition_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    definition_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_definition_access (
                    user_id TEXT NOT NULL,
                    definition_hash TEXT NOT NULL
                        REFERENCES feature_definitions(definition_hash) ON DELETE CASCADE,
                    PRIMARY KEY(user_id, definition_hash)
                 );
                 CREATE TABLE IF NOT EXISTS feature_definition_presentation (
                    user_id TEXT NOT NULL,
                    definition_hash TEXT NOT NULL
                        REFERENCES feature_definitions(definition_hash) ON DELETE CASCADE,
                    name TEXT NOT NULL DEFAULT '',
                    description TEXT NOT NULL DEFAULT '',
                    tags_json TEXT NOT NULL DEFAULT '[]',
                    updated_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(user_id, definition_hash)
                 );
                 CREATE TABLE IF NOT EXISTS feature_fitting_protocols (
                    protocol_hash TEXT PRIMARY KEY,
                    protocol_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_fitting_attempts (
                    queue_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    attempt_id TEXT NOT NULL UNIQUE,
                    user_id TEXT NOT NULL,
                    protocol_hash TEXT NOT NULL
                        REFERENCES feature_fitting_protocols(protocol_hash),
                    plan_hash TEXT NOT NULL,
                    plan_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    source_attempt_id TEXT,
                    artifact_id TEXT,
                    failure_code TEXT,
                    diagnostic TEXT,
                    progress_completed INTEGER NOT NULL DEFAULT 0,
                    progress_total INTEGER NOT NULL DEFAULT 0,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS feature_fitting_attempts_fifo
                    ON feature_fitting_attempts(status, queue_sequence);
                 CREATE TABLE IF NOT EXISTS feature_fitted_artifacts (
                    artifact_id TEXT PRIMARY KEY,
                    protocol_hash TEXT NOT NULL,
                    artifact_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_artifact_access (
                    user_id TEXT NOT NULL,
                    artifact_id TEXT NOT NULL
                        REFERENCES feature_fitted_artifacts(artifact_id) ON DELETE CASCADE,
                    PRIMARY KEY(user_id, artifact_id)
                 );
                 CREATE TABLE IF NOT EXISTS feature_artifact_references (
                    artifact_id TEXT NOT NULL
                        REFERENCES feature_fitted_artifacts(artifact_id) ON DELETE CASCADE,
                    referencing_user_id TEXT NOT NULL,
                    reference_id TEXT NOT NULL,
                    PRIMARY KEY(artifact_id, referencing_user_id, reference_id)
                 );",
            )
            .map_err(string)?;
        if !schema_exists {
            self.database
                .execute(
                    "INSERT INTO feature_workspace_schema(schema_version) VALUES (?1)",
                    [FEATURE_WORKSPACE_SCHEMA_VERSION],
                )
                .map_err(string)?;
        }
        self.recover_interrupted_fitting()
    }

    /// Pending Fitting Attempts survive a restart; stale Running ones become
    /// Failed with interruption evidence.
    fn recover_interrupted_fitting(&self) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE feature_fitting_attempts
                 SET status = 'failed', failure_code = 'interrupted',
                     diagnostic = 'Running fitting was interrupted before completion',
                     updated_at_ms = ?1
                 WHERE status = 'running'",
                [now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    // ---- Definitions ----

    pub(crate) fn publish_definition(
        &self,
        user_id: &str,
        definition_id: &str,
        revision: i64,
        definition_hash: &str,
        definition_json: &str,
    ) -> Result<(), String> {
        let existing: Option<String> = self
            .database
            .query_row(
                "SELECT definition_hash FROM feature_definitions WHERE definition_hash = ?1",
                [definition_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        if existing.is_none() {
            let latest: Option<i64> = self
                .database
                .query_row(
                    "SELECT MAX(revision) FROM feature_definitions WHERE definition_id = ?1",
                    [definition_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .map_err(string)?;
            if latest.is_some_and(|latest| latest >= revision) {
                return Err("Feature Definition revision must increase".into());
            }
            self.database
                .execute(
                    "INSERT INTO feature_definitions(
                         definition_hash, definition_id, revision, definition_json, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        definition_hash,
                        definition_id,
                        revision,
                        definition_json,
                        now_ms()
                    ],
                )
                .map_err(string)?;
        }
        self.database
            .execute(
                "INSERT OR IGNORE INTO feature_definition_access(user_id, definition_hash)
                 VALUES (?1, ?2)",
                params![user_id, definition_hash],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn upsert_presentation(
        &self,
        user_id: &str,
        definition_hash: &str,
        name: &str,
        description: &str,
        tags_json: &str,
    ) -> Result<(), String> {
        let granted = self
            .database
            .execute(
                "INSERT INTO feature_definition_presentation(
                     user_id, definition_hash, name, description, tags_json, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(user_id, definition_hash) DO UPDATE SET
                     name = excluded.name,
                     description = excluded.description,
                     tags_json = excluded.tags_json,
                     updated_at_ms = excluded.updated_at_ms",
                params![
                    user_id,
                    definition_hash,
                    name,
                    description,
                    tags_json,
                    now_ms()
                ],
            )
            .map_err(string)?;
        if granted == 0 {
            return Err("Feature Definition not found".into());
        }
        Ok(())
    }

    pub(crate) fn list_definitions(&self, user_id: &str) -> Result<Vec<DefinitionRecord>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT d.definition_hash, d.definition_id, d.revision, d.definition_json,
                        d.created_at_ms, COALESCE(p.name, ''), COALESCE(p.description, ''),
                        COALESCE(p.tags_json, '[]')
                 FROM feature_definitions d
                 JOIN feature_definition_access a USING(definition_hash)
                 LEFT JOIN feature_definition_presentation p
                   ON p.definition_hash = d.definition_hash AND p.user_id = a.user_id
                 WHERE a.user_id = ?1
                   AND d.revision = (
                       SELECT MAX(d2.revision)
                       FROM feature_definitions d2
                       JOIN feature_definition_access a2
                         ON a2.definition_hash = d2.definition_hash
                       WHERE d2.definition_id = d.definition_id AND a2.user_id = ?1
                   )
                 ORDER BY d.created_at_ms DESC, d.definition_hash",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok(DefinitionRecord {
                    definition_hash: row.get(0)?,
                    definition_id: row.get(1)?,
                    revision: row.get(2)?,
                    definition_json: row.get(3)?,
                    created_at_ms: row.get(4)?,
                    name: row.get(5)?,
                    description: row.get(6)?,
                    tags_json: row.get(7)?,
                })
            })
            .map_err(string)?;
        rows.map(|row| row.map_err(string)).collect()
    }

    pub(crate) fn get_definition(
        &self,
        user_id: &str,
        definition_hash: &str,
    ) -> Result<DefinitionRecord, String> {
        self.database
            .query_row(
                "SELECT d.definition_hash, d.definition_id, d.revision, d.definition_json,
                        d.created_at_ms, COALESCE(p.name, ''), COALESCE(p.description, ''),
                        COALESCE(p.tags_json, '[]')
                 FROM feature_definitions d
                 JOIN feature_definition_access a USING(definition_hash)
                 LEFT JOIN feature_definition_presentation p
                   ON p.definition_hash = d.definition_hash AND p.user_id = a.user_id
                 WHERE a.user_id = ?1 AND d.definition_hash = ?2",
                params![user_id, definition_hash],
                |row| {
                    Ok(DefinitionRecord {
                        definition_hash: row.get(0)?,
                        definition_id: row.get(1)?,
                        revision: row.get(2)?,
                        definition_json: row.get(3)?,
                        created_at_ms: row.get(4)?,
                        name: row.get(5)?,
                        description: row.get(6)?,
                        tags_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(string)?
            .ok_or_else(|| "Feature Definition not found".into())
    }

    // ---- Fitting Protocols and Attempts ----

    pub(crate) fn upsert_protocol(
        &self,
        protocol_hash: &str,
        protocol_json: &str,
    ) -> Result<(), String> {
        let stored: Option<String> = self
            .database
            .query_row(
                "SELECT protocol_json FROM feature_fitting_protocols WHERE protocol_hash = ?1",
                [protocol_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        match stored {
            Some(existing) if existing != protocol_json => {
                Err("fitted-protocol-content-collision".into())
            }
            Some(_) => Ok(()),
            None => self
                .database
                .execute(
                    "INSERT INTO feature_fitting_protocols(protocol_hash, protocol_json, created_at_ms)
                     VALUES (?1, ?2, ?3)",
                    params![protocol_hash, protocol_json, now_ms()],
                )
                .map(|_| ())
                .map_err(string),
        }
    }

    /// Coalesces an exact effective Protocol: one Pending or Running Attempt,
    /// or one Completed Attempt whose Artifact the User still holds, is
    /// returned instead of creating redundant work.
    pub(crate) fn prepare_fitting(
        &self,
        user_id: &str,
        protocol_hash: &str,
        plan_hash: &str,
        plan_json: &str,
        new_attempt_id: impl FnOnce() -> String,
    ) -> Result<(FittingAttemptRecord, bool), String> {
        if let Some(existing) = self.reusable_fitting_attempt(user_id, protocol_hash)? {
            return Ok((existing, false));
        }
        let attempt = insert_fitting_attempt(
            self.database,
            &new_attempt_id(),
            user_id,
            protocol_hash,
            plan_hash,
            plan_json,
            None,
        )?;
        Ok((attempt, true))
    }

    pub(crate) fn prepare_fitting_retry(
        &self,
        user_id: &str,
        attempt_id: &str,
        new_attempt_id: impl FnOnce() -> String,
    ) -> Result<FittingAttemptRecord, String> {
        let previous = self.fitting_attempt(user_id, attempt_id)?;
        if !matches!(
            previous.status,
            FeatureAttemptStatus::Failed | FeatureAttemptStatus::Cancelled
        ) {
            return Err("Feature Fitting Attempt cannot be retried".into());
        }
        if let Some(active) = self.reusable_fitting_attempt(user_id, &previous.protocol_hash)? {
            return Ok(active);
        }
        insert_fitting_attempt(
            self.database,
            &new_attempt_id(),
            user_id,
            &previous.protocol_hash,
            &previous.plan_hash,
            &previous.plan_json,
            Some(attempt_id),
        )
    }

    fn reusable_fitting_attempt(
        &self,
        user_id: &str,
        protocol_hash: &str,
    ) -> Result<Option<FittingAttemptRecord>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id, user_id, protocol_hash, plan_hash, plan_json, status,
                        source_attempt_id, artifact_id, failure_code, diagnostic,
                        progress_completed, progress_total, created_at_ms, updated_at_ms
                 FROM feature_fitting_attempts
                 WHERE user_id = ?1 AND protocol_hash = ?2
                   AND (status IN ('pending', 'running')
                        OR (status = 'completed' AND artifact_id IS NOT NULL
                            AND EXISTS (
                                SELECT 1 FROM feature_artifact_access access
                                WHERE access.user_id = ?1
                                  AND access.artifact_id = feature_fitting_attempts.artifact_id
                            )))
                 ORDER BY queue_sequence",
            )
            .map_err(string)?;
        let rows = statement
            .query_map(params![user_id, protocol_hash], row_to_fitting_attempt)
            .map_err(string)?;
        rows.map(|row| row.map_err(string)).next().transpose()
    }

    pub(crate) fn fitting_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<FittingAttemptRecord, String> {
        self.database
            .query_row(
                "SELECT attempt_id, user_id, protocol_hash, plan_hash, plan_json, status,
                        source_attempt_id, artifact_id, failure_code, diagnostic,
                        progress_completed, progress_total, created_at_ms, updated_at_ms
                 FROM feature_fitting_attempts
                 WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                row_to_fitting_attempt,
            )
            .optional()
            .map_err(string)?
            .ok_or_else(|| "Feature Fitting Attempt not found".into())
    }

    pub(crate) fn fitting_attempts(
        &self,
        user_id: &str,
    ) -> Result<Vec<FittingAttemptRecord>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id, user_id, protocol_hash, plan_hash, plan_json, status,
                        source_attempt_id, artifact_id, failure_code, diagnostic,
                        progress_completed, progress_total, created_at_ms, updated_at_ms
                 FROM feature_fitting_attempts
                 WHERE user_id = ?1
                 ORDER BY queue_sequence",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([user_id], row_to_fitting_attempt)
            .map_err(string)?;
        rows.map(|row| row.map_err(string)).collect()
    }

    /// The oldest Pending Fitting Attempt with its Protocol evidence, in
    /// persistent FIFO order.
    pub(crate) fn next_pending_fitting(
        &self,
    ) -> Result<Option<(FittingAttemptRecord, String)>, String> {
        self.database
            .query_row(
                "SELECT a.attempt_id, a.user_id, a.protocol_hash, a.plan_hash, a.plan_json,
                        a.status, a.source_attempt_id, a.artifact_id, a.failure_code,
                        a.diagnostic, a.progress_completed, a.progress_total,
                        a.created_at_ms, a.updated_at_ms, p.protocol_json
                 FROM feature_fitting_attempts a
                 JOIN feature_fitting_protocols p USING(protocol_hash)
                 WHERE a.status = 'pending'
                 ORDER BY a.queue_sequence LIMIT 1",
                [],
                |row| Ok((row_to_fitting_attempt(row)?, row.get::<_, String>(14)?)),
            )
            .optional()
            .map_err(string)
    }

    pub(crate) fn mark_fitting_running(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE feature_fitting_attempts SET status = 'running', updated_at_ms = ?3
                 WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'pending'",
                params![attempt_id, user_id, now_ms()],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn set_fitting_progress(
        &self,
        user_id: &str,
        attempt_id: &str,
        completed: i64,
        total: i64,
    ) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE feature_fitting_attempts
                 SET progress_completed = ?3, progress_total = ?4, updated_at_ms = ?5
                 WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'running'
                   AND ?3 >= progress_completed",
                params![attempt_id, user_id, completed, total, now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn cancel_fitting(
        &self,
        user_id: &str,
        attempt_id: &str,
        statuses: &[&str],
    ) -> Result<bool, String> {
        let mut sql = String::from(
            "UPDATE feature_fitting_attempts
             SET status = 'cancelled', failure_code = 'cancelled',
                 diagnostic = 'Fitting cancelled', updated_at_ms = ?3
             WHERE attempt_id = ?1 AND user_id = ?2 AND status IN (",
        );
        let placeholders = (0..statuses.len())
            .map(|offset| format!("?{}", 4 + offset))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&placeholders);
        sql.push(')');
        let mut statement = self.database.prepare(&sql).map_err(string)?;
        statement
            .raw_bind_parameter(1, attempt_id.to_owned())
            .map_err(string)?;
        statement
            .raw_bind_parameter(2, user_id.to_owned())
            .map_err(string)?;
        statement.raw_bind_parameter(3, now_ms()).map_err(string)?;
        let mut index = 4;
        for status in statuses {
            statement
                .raw_bind_parameter(index, (*status).to_owned())
                .map_err(string)?;
            index += 1;
        }
        statement
            .raw_execute()
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn fail_fitting(
        &self,
        user_id: &str,
        attempt_id: &str,
        failure_code: &str,
        diagnostic: &str,
    ) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE feature_fitting_attempts
                 SET status = 'failed', failure_code = ?3, diagnostic = ?4, updated_at_ms = ?5
                 WHERE attempt_id = ?1 AND user_id = ?2 AND status IN ('pending', 'running')",
                params![attempt_id, user_id, failure_code, diagnostic, now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn complete_fitting(
        &self,
        user_id: &str,
        attempt_id: &str,
        artifact_id: &str,
    ) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE feature_fitting_attempts
                 SET status = 'completed', artifact_id = ?3,
                     failure_code = NULL, diagnostic = NULL, updated_at_ms = ?4
                 WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'running'",
                params![attempt_id, user_id, artifact_id, now_ms()],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    pub(crate) fn active_fitting_ids_for_user(&self, user_id: &str) -> Result<Vec<String>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id FROM feature_fitting_attempts
                 WHERE user_id = ?1 AND status IN ('pending', 'running')",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| row.get(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    // ---- Artifacts ----

    pub(crate) fn publish_artifact(
        &self,
        user_id: &str,
        artifact_id: &str,
        protocol_hash: &str,
        artifact_json: &str,
    ) -> Result<(), String> {
        let stored: Option<String> = self
            .database
            .query_row(
                "SELECT artifact_json FROM feature_fitted_artifacts WHERE artifact_id = ?1",
                [artifact_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        match stored {
            Some(existing) if existing != artifact_json => {
                return Err("fitted-artifact-id-collision".into());
            }
            Some(_) => {}
            None => {
                self.database
                    .execute(
                        "INSERT INTO feature_fitted_artifacts(
                             artifact_id, protocol_hash, artifact_json, created_at_ms
                         ) VALUES (?1, ?2, ?3, ?4)",
                        params![artifact_id, protocol_hash, artifact_json, now_ms()],
                    )
                    .map_err(string)?;
            }
        }
        self.database
            .execute(
                "INSERT OR IGNORE INTO feature_artifact_access(user_id, artifact_id)
                 VALUES (?1, ?2)",
                params![user_id, artifact_id],
            )
            .map(|_| ())
            .map_err(string)
    }

    pub(crate) fn artifact_for_user(
        &self,
        user_id: &str,
        artifact_id: &str,
    ) -> Result<super::ArtifactRecord, String> {
        self.database
            .query_row(
                "SELECT f.artifact_id, f.protocol_hash, f.artifact_json, f.created_at_ms
                 FROM feature_fitted_artifacts f
                 JOIN feature_artifact_access a USING(artifact_id)
                 WHERE a.user_id = ?1 AND f.artifact_id = ?2",
                params![user_id, artifact_id],
                |row| {
                    Ok(super::ArtifactRecord {
                        artifact_id: row.get(0)?,
                        protocol_hash: row.get(1)?,
                        artifact_json: row.get(2)?,
                        created_at_ms: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(string)?
            .ok_or_else(|| "Fitted Transformation Artifact not found".into())
    }

    pub(crate) fn artifacts_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<super::ArtifactRecord>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT f.artifact_id, f.protocol_hash, f.artifact_json, f.created_at_ms
                 FROM feature_fitted_artifacts f
                 JOIN feature_artifact_access a USING(artifact_id)
                 WHERE a.user_id = ?1
                 ORDER BY f.created_at_ms DESC, f.artifact_id",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok(super::ArtifactRecord {
                    artifact_id: row.get(0)?,
                    protocol_hash: row.get(1)?,
                    artifact_json: row.get(2)?,
                    created_at_ms: row.get(3)?,
                })
            })
            .map_err(string)?;
        rows.map(|row| row.map_err(string)).collect()
    }

    /// Deletion is locked while any downstream local research owner keeps a
    /// narrow typed reference to the Artifact.
    pub(crate) fn delete_artifact(&self, user_id: &str, artifact_id: &str) -> Result<(), String> {
        self.artifact_for_user(user_id, artifact_id)?;
        let referenced: i64 = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM feature_artifact_references WHERE artifact_id = ?1",
                [artifact_id],
                |row| row.get(0),
            )
            .map_err(string)?;
        if referenced != 0 {
            return Err("artifact-referenced".into());
        }
        self.database
            .execute(
                "DELETE FROM feature_artifact_access WHERE user_id = ?1 AND artifact_id = ?2",
                params![user_id, artifact_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_fitted_artifacts
                 WHERE artifact_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM feature_artifact_access a WHERE a.artifact_id = ?1
                   )",
                [artifact_id],
            )
            .map(|_| ())
            .map_err(string)
    }

    // ---- Reset ----

    pub(crate) fn delete_for_user(&self, user_id: &str) -> Result<(), String> {
        self.database
            .execute(
                "DELETE FROM feature_artifact_references WHERE referencing_user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_fitting_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_artifact_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_fitted_artifacts
                 WHERE NOT EXISTS (
                     SELECT 1 FROM feature_artifact_access a
                     WHERE a.artifact_id = feature_fitted_artifacts.artifact_id
                 )",
                [],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_definition_presentation WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_definition_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        self.database
            .execute(
                "DELETE FROM feature_definitions
                 WHERE NOT EXISTS (
                     SELECT 1 FROM feature_definition_access a
                     WHERE a.definition_hash = feature_definitions.definition_hash
                 )",
                [],
            )
            .map(|_| ())
            .map_err(string)
    }
}

fn insert_fitting_attempt(
    database: &Connection,
    attempt_id: &str,
    user_id: &str,
    protocol_hash: &str,
    plan_hash: &str,
    plan_json: &str,
    source_attempt_id: Option<&str>,
) -> Result<FittingAttemptRecord, String> {
    let now = now_ms();
    database
        .execute(
            "INSERT INTO feature_fitting_attempts(
                 attempt_id, user_id, protocol_hash, plan_hash, plan_json, status,
                 source_attempt_id, progress_completed, progress_total,
                 created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, 0, 0, ?7, ?7)",
            params![
                attempt_id,
                user_id,
                protocol_hash,
                plan_hash,
                plan_json,
                source_attempt_id,
                now
            ],
        )
        .map_err(string)?;
    Ok(FittingAttemptRecord {
        attempt_id: attempt_id.into(),
        user_id: user_id.into(),
        protocol_hash: protocol_hash.into(),
        plan_hash: plan_hash.into(),
        plan_json: plan_json.into(),
        status: FeatureAttemptStatus::Pending,
        source_attempt_id: source_attempt_id.map(str::to_owned),
        artifact_id: None,
        failure_code: None,
        diagnostic: None,
        progress_completed: 0,
        progress_total: 0,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn row_to_fitting_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<FittingAttemptRecord> {
    let status: String = row.get(5)?;
    Ok(FittingAttemptRecord {
        attempt_id: row.get(0)?,
        user_id: row.get(1)?,
        protocol_hash: row.get(2)?,
        plan_hash: row.get(3)?,
        plan_json: row.get(4)?,
        status: FeatureAttemptStatus::try_from(status.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(error)),
            )
        })?,
        source_attempt_id: row.get(6)?,
        artifact_id: row.get(7)?,
        failure_code: row.get(8)?,
        diagnostic: row.get(9)?,
        progress_completed: row.get(10)?,
        progress_total: row.get(11)?,
        created_at_ms: row.get(12)?,
        updated_at_ms: row.get(13)?,
    })
}

fn table_exists(database: &Connection, table: &str) -> Result<bool, String> {
    database
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count != 0)
        .map_err(string)
}

fn table_columns(database: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut statement = database
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(string)?;
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn string(error: impl ToString) -> String {
    error.to_string()
}
