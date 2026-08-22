//! Component Library module.
//!
//! One deep, Tauri-independent module owning the Component Package
//! lifecycle: import, listing, paging, imported-check, compatibility
//! lookups, and deletion. The module owns both Component tables
//! (`component_content` and `component_access`), their DDL and every
//! entitlement SQL statement, and the archive-file staging and orphan
//! cleanup the reset flows consume. Run locks flow through the narrow
//! component-lock queries the Backtest Run module exposes, so this domain
//! never issues SQL over the Run or bridge tables; the immutable Signal
//! Dataset lock check reads the forecast_signal_dataset-owned metadata the way the Validation
//! Report reference check reads its own domain.

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, MutexGuard},
};

use adaq_component_tooling::{
    ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage, FeatureSlotDefinition,
    ModelArtifact, ModelOutput, ModelScope, ParameterDefinition, QualificationAttempt,
    StrategyArchitecture, qualify_package, strategy_architecture, verify_package,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::user::validate_user;

const COMPONENT_PAGE_SIZE: usize = 10;

/// The narrow cross-domain reads the Component Library consumes from the
/// Backtest Run module: the component-lock query covering deletion-lock
/// and listing-lock needs, and the Run-locked hash set the reset flows'
/// orphan guard needs. Implemented by the composition root over the
/// Backtest Run module; the complete Local Research state is never passed
/// in, and this module never issues SQL over the Run tables itself.
pub(crate) trait ComponentLockSource {
    fn runs_locking_components(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<HashMap<String, Vec<String>>, String>;
    fn component_hashes_locked_by_runs(
        &self,
        database: &Connection,
        excluding_user: Option<&str>,
    ) -> Result<HashSet<String>, String>;
}

/// The concrete local dependencies composed into the Component Library.
/// Only database access, the archive directory, and the Backtest Run
/// module's component-lock queries are shared; the complete Local
/// Research state is not.
pub(crate) trait ComponentSource: Send + Sync {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn archive_directory(&self) -> Result<PathBuf, String>;
    fn locks(&self) -> &dyn ComponentLockSource;
}

/// The Component Package evidence counts and footprint the Local Data
/// summary reports for one User.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ComponentSummary {
    pub component_count: u64,
    pub model_artifact_count: u64,
    pub component_bytes: u64,
    pub component_blocking_run_count: u64,
}

/// The Component Library interface: import, listing, paging,
/// imported-check, compatibility lookups, deletion, and entitlement-scoped
/// Package reads, plus the summary-for-user and reset hooks the
/// composition root calls.
#[derive(Clone)]
pub(crate) struct ComponentLibrary(Arc<dyn ComponentSource>);

impl ComponentLibrary {
    /// Creates the module and initializes the Component Package schema,
    /// which lives inside this module.
    pub(crate) fn open(source: Arc<dyn ComponentSource>) -> Result<Self, String> {
        source
            .database()?
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS component_content (
                archive_sha256 TEXT PRIMARY KEY,
                component_id TEXT NOT NULL,
                version TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                wasm_sha256 TEXT NOT NULL,
                archive_path TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '',
                UNIQUE(component_id, version)
             );
             CREATE TABLE IF NOT EXISTS component_access (
                user_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                PRIMARY KEY(user_id, archive_sha256),
                FOREIGN KEY(archive_sha256) REFERENCES component_content(archive_sha256)
             );
             CREATE TABLE IF NOT EXISTS component_qualification_attempts (
                attempt_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                qualified INTEGER NOT NULL,
                evidence_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
             );",
            )
            .map_err(string)?;
        Ok(Self(source))
    }

    /// Imports one Component Package for one User, rejecting a reused
    /// identity whose archive or wasm no longer matches.
    pub(crate) fn import(&self, user_id: &str, bytes: &[u8]) -> Result<LibraryComponent, String> {
        validate_user(user_id)?;
        let qualification = self.qualify(user_id, bytes)?;
        if !qualification.qualified {
            return Err("Component Package failed qualification".into());
        }
        let package = ComponentPackage::read(bytes).map_err(string)?;
        verify_package(&package)?;
        let component_id = package.manifest.component_id.to_string();
        let version = package.manifest.version.to_string();
        let sdk_version = package.manifest.sdk_version.to_string();
        let kind = format!("{:?}", package.manifest.kind).to_lowercase();
        let mut database = self.0.database()?;
        let existing: Option<(String, String)> = database.query_row(
            "SELECT archive_sha256, wasm_sha256 FROM component_content WHERE component_id = ?1 AND version = ?2",
            params![component_id, version], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(string)?;
        if existing.as_ref().is_some_and(|(archive, wasm)| {
            archive != &package.archive_sha256 || wasm != &package.manifest.wasm_sha256
        }) {
            return Err("A different Component already uses this identity and version".into());
        }
        let path = self
            .0
            .archive_directory()?
            .join(format!("{}.adaq", package.archive_sha256));
        if !path.is_file() {
            fs::write(&path, bytes).map_err(string)?;
        }
        let transaction = database.transaction().map_err(string)?;
        let component = LibraryComponent {
            component_id,
            version,
            manifest_schema_version: package.manifest.manifest_schema_version.to_string(),
            sdk_version,
            abi_version: package.manifest.abi_version.to_string(),
            architecture: strategy_architecture(&package.manifest),
            name: package.manifest.name,
            kind,
            archive_sha256: package.archive_sha256,
            wasm_sha256: package.manifest.wasm_sha256,
            parameters: package.manifest.parameters,
            feature_slots: package.manifest.feature_slots,
            output_names: package.manifest.output_names,
            dependencies: package.manifest.dependencies,
            warmup_bars: package.manifest.warmup_bars,
            model_scope: package.manifest.model_scope,
            model_outputs: package.manifest.model_outputs,
            model_artifact: package.manifest.model_artifact,
            compatible: true,
            compatibility_error: None,
            locked_by_run_ids: vec![],
        };
        let metadata_json = serde_json::to_string(&component).map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_content
             (archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    component.archive_sha256,
                    component.component_id,
                    component.version,
                    component.name,
                    component.kind,
                    component.wasm_sha256,
                    path.to_string_lossy(),
                    metadata_json,
                ],
            )
            .map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_access(user_id, archive_sha256) VALUES (?1, ?2)",
                params![user_id, component.archive_sha256],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)?;
        Ok(component)
    }

    pub(crate) fn qualify(
        &self,
        user_id: &str,
        bytes: &[u8],
    ) -> Result<QualificationAttempt, String> {
        validate_user(user_id)?;
        // Imported archives do not carry a second source runtime. The tooling
        // conformance replay is therefore the package's executable equivalence
        // witness at this trust boundary.
        let attempt = qualify_package(uuid::Uuid::new_v4().to_string(), bytes, |package, _| {
            verify_package(package)
        });
        let database = self.0.database()?;
        database
            .execute(
                "INSERT INTO component_qualification_attempts
                 (attempt_id, user_id, archive_sha256, qualified, evidence_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    attempt.attempt_id,
                    user_id,
                    attempt.archive_sha256,
                    attempt.qualified,
                    serde_json::to_string(&attempt.evidence).map_err(string)?,
                    crate::unix_now_ms(),
                ],
            )
            .map_err(string)?;
        Ok(attempt)
    }

    /// Lists every Component Package one User is entitled to, with Run
    /// locks observed through the Backtest Run module.
    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<LibraryComponent>, String> {
        self.list_range(user_id, -1, 0)
    }

    /// Pages one User's Component Library ten Packages at a time.
    pub(crate) fn page(&self, user_id: &str, page: usize) -> Result<ComponentPage, String> {
        validate_user(user_id)?;
        if page == 0 {
            return Err("Component Package page is invalid".into());
        }
        let total = self
            .0
            .database()?
            .query_row(
                "SELECT COUNT(*) FROM component_content c
                 JOIN component_access a USING(archive_sha256)
                 WHERE a.user_id = ?1",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Component Package count is invalid")?;
        let offset = page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(COMPONENT_PAGE_SIZE))
            .ok_or_else(|| "Component Package page is too large".to_owned())?;
        Ok(ComponentPage {
            items: self.list_range(user_id, COMPONENT_PAGE_SIZE as i64, offset as i64)?,
            total,
            page,
            page_size: COMPONENT_PAGE_SIZE,
        })
    }

    /// Whether one User has already imported one Component Package.
    pub(crate) fn is_imported(&self, user_id: &str, hash: &str) -> Result<bool, String> {
        validate_user(user_id)?;
        self.0
            .database()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2)",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(string)
    }

    /// Deletes one Component Package for one User unless an immutable
    /// Backtest Run or Signal Dataset still locks it; the archive file is
    /// removed once no User can read it anymore.
    pub(crate) fn delete(&self, user_id: &str, hash: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let mut database = self.0.database()?;
        let entitled: bool = database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2)",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        if !entitled {
            return Err("Component Package is not available to this User".into());
        }
        // Query the Backtest module on the held connection so the locks
        // observed here match the rows the deletion removes.
        let locked_by_hash = self.0.locks().runs_locking_components(&database, user_id)?;
        let locked_by_run_ids = locked_by_hash.get(hash).cloned().unwrap_or_default();
        if !locked_by_run_ids.is_empty() {
            let noun = if locked_by_run_ids.len() == 1 {
                "Backtest Run"
            } else {
                "Backtest Runs"
            };
            return Err(format!(
                "Component Package is locked by {noun}: {}",
                locked_by_run_ids.join(", ")
            ));
        }
        let locked_by_dataset_ids = database
            .prepare(
                "SELECT c.dataset_id FROM signal_dataset_content c
                 JOIN signal_dataset_access a USING(dataset_id)
                 WHERE a.user_id = ?1 AND (
                    json_extract(c.metadata_json, '$.modelArchiveSha256') = ?2
                    OR EXISTS(SELECT 1 FROM json_each(c.metadata_json, '$.componentLock') WHERE json_extract(value, '$.archiveSha256') = ?2)
                    OR EXISTS(SELECT 1 FROM json_each(c.metadata_json, '$.externalProducerSegments') WHERE json_extract(value, '$.modelArtifact.sha256') = ?2)
                 )
                 ORDER BY c.dataset_id",
            )
            .map_err(string)?
            .query_map(params![user_id, hash], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        if !locked_by_dataset_ids.is_empty() {
            return Err(format!(
                "Component Package is locked by immutable Signal Dataset(s): {}",
                locked_by_dataset_ids.join(", ")
            ));
        }
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2",
                params![user_id, hash],
            )
            .map_err(string)?;
        let remaining: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM component_access WHERE archive_sha256 = ?1",
                [hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        let path = if remaining == 0 {
            transaction
                .query_row(
                    "SELECT archive_path FROM component_content WHERE archive_sha256 = ?1",
                    [hash],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(string)?
        } else {
            None
        };
        if remaining == 0 {
            transaction
                .execute(
                    "DELETE FROM component_content WHERE archive_sha256 = ?1",
                    [hash],
                )
                .map_err(string)?;
        }
        transaction.commit().map_err(string)?;
        if let Some(path) = path {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    /// The compatible Factor archive hashes for each dependency alias of
    /// one User's Strategy or Model Component.
    pub(crate) fn compatible_factors(
        &self,
        user_id: &str,
        consumer_archive_sha256: &str,
    ) -> Result<BTreeMap<String, Vec<String>>, String> {
        let consumer = self.package_for_user(user_id, consumer_archive_sha256)?;
        if !matches!(
            consumer.manifest.kind,
            ComponentKind::Strategy | ComponentKind::Model
        ) {
            return Err("Compatible Factors require a Strategy or Model Component".into());
        }
        let components = self.list(user_id)?;
        let packages = components
            .iter()
            .filter(|component| component.kind == "factor" && component.compatible)
            .map(|component| {
                self.package_for_user(user_id, &component.archive_sha256)
                    .map(|package| (component, package))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(compatible_factor_hashes(&consumer.manifest, &packages))
    }

    /// Reads one entitled Component Package from disk, verifying it still
    /// matches its stored identity and hashes.
    pub(crate) fn package_for_user(
        &self,
        user_id: &str,
        hash: &str,
    ) -> Result<ComponentPackage, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let (path, archive_sha256, wasm_sha256): (String, String, String) = database
            .query_row(
                "SELECT c.archive_path, c.archive_sha256, c.wasm_sha256 FROM component_content c
                 JOIN component_access a USING(archive_sha256)
                 WHERE a.user_id = ?1 AND c.archive_sha256 = ?2",
                params![user_id, hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Component Package is not available to this User".to_owned())?;
        drop(database);
        let package = ComponentPackage::read(&fs::read(path).map_err(string)?).map_err(string)?;
        verify_package(&package)?;
        if package.archive_sha256 != archive_sha256 || package.manifest.wasm_sha256 != wasm_sha256 {
            return Err("Component Package does not match stored identity or hashes".into());
        }
        Ok(package)
    }

    /// The summary hook the composition root calls: the Component Package
    /// counts, footprint, and Run-blocking count for one User.
    pub(crate) fn summary_for_user(&self, user_id: &str) -> Result<ComponentSummary, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let count = |sql: &str| -> Result<u64, String> {
            database
                .query_row(sql, [user_id], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(string)
        };
        let component_paths = strings(
            &database,
            "SELECT c.archive_path FROM component_content c
             JOIN component_access a USING(archive_sha256) WHERE a.user_id = ?1",
            user_id,
        )?;
        let owned_components = owned_component_hashes(&database, user_id)?;
        let locking_runs = self.0.locks().runs_locking_components(&database, user_id)?;
        Ok(ComponentSummary {
            component_count: count("SELECT COUNT(*) FROM component_access WHERE user_id = ?1")?,
            model_artifact_count: count(
                "SELECT COUNT(*) FROM component_content c JOIN component_access a USING(archive_sha256) WHERE a.user_id = ?1 AND c.kind = 'model'",
            )?,
            component_bytes: component_paths.iter().map(file_bytes).sum(),
            component_blocking_run_count: count_runs_locking_owned_components(
                &locking_runs,
                &owned_components,
            ),
        })
    }

    /// The Component reset hook the composition root calls under the held
    /// database lock: refuses to break any Run or Signal Dataset lock,
    /// then stages the orphaned archives, drops one User's entitlements,
    /// prunes the orphaned content, and finishes the staged files.
    pub(crate) fn reset_for_user(
        &self,
        database: &mut Connection,
        user_id: &str,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        let owned_components = owned_component_hashes(database, user_id)?;
        let locking_runs = self.0.locks().runs_locking_components(database, user_id)?;
        let blocking_runs =
            count_runs_locking_owned_components(&locking_runs, &owned_components) as i64;
        let blocking_datasets: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .map_err(string)?;
        let blocking = blocking_runs + blocking_datasets;
        if blocking > 0 {
            return Err(format!(
                "Component Package reset is blocked by {blocking} immutable Backtest Run(s)"
            ));
        }
        let paths = self.orphan_archive_paths(database, user_id, None)?;
        let archive_directory = self.0.archive_directory()?;
        let staged = stage_files(paths, &archive_directory)?;
        let result = (|| {
            let transaction = database.transaction().map_err(string)?;
            transaction
                .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
                .map_err(string)?;
            self.delete_orphan_content(&transaction, None)?;
            transaction.commit().map_err(string)
        })();
        finish_staged_files(staged, result)
    }

    /// The Reset All hook: drops one User's Component entitlements inside
    /// the composition root's reset transaction. The orphan-content prune
    /// happens later in the same transaction through
    /// [`Self::delete_orphan_content`], mirroring how the other modules'
    /// reset hooks own their table deletes.
    pub(crate) fn reset_access_for_user(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
    ) -> Result<(), String> {
        transaction
            .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        Ok(())
    }

    /// The archive paths of one User's exclusively-owned Component content
    /// that the Backtest module does not report as Run-locked, the set the
    /// reset flows stage before deleting rows. `excluding_user` drops one
    /// User's Runs from the guard; the Reset All flow passes the reset
    /// User because those Runs are deleted in the same transaction.
    pub(crate) fn orphan_archive_paths(
        &self,
        database: &Connection,
        user_id: &str,
        excluding_user: Option<&str>,
    ) -> Result<Vec<PathBuf>, String> {
        let locked_by_runs = self
            .0
            .locks()
            .component_hashes_locked_by_runs(database, excluding_user)?;
        Ok(orphan_component_candidates(database, user_id)?
            .into_iter()
            .filter(|(hash, _)| !locked_by_runs.contains(hash))
            .map(|(_, path)| PathBuf::from(path))
            .collect())
    }

    /// Deletes Component content rows nobody can read anymore, skipping
    /// the hashes the Backtest module reports as still locked by Runs.
    /// `excluding_user` mirrors [`Self::orphan_archive_paths`].
    pub(crate) fn delete_orphan_content(
        &self,
        transaction: &Transaction<'_>,
        excluding_user: Option<&str>,
    ) -> Result<(), String> {
        let locked_by_runs = self
            .0
            .locks()
            .component_hashes_locked_by_runs(transaction, excluding_user)?;
        let mut statement = transaction
            .prepare(
                "SELECT archive_sha256 FROM component_content
                 WHERE NOT EXISTS(SELECT 1 FROM component_access a
                     WHERE a.archive_sha256 = component_content.archive_sha256)",
            )
            .map_err(string)?;
        let orphans = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        for hash in orphans
            .iter()
            .filter(|hash| !locked_by_runs.contains(*hash))
        {
            transaction
                .execute(
                    "DELETE FROM component_content WHERE archive_sha256 = ?1",
                    [hash],
                )
                .map_err(string)?;
        }
        Ok(())
    }

    fn list_range(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LibraryComponent>, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        // Query the Backtest module on the held connection so the locks
        // observed here match the rows the listing reads.
        let locked_by_hash = self.0.locks().runs_locking_components(&database, user_id)?;
        let mut statement = database
            .prepare(
                "SELECT c.component_id, c.version, c.name, c.kind, c.archive_sha256, c.wasm_sha256, c.archive_path, c.metadata_json
             FROM component_content c JOIN component_access a USING(archive_sha256)
             WHERE a.user_id = ?1 ORDER BY c.name, c.version, c.archive_sha256
             LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        statement
            .query_map(params![user_id, limit, offset], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?
            .into_iter()
            .map(
                |(
                    component_id,
                    version,
                    name,
                    kind,
                    archive_sha256,
                    wasm_sha256,
                    path,
                    metadata_json,
                )| {
                    if !metadata_json.is_empty() {
                        return serde_json::from_str::<LibraryComponent>(&metadata_json)
                            .map(|mut component| {
                                component.component_id = component_id;
                                component.version = version;
                                component.name = name;
                                component.kind = kind;
                                component.archive_sha256 = archive_sha256.clone();
                                component.wasm_sha256 = wasm_sha256;
                                component.locked_by_run_ids = locked_by_hash
                                    .get(&archive_sha256)
                                    .cloned()
                                    .unwrap_or_default();
                                component
                            })
                            .map_err(string);
                    }
                    match fs::read(path)
                        .map_err(string)
                        .and_then(|bytes| ComponentPackage::read(&bytes).map_err(string))
                        .and_then(|package| {
                            verify_package(&package)?;
                            let package_kind =
                                format!("{:?}", package.manifest.kind).to_lowercase();
                            if package.archive_sha256 != archive_sha256
                                || package.manifest.component_id.to_string() != component_id
                                || package.manifest.version.to_string() != version
                                || package.manifest.name != name
                                || package_kind != kind
                                || package.manifest.wasm_sha256 != wasm_sha256
                            {
                                return Err(
                                    "Component Package does not match stored identity or hashes"
                                        .into(),
                                );
                            }
                            Ok(package)
                        }) {
                        Ok(package) => Ok(LibraryComponent {
                            component_id,
                            version,
                            manifest_schema_version: package
                                .manifest
                                .manifest_schema_version
                                .to_string(),
                            sdk_version: package.manifest.sdk_version.to_string(),
                            abi_version: package.manifest.abi_version.to_string(),
                            name,
                            kind,
                            locked_by_run_ids: locked_by_hash
                                .get(&archive_sha256)
                                .cloned()
                                .unwrap_or_default(),
                            archive_sha256,
                            wasm_sha256,
                            architecture: strategy_architecture(&package.manifest),
                            parameters: package.manifest.parameters,
                            feature_slots: package.manifest.feature_slots,
                            output_names: package.manifest.output_names,
                            dependencies: package.manifest.dependencies,
                            warmup_bars: package.manifest.warmup_bars,
                            model_scope: package.manifest.model_scope,
                            model_outputs: package.manifest.model_outputs,
                            model_artifact: package.manifest.model_artifact,
                            compatible: true,
                            compatibility_error: None,
                        }),
                        Err(error) => Ok(LibraryComponent {
                            component_id,
                            version,
                            manifest_schema_version: String::new(),
                            sdk_version: String::new(),
                            abi_version: String::new(),
                            name,
                            kind,
                            locked_by_run_ids: locked_by_hash
                                .get(&archive_sha256)
                                .cloned()
                                .unwrap_or_default(),
                            archive_sha256,
                            wasm_sha256,
                            parameters: vec![],
                            feature_slots: vec![],
                            output_names: vec![],
                            dependencies: vec![],
                            warmup_bars: 0,
                            model_scope: None,
                            model_outputs: vec![],
                            model_artifact: None,
                            architecture: None,
                            compatible: false,
                            compatibility_error: Some(format!(
                                "Incompatible Component Package: {error}"
                            )),
                        }),
                    }
                },
            )
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryComponent {
    component_id: String,
    version: String,
    manifest_schema_version: String,
    sdk_version: String,
    abi_version: String,
    name: String,
    kind: String,
    archive_sha256: String,
    wasm_sha256: String,
    parameters: Vec<ParameterDefinition>,
    feature_slots: Vec<FeatureSlotDefinition>,
    output_names: Vec<String>,
    dependencies: Vec<ComponentDependency>,
    warmup_bars: u32,
    model_scope: Option<ModelScope>,
    model_outputs: Vec<ModelOutput>,
    model_artifact: Option<ModelArtifact>,
    architecture: Option<StrategyArchitecture>,
    compatible: bool,
    compatibility_error: Option<String>,
    locked_by_run_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentUserRequest {
    pub user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPageRequest {
    pub user_id: String,
    pub page: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPage {
    pub items: Vec<LibraryComponent>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentImportRequest {
    pub user_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDeleteRequest {
    pub user_id: String,
    pub archive_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentArchiveRequest {
    pub user_id: String,
    pub archive_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestDependencyRequest {
    pub user_id: String,
    pub strategy_archive_sha256: String,
}

fn compatible_factor_hashes(
    strategy: &ComponentManifest,
    packages: &[(&LibraryComponent, ComponentPackage)],
) -> BTreeMap<String, Vec<String>> {
    strategy
        .dependencies
        .iter()
        .map(|dependency| {
            let hashes = packages
                .iter()
                .filter(|(component, package)| {
                    component.kind == "factor"
                        && component.compatible
                        && package.manifest.component_id == dependency.component_id
                        && dependency.version.matches(&package.manifest.version)
                })
                .map(|(component, _)| component.archive_sha256.clone())
                .collect();
            (dependency.alias.clone(), hashes)
        })
        .collect()
}

fn owned_component_hashes(database: &Connection, user_id: &str) -> Result<HashSet<String>, String> {
    let mut statement = database
        .prepare("SELECT archive_sha256 FROM component_access WHERE user_id = ?1")
        .map_err(string)?;
    statement
        .query_map([user_id], |row| row.get::<_, String>(0))
        .map_err(string)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(string)
}

/// The Component content one User accesses that no other User accesses.
/// Both Component Reset and Reset All stage and prune from this candidate
/// set after applying the Backtest module's Run-lock guard.
fn orphan_component_candidates(
    database: &Connection,
    user_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut statement = database
        .prepare(
            "SELECT c.archive_sha256, c.archive_path FROM component_content c
             JOIN component_access a USING(archive_sha256)
             WHERE a.user_id = ?1
             AND NOT EXISTS(SELECT 1 FROM component_access other
                 WHERE other.archive_sha256 = c.archive_sha256 AND other.user_id <> ?1)",
        )
        .map_err(string)?;
    statement
        .query_map([user_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

/// The distinct Runs of one User that lock Component Packages the User
/// still owns; the count both the summary and the Component Reset
/// blocking check report.
fn count_runs_locking_owned_components(
    locking_runs: &HashMap<String, Vec<String>>,
    owned_components: &HashSet<String>,
) -> u64 {
    locking_runs
        .iter()
        .filter(|(hash, _)| owned_components.contains(*hash))
        .flat_map(|(_, runs)| runs)
        .collect::<HashSet<_>>()
        .len() as u64
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

/// Renames each file to a staging name under an allowed root, refusing
/// paths outside it; the reset transaction's outcome decides whether the
/// staged files are removed or restored. Lives here because Component
/// archive staging owns the mechanism; the other reset flows reuse it.
pub(crate) fn stage_files(
    paths: impl IntoIterator<Item = PathBuf>,
    allowed_root: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut staged = Vec::new();
    for path in paths {
        if !path.starts_with(allowed_root) {
            restore_staged_files(&staged);
            return Err(format!(
                "Refusing to reset a file outside the local data store: {}",
                path.display()
            ));
        }
        if !path.is_file() {
            continue;
        }
        let temporary = path.with_extension(format!(
            "{}.reset",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("data")
        ));
        if temporary.exists() {
            restore_staged_files(&staged);
            return Err(format!(
                "Reset staging path already exists: {}",
                temporary.display()
            ));
        }
        if let Err(error) = fs::rename(&path, &temporary) {
            restore_staged_files(&staged);
            return Err(error.to_string());
        }
        staged.push((path, temporary));
    }
    Ok(staged)
}

pub(crate) fn finish_staged_files(
    staged: Vec<(PathBuf, PathBuf)>,
    result: Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = result {
        restore_staged_files(&staged);
        return Err(error);
    }
    for (_, temporary) in staged {
        let _ = fs::remove_file(temporary);
    }
    Ok(())
}

fn restore_staged_files(staged: &[(PathBuf, PathBuf)]) {
    for (path, temporary) in staged.iter().rev() {
        let _ = fs::rename(temporary, path);
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
