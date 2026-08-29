//! Static, Tauri-independent contracts for source-visible Python Research Projects.
//!
//! This crate deliberately does not import Python, open a database, spawn a
//! process, or own GUI state. Those boundaries belong to later M12 slices and
//! the native control plane.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use zip::{
    ZipArchive,
    write::{SimpleFileOptions, ZipWriter},
};

pub mod factor;
pub mod fixture;
pub mod model;
pub mod runner;
pub mod runtime;
pub mod tuning;

pub const PYTHON_RESEARCH_SCHEMA_VERSION: &str = "1.0.0";
const PYTHON_RESEARCH_METADATA_FILE: &str = "python-research-meta.json";
pub const PUBLIC_SDK_ARTIFACT_SHA256: &str =
    "54cb0dd8f1b2f911a30099f1c7ffdc3798cd3d18e7a331b6708b437f6fa28ed7";
pub const MAX_ARCHIVE_ENTRIES: usize = 512;
pub const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PROJECT_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_SOURCE_FILES: usize = 128;
pub const MAX_RESOURCE_WALL_MS: u64 = 30 * 60 * 1000;
pub const MAX_RESOURCE_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_RESOURCE_THREADS: u32 = 64;
pub const MAX_RESOURCE_INPUT_ROWS: u64 = 10_000_000;
pub const MAX_RESOURCE_OUTPUT_ROWS: u64 = 10_000_000;

fn default_max_output_rows() -> u64 {
    MAX_RESOURCE_OUTPUT_ROWS
}

const REQUIRED_FILES: [&str; 6] = [
    "adaq-project.toml",
    "pyproject.toml",
    "pylock.toml",
    "src/project.py",
    "README.md",
    "LICENSE",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonResearchError(pub String);

impl fmt::Display for PythonResearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PythonResearchError {}

impl From<std::io::Error> for PythonResearchError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<toml::de::Error> for PythonResearchError {
    fn from(error: toml::de::Error) -> Self {
        Self(format!("invalid-python-project-manifest: {error}"))
    }
}

impl From<toml::ser::Error> for PythonResearchError {
    fn from(error: toml::ser::Error) -> Self {
        Self(format!("invalid-python-project-manifest: {error}"))
    }
}

impl From<zip::result::ZipError> for PythonResearchError {
    fn from(error: zip::result::ZipError) -> Self {
        Self(format!("invalid-python-project-archive: {error}"))
    }
}

fn invalid(message: impl Into<String>) -> PythonResearchError {
    PythonResearchError(message.into())
}

fn schema_reset_diagnostic() -> String {
    format!(
        "python-research-schema-incompatible: expected {PYTHON_RESEARCH_SCHEMA_VERSION}; run Reset Python Research Evidence"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectKind {
    Factor,
    Model,
    Strategy,
}

impl ProjectKind {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Factor => "py-factor-",
            Self::Model => "py-model-",
            Self::Strategy => "py-strategy-",
        }
    }

    fn from_project_id(project_id: &str) -> Result<Self, PythonResearchError> {
        if let Some(_) = project_id.strip_prefix(Self::Factor.prefix()) {
            Ok(Self::Factor)
        } else if let Some(_) = project_id.strip_prefix(Self::Model.prefix()) {
            Ok(Self::Model)
        } else if let Some(_) = project_id.strip_prefix(Self::Strategy.prefix()) {
            Ok(Self::Strategy)
        } else {
            Err(invalid("project-id-kind-prefix-or-format-invalid"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectMode {
    ImperativePython,
    PortableDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectScope {
    Pointwise,
    TimeSeries,
    CrossSectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterType {
    Boolean,
    Decimal,
    Integer,
    String,
    Enum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ParameterSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub value_type: ParameterType,
    pub default: String,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct InputSlotSpec {
    pub id: String,
    pub role: String,
    pub scope: ProjectScope,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OutputSpec {
    pub id: String,
    pub value_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct TargetSpec {
    pub id: String,
    pub kind: String,
    pub horizon_bars: u32,
    pub value_scale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SignalSpec {
    pub id: String,
    pub kind: String,
    pub value_scale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ResourceRequest {
    pub max_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_threads: u32,
    pub max_input_rows: u64,
    pub max_output_rows: u64,
}

/// Host-owned ceiling applied to every Python Research Attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostResourcePolicy {
    pub policy_id: String,
    pub max_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_threads: u32,
    pub max_processes: u32,
    pub max_input_rows: u64,
    pub max_input_columns: u32,
    pub max_input_cells: u64,
    #[serde(default = "default_max_output_rows")]
    pub max_output_rows: u64,
    pub max_control_bytes: u64,
    pub max_arrow_bytes: u64,
    pub max_staged_bytes: u64,
    pub max_artifact_bytes: u64,
    pub max_checkpoint_bytes: u64,
    pub max_log_bytes: u64,
}

impl HostResourcePolicy {
    pub fn m12_default() -> Self {
        Self {
            policy_id: "adaq-python-resource-policy@1".into(),
            max_wall_ms: 30 * 60 * 1000,
            max_memory_bytes: 4 * 1024 * 1024 * 1024,
            max_threads: 64,
            max_processes: 1,
            max_input_rows: 10_000_000,
            max_input_columns: 1024,
            max_input_cells: 100_000_000,
            max_output_rows: 10_000_000,
            max_control_bytes: 16 * 1024 * 1024,
            max_arrow_bytes: 256 * 1024 * 1024,
            max_staged_bytes: 512 * 1024 * 1024,
            max_artifact_bytes: 128 * 1024 * 1024,
            max_checkpoint_bytes: 128 * 1024 * 1024,
            max_log_bytes: 1024 * 1024,
        }
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.policy_id.is_empty()
            || self.max_wall_ms == 0
            || self.max_memory_bytes == 0
            || self.max_threads == 0
            || self.max_processes == 0
            || self.max_input_rows == 0
            || self.max_input_columns == 0
            || self.max_input_cells == 0
            || self.max_output_rows == 0
            || self.max_control_bytes == 0
            || self.max_arrow_bytes == 0
            || self.max_staged_bytes == 0
            || self.max_artifact_bytes == 0
            || self.max_checkpoint_bytes == 0
            || self.max_log_bytes == 0
        {
            return Err(invalid("resource-policy-invalid"));
        }
        Ok(())
    }

    pub fn lowered_by(&self, request: &Self) -> Result<Self, PythonResearchError> {
        self.validate()?;
        request.validate()?;
        if request.policy_id != self.policy_id {
            return Err(invalid("resource-policy-version-mismatch"));
        }
        let values = [
            (request.max_wall_ms, self.max_wall_ms),
            (request.max_memory_bytes, self.max_memory_bytes),
            (request.max_threads as u64, self.max_threads as u64),
            (request.max_processes as u64, self.max_processes as u64),
            (request.max_input_rows, self.max_input_rows),
            (
                request.max_input_columns as u64,
                self.max_input_columns as u64,
            ),
            (request.max_input_cells, self.max_input_cells),
            (request.max_output_rows, self.max_output_rows),
            (request.max_control_bytes, self.max_control_bytes),
            (request.max_arrow_bytes, self.max_arrow_bytes),
            (request.max_staged_bytes, self.max_staged_bytes),
            (request.max_artifact_bytes, self.max_artifact_bytes),
            (request.max_checkpoint_bytes, self.max_checkpoint_bytes),
            (request.max_log_bytes, self.max_log_bytes),
        ];
        if values.iter().any(|(requested, host)| requested > host) {
            return Err(invalid("resource-policy-request-exceeds-host"));
        }
        Ok(request.clone())
    }

    pub fn lowered_by_request(
        &self,
        request: &ResourceRequest,
    ) -> Result<Self, PythonResearchError> {
        request.validate()?;
        let mut lowered = self.clone();
        lowered.max_wall_ms = request.max_wall_ms;
        lowered.max_memory_bytes = request.max_memory_bytes;
        lowered.max_threads = request.max_threads;
        lowered.max_input_rows = request.max_input_rows;
        lowered.max_output_rows = request.max_output_rows;
        self.lowered_by(&lowered)
    }
}

impl ResourceRequest {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.max_wall_ms == 0 || self.max_wall_ms > MAX_RESOURCE_WALL_MS {
            return Err(invalid("resource-wall-limit-invalid"));
        }
        if self.max_memory_bytes == 0 || self.max_memory_bytes > MAX_RESOURCE_MEMORY_BYTES {
            return Err(invalid("resource-memory-limit-invalid"));
        }
        if self.max_threads == 0 || self.max_threads > MAX_RESOURCE_THREADS {
            return Err(invalid("resource-thread-limit-invalid"));
        }
        if self.max_input_rows == 0 || self.max_input_rows > MAX_RESOURCE_INPUT_ROWS {
            return Err(invalid("resource-input-row-limit-invalid"));
        }
        if self.max_output_rows == 0 || self.max_output_rows > MAX_RESOURCE_OUTPUT_ROWS {
            return Err(invalid("resource-output-row-limit-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ProjectManifest {
    pub schema_version: String,
    pub project_id: String,
    pub kind: ProjectKind,
    #[serde(default)]
    pub mode: Option<ProjectMode>,
    pub scope: ProjectScope,
    pub entry_point: String,
    pub sdk_profile: String,
    pub runtime_profile: String,
    #[serde(default)]
    pub source_files: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<ParameterSpec>,
    pub input_slots: Vec<InputSlotSpec>,
    pub outputs: Vec<OutputSpec>,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(default)]
    pub signal: Option<SignalSpec>,
    #[serde(default)]
    pub adapter_id: Option<String>,
    pub dependency_lock_sha256: String,
    pub resource_request: ResourceRequest,
    pub license: String,
}

impl ProjectManifest {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.schema_version != PYTHON_RESEARCH_SCHEMA_VERSION {
            return Err(invalid(format!(
                "python-research-schema-incompatible: expected {PYTHON_RESEARCH_SCHEMA_VERSION}"
            )));
        }
        validate_project_id(&self.project_id, self.kind)?;
        validate_entry_point(&self.entry_point)?;
        if self.sdk_profile != "adaq-research-sdk@1" {
            return Err(invalid("unsupported-sdk-profile"));
        }
        if self.runtime_profile != "adaq-python@1" {
            return Err(invalid("unsupported-runtime-profile"));
        }
        match self.kind {
            ProjectKind::Model if self.mode.is_some() => {
                return Err(invalid("model-project-must-not-declare-mode"));
            }
            ProjectKind::Factor | ProjectKind::Strategy if self.mode.is_none() => {
                return Err(invalid("factor-or-strategy-project-requires-mode"));
            }
            _ => {}
        }
        if self.input_slots.is_empty() || self.outputs.is_empty() {
            return Err(invalid("project-contract-must-declare-inputs-and-outputs"));
        }
        validate_unique_ids(
            self.input_slots.iter().map(|slot| slot.id.as_str()),
            "input-slot",
        )?;
        validate_unique_ids(
            self.outputs.iter().map(|output| output.id.as_str()),
            "output",
        )?;
        validate_unique_ids(
            self.parameters
                .iter()
                .map(|parameter| parameter.id.as_str()),
            "parameter",
        )?;
        for parameter in &self.parameters {
            validate_identifier(&parameter.id, "parameter-id")?;
            if parameter.default.trim().is_empty() {
                return Err(invalid("parameter-default-must-not-be-empty"));
            }
            if parameter.value_type == ParameterType::Enum && parameter.allowed_values.is_empty() {
                return Err(invalid("enum-parameter-must-declare-values"));
            }
            if !parameter.allowed_values.is_empty()
                && !parameter.allowed_values.contains(&parameter.default)
            {
                return Err(invalid("parameter-default-is-not-allowed"));
            }
        }
        for slot in &self.input_slots {
            validate_identifier(&slot.id, "input-slot-id")?;
            if slot.role.trim().is_empty() {
                return Err(invalid("input-slot-role-must-not-be-empty"));
            }
        }
        for output in &self.outputs {
            validate_identifier(&output.id, "output-id")?;
            if output.value_type.trim().is_empty() {
                return Err(invalid("output-type-must-not-be-empty"));
            }
        }
        if self.source_files.is_empty() || self.source_files.iter().any(|path| path == "") {
            return Err(invalid("project-must-declare-source-files"));
        }
        if self.source_files.len() > MAX_SOURCE_FILES {
            return Err(invalid("project-source-file-count-exceeded"));
        }
        let mut source_files = BTreeSet::new();
        for path in &self.source_files {
            validate_project_path(path)?;
            if !path.starts_with("src/") || !path.ends_with(".py") {
                return Err(invalid("declared-source-must-be-python-under-src"));
            }
            if !source_files.insert(path) {
                return Err(invalid("duplicate-declared-source"));
            }
        }
        if !source_files
            .iter()
            .any(|path| path.as_str() == "src/project.py")
        {
            return Err(invalid("src-project-py-must-be-declared"));
        }
        if !is_sha256(&self.dependency_lock_sha256) {
            return Err(invalid("dependency-lock-sha256-invalid"));
        }
        if self.license.trim().is_empty() {
            return Err(invalid("project-license-is-required"));
        }
        if self.kind == ProjectKind::Model {
            model::validate_model_manifest(self)?;
        }
        self.resource_request.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationState {
    Clean,
    Dirty,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ValidationIssue {
    pub code: String,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ValidationReport {
    pub state: ValidationState,
    pub manifest: Option<ProjectManifest>,
    pub manifest_sha256: Option<String>,
    pub source_sha256: Option<String>,
    pub dependency_lock_sha256: Option<String>,
    pub files: Vec<String>,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn valid(&self) -> bool {
        matches!(self.state, ValidationState::Clean)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectRevision {
    pub revision_sha256: String,
    pub project_id: String,
    pub manifest_sha256: String,
    pub source_sha256: String,
    pub dependency_lock_sha256: String,
    pub sdk_artifact_sha256: String,
    pub runtime_profile: String,
    pub runtime_artifact_sha256: Option<String>,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TrustDecision {
    pub decision_id: String,
    pub project_id: String,
    pub revision_sha256: String,
    pub user_id: String,
    pub decided_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenceKind {
    Proprietary,
    Redistributable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkingCopyState {
    Clean,
    Dirty,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkingCopySummary {
    pub user_id: String,
    pub project_id: String,
    pub path: String,
    pub state: WorkingCopyState,
    pub revision_sha256: Option<String>,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonResearchResetReport {
    pub user_id: String,
    pub preserves_working_copies: bool,
    pub preserves_exported_archives: bool,
    pub removes_revision_metadata: bool,
    pub removes_attempt_metadata: bool,
    pub removes_trust_decisions: bool,
    pub removes_result_metadata: bool,
    pub preserves_cache: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectStore {
    root: PathBuf,
    baselines: Arc<Mutex<BTreeMap<String, String>>>,
    revisions: Arc<Mutex<BTreeMap<String, Vec<ProjectRevision>>>>,
    environment_gate: Arc<Mutex<()>>,
    schema_error: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PythonResearchMetadata {
    schema_version: String,
}

impl ProjectStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let baselines = fs::read(root.join("working-copy-baselines.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let revisions = fs::read(root.join("revisions.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let schema_error = match fs::read(root.join(PYTHON_RESEARCH_METADATA_FILE)) {
            Ok(bytes) => match serde_json::from_slice::<PythonResearchMetadata>(&bytes) {
                Ok(metadata) if metadata.schema_version == PYTHON_RESEARCH_SCHEMA_VERSION => None,
                Ok(_) | Err(_) => Some(schema_reset_diagnostic()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(format!("python-research-metadata-unreadable:{error}")),
        };
        Self {
            root,
            baselines: Arc::new(Mutex::new(baselines)),
            revisions: Arc::new(Mutex::new(revisions)),
            environment_gate: Arc::new(Mutex::new(())),
            schema_error: Arc::new(Mutex::new(schema_error)),
        }
    }

    pub fn create_from_example(
        &self,
        user_id: &str,
        example_root: &Path,
        project_id: &str,
    ) -> Result<WorkingCopySummary, PythonResearchError> {
        self.ensure_compatible()?;
        validate_user_id(user_id)?;
        let source_report = inspect_project(example_root);
        if !source_report.valid() {
            return Err(invalid("bundled-example-is-invalid"));
        }
        let manifest = source_report
            .manifest
            .ok_or_else(|| invalid("bundled-example-manifest-missing"))?;
        if manifest.project_id != project_id {
            return Err(invalid("example-project-id-mismatch"));
        }
        validate_project_id(project_id, manifest.kind)?;
        let destination = self.project_path(user_id, project_id)?;
        if destination.exists() {
            return Err(invalid("project-working-copy-already-exists"));
        }
        self.copy_project(example_root, &destination, &source_report.files)?;
        self.summary(user_id, &destination)
    }

    pub fn import_archive(
        &self,
        user_id: &str,
        bytes: &[u8],
    ) -> Result<WorkingCopySummary, PythonResearchError> {
        self.ensure_compatible()?;
        validate_user_id(user_id)?;
        let archive = validate_archive(bytes)?;
        let destination = self.project_path(user_id, &archive.project_id)?;
        if destination.exists() {
            return Err(invalid("project-working-copy-already-exists"));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        import_archive(bytes, &destination)?;
        self.summary(user_id, &destination)
    }

    pub fn validate(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<ValidationReport, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        if !path.is_dir() {
            return Err(invalid("project-working-copy-not-found"));
        }
        Ok(self.report_with_state(user_id, &path))
    }

    pub fn dependency_intent(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<runtime::DependencyIntent, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        if !path.is_dir() {
            return Err(invalid("project-working-copy-not-found"));
        }
        runtime::DependencyIntent::parse(&fs::read(path.join("pyproject.toml"))?)
    }

    pub fn dependency_lock_sha256(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<String, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        let report = inspect_project(&path);
        report
            .valid()
            .then(|| report.dependency_lock_sha256.clone())
            .flatten()
            .ok_or_else(|| invalid("python-project-lock-not-ready"))
    }

    pub fn dependency_lock_bytes(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<Vec<u8>, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        let report = inspect_project(&path);
        if !report.valid() {
            return Err(invalid("python-project-lock-not-ready"));
        }
        fs::read(path.join("pylock.toml")).map_err(Into::into)
    }

    pub fn apply_environment_lock(
        &self,
        user_id: &str,
        project_id: &str,
        lock: &runtime::EnvironmentLock,
    ) -> Result<String, PythonResearchError> {
        let _environment_gate = self
            .environment_gate
            .lock()
            .map_err(|_| invalid("python-project-environment-gate-poisoned"))?;
        self.ensure_compatible()?;
        lock.validate()?;
        let path = self.project_path(user_id, project_id)?;
        let report = inspect_project(&path);
        if !report.valid() {
            return Err(invalid("cannot-write-lock-to-invalid-project"));
        }
        let mut manifest = report
            .manifest
            .ok_or_else(|| invalid("project-manifest-missing"))?;
        let lock_bytes = toml::to_string(lock)?.into_bytes();
        let lock_hash = sha256(&lock_bytes);
        manifest.dependency_lock_sha256 = lock_hash.clone();
        let manifest_bytes = toml::to_string(&manifest)?.into_bytes();
        let lock_temporary = path.join("pylock.toml.tmp");
        let manifest_temporary = path.join("adaq-project.toml.tmp");
        let result = (|| {
            fs::write(&lock_temporary, lock_bytes)?;
            fs::write(&manifest_temporary, manifest_bytes)?;
            publish_environment_files(
                &lock_temporary,
                &path.join("pylock.toml"),
                &manifest_temporary,
                &path.join("adaq-project.toml"),
            )
        })();
        if result.is_err() {
            let _ = fs::remove_file(&lock_temporary);
            let _ = fs::remove_file(&manifest_temporary);
        }
        result?;
        let revision = freeze_revision(&path, sha256(b"unresolved-sdk-artifact"), None)?;
        let key = format!("{user_id}:{}", path.to_string_lossy());
        let mut baselines = self
            .baselines
            .lock()
            .map_err(|_| invalid("project-baseline-store-lock-poisoned"))?;
        baselines.insert(key, revision.revision_sha256);
        self.persist_baselines(&baselines);
        Ok(lock_hash)
    }

    pub fn freeze(
        &self,
        user_id: &str,
        project_id: &str,
        sdk_artifact_sha256: impl Into<String>,
        runtime_artifact_sha256: Option<String>,
    ) -> Result<ProjectRevision, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        let revision = freeze_revision(&path, sdk_artifact_sha256, runtime_artifact_sha256)?;
        let mut revisions = self
            .revisions
            .lock()
            .map_err(|_| invalid("project-revision-store-lock-poisoned"))?;
        let entries = revisions
            .entry(format!("{user_id}:{project_id}"))
            .or_default();
        if entries
            .last()
            .is_none_or(|existing| existing.revision_sha256 != revision.revision_sha256)
        {
            entries.push(revision.clone());
            persist_json_file(&self.root.join("revisions.json"), &*revisions)?;
        }
        self.persist_revision_archive(user_id, project_id, &revision)?;
        Ok(revision)
    }

    pub fn revision(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
    ) -> Result<ProjectRevision, PythonResearchError> {
        self.ensure_compatible()?;
        validate_user_id(user_id)?;
        validate_project_id(project_id, ProjectKind::from_project_id(project_id)?)?;
        if !is_sha256(revision_sha256) {
            return Err(invalid("project-revision-identity-invalid"));
        }
        self.revisions
            .lock()
            .map_err(|_| invalid("project-revision-store-lock-poisoned"))?
            .get(&format!("{user_id}:{project_id}"))
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|revision| revision.revision_sha256 == revision_sha256)
            })
            .cloned()
            .ok_or_else(|| invalid("project-revision-not-found"))
    }

    pub fn revision_manifest(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
    ) -> Result<ProjectManifest, PythonResearchError> {
        self.ensure_compatible()?;
        let archive = self.revision_archive_path(user_id, project_id, revision_sha256)?;
        let bytes = fs::read(archive)?;
        Ok(validate_archive(&bytes)?.manifest)
    }

    pub fn materialize_revision(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        destination: &Path,
    ) -> Result<ProjectRevision, PythonResearchError> {
        self.ensure_compatible()?;
        let revision = self.revision(user_id, project_id, revision_sha256)?;
        let archive = self.revision_archive_path(user_id, project_id, revision_sha256)?;
        if !archive.is_file() {
            return Err(invalid("project-revision-archive-not-found"));
        }
        import_archive(&fs::read(archive)?, destination)?;
        Ok(revision)
    }

    pub fn export(
        &self,
        user_id: &str,
        project_id: &str,
        revision: &ProjectRevision,
    ) -> Result<Vec<u8>, PythonResearchError> {
        self.ensure_compatible()?;
        let path = self.project_path(user_id, project_id)?;
        let archive = self.revision_archive_path(user_id, project_id, &revision.revision_sha256)?;
        let current = freeze_revision(
            &path,
            revision.sdk_artifact_sha256.clone(),
            revision.runtime_artifact_sha256.clone(),
        );
        if current.is_ok_and(|current| current == *revision) {
            return deterministic_archive(&path, revision);
        }
        fs::read(archive).map_err(Into::into)
    }

    pub fn list(&self, user_id: &str) -> Result<Vec<WorkingCopySummary>, PythonResearchError> {
        self.ensure_compatible()?;
        validate_user_id(user_id)?;
        let directory = self.user_directory(user_id)?;
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut projects = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                projects.push(self.summary(user_id, &entry.path())?);
            }
        }
        projects.sort_by(|left, right| left.project_id.cmp(&right.project_id));
        Ok(projects)
    }

    pub fn summary(
        &self,
        user_id: &str,
        project_root: &Path,
    ) -> Result<WorkingCopySummary, PythonResearchError> {
        self.ensure_compatible()?;
        validate_user_id(user_id)?;
        let report = self.report_with_state(user_id, project_root);
        let project_id = report
            .manifest
            .as_ref()
            .map(|manifest| manifest.project_id.clone())
            .or_else(|| {
                project_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "unknown-project".into());
        let revision_sha256 = self.revisions.lock().ok().and_then(|revisions| {
            revisions
                .get(&format!("{user_id}:{project_id}"))
                .and_then(|entries| entries.last())
                .map(|revision| revision.revision_sha256.clone())
        });
        Ok(WorkingCopySummary {
            user_id: user_id.into(),
            project_id,
            path: project_root.to_string_lossy().into_owned(),
            state: match report.state {
                ValidationState::Clean => WorkingCopyState::Clean,
                ValidationState::Dirty => WorkingCopyState::Dirty,
                ValidationState::Invalid => WorkingCopyState::Invalid,
            },
            revision_sha256,
            issues: report.issues,
        })
    }

    pub fn reset_python_research_evidence(
        &self,
        user_id: &str,
    ) -> Result<PythonResearchResetReport, PythonResearchError> {
        validate_user_id(user_id)?;
        let mut baselines = self
            .baselines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prefix = format!("{user_id}:");
        baselines.retain(|key, _| !key.starts_with(&prefix));
        self.persist_baselines(&baselines);
        let mut revisions = self
            .revisions
            .lock()
            .map_err(|_| invalid("project-revision-store-lock-poisoned"))?;
        revisions.retain(|key, _| !key.starts_with(&prefix));
        persist_json_file(&self.root.join("revisions.json"), &*revisions)?;
        let revision_directory = self.root.join("revisions").join(user_id);
        if revision_directory.is_dir() {
            fs::remove_dir_all(revision_directory)?;
        }
        persist_json_file(
            &self.root.join(PYTHON_RESEARCH_METADATA_FILE),
            &PythonResearchMetadata {
                schema_version: PYTHON_RESEARCH_SCHEMA_VERSION.into(),
            },
        )?;
        *self
            .schema_error
            .lock()
            .map_err(|_| invalid("project-schema-lock-poisoned"))? = None;
        Ok(PythonResearchResetReport {
            user_id: user_id.into(),
            preserves_working_copies: true,
            preserves_exported_archives: true,
            removes_revision_metadata: true,
            removes_attempt_metadata: true,
            removes_trust_decisions: true,
            removes_result_metadata: true,
            preserves_cache: true,
        })
    }

    fn ensure_compatible(&self) -> Result<(), PythonResearchError> {
        if let Some(error) = self
            .schema_error
            .lock()
            .map_err(|_| invalid("project-schema-lock-poisoned"))?
            .clone()
        {
            return Err(invalid(error));
        }
        let metadata_path = self.root.join(PYTHON_RESEARCH_METADATA_FILE);
        if !metadata_path.is_file() {
            persist_json_file(
                &metadata_path,
                &PythonResearchMetadata {
                    schema_version: PYTHON_RESEARCH_SCHEMA_VERSION.into(),
                },
            )?;
        }
        Ok(())
    }

    fn user_directory(&self, user_id: &str) -> Result<PathBuf, PythonResearchError> {
        validate_user_id(user_id)?;
        Ok(self.root.join("working-copies").join(user_id))
    }

    fn project_path(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<PathBuf, PythonResearchError> {
        validate_user_id(user_id)?;
        if !project_id.starts_with("py-") {
            return Err(invalid("project-id-kind-prefix-or-format-invalid"));
        }
        Ok(self.user_directory(user_id)?.join(project_id))
    }

    fn persist_revision_archive(
        &self,
        user_id: &str,
        project_id: &str,
        revision: &ProjectRevision,
    ) -> Result<(), PythonResearchError> {
        let archive = deterministic_archive(&self.project_path(user_id, project_id)?, revision)?;
        let destination =
            self.revision_archive_path(user_id, project_id, &revision.revision_sha256)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = destination.with_extension("zip.tmp");
        fs::write(&temporary, archive)?;
        fs::rename(temporary, destination)?;
        Ok(())
    }

    fn revision_archive_path(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
    ) -> Result<PathBuf, PythonResearchError> {
        validate_user_id(user_id)?;
        validate_project_id(project_id, ProjectKind::from_project_id(project_id)?)?;
        if !is_sha256(revision_sha256) {
            return Err(invalid("project-revision-identity-invalid"));
        }
        Ok(self
            .root
            .join("revisions")
            .join(user_id)
            .join(project_id)
            .join(format!("{revision_sha256}.zip")))
    }

    fn copy_project(
        &self,
        source: &Path,
        destination: &Path,
        files: &[String],
    ) -> Result<(), PythonResearchError> {
        let staging = destination.with_extension(format!(
            "staging-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid("system-clock-before-unix-epoch"))?
                .as_nanos()
        ));
        fs::create_dir_all(&staging)?;
        let result = (|| {
            for file in files {
                let target = staging.join(file);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(source.join(file), target)?;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&staging, destination)?;
            Ok::<(), PythonResearchError>(())
        })();
        if result.is_err() {
            fs::remove_dir_all(&staging).ok();
        }
        result
    }

    fn report_with_state(&self, user_id: &str, project_root: &Path) -> ValidationReport {
        let mut report = inspect_project(project_root);
        if !report.valid() {
            return report;
        }
        let Ok(revision) = freeze_revision(project_root, sha256(b"unresolved-sdk-artifact"), None)
        else {
            report.state = ValidationState::Invalid;
            report.issues.push(ValidationIssue {
                code: "revision-freeze-failed".into(),
                path: None,
                message: "valid project could not be frozen".into(),
            });
            return report;
        };
        let key = format!("{user_id}:{}", project_root.to_string_lossy());
        let mut baselines = self
            .baselines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match baselines.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(revision.revision_sha256);
                self.persist_baselines(&baselines);
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get() != &revision.revision_sha256 =>
            {
                report.state = ValidationState::Dirty;
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
        report
    }

    fn persist_baselines(&self, baselines: &BTreeMap<String, String>) {
        let temporary = self.root.join("working-copy-baselines.json.tmp");
        if fs::create_dir_all(&self.root).is_ok()
            && serde_json::to_vec(baselines)
                .ok()
                .and_then(|bytes| fs::write(&temporary, bytes).ok())
                .is_some()
        {
            let _ = fs::rename(temporary, self.root.join("working-copy-baselines.json"));
        }
    }
}

fn persist_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), PythonResearchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(|error| invalid(error.to_string()))?,
    )?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn publish_environment_files(
    lock_temporary: &Path,
    lock_destination: &Path,
    manifest_temporary: &Path,
    manifest_destination: &Path,
) -> Result<(), PythonResearchError> {
    let lock_backup = lock_destination.with_extension("bak");
    let manifest_backup = manifest_destination.with_extension("bak");
    let _ = fs::remove_file(&lock_backup);
    let _ = fs::remove_file(&manifest_backup);
    let lock_had_destination = lock_destination.exists();
    let manifest_had_destination = manifest_destination.exists();
    if lock_had_destination {
        fs::rename(lock_destination, &lock_backup)?;
    }
    if let Err(error) = if manifest_had_destination {
        fs::rename(manifest_destination, &manifest_backup)
    } else {
        Ok(())
    } {
        if lock_had_destination {
            let _ = fs::rename(&lock_backup, lock_destination);
        }
        return Err(error.into());
    }

    let result = (|| {
        fs::rename(lock_temporary, lock_destination)?;
        fs::rename(manifest_temporary, manifest_destination)?;
        Ok::<(), PythonResearchError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(lock_destination);
        let _ = fs::remove_file(manifest_destination);
        if lock_had_destination {
            let _ = fs::rename(&lock_backup, lock_destination);
        }
        if manifest_had_destination {
            let _ = fs::rename(&manifest_backup, manifest_destination);
        }
    } else {
        let _ = fs::remove_file(lock_backup);
        let _ = fs::remove_file(manifest_backup);
    }
    result
}

pub fn parse_manifest(bytes: &[u8]) -> Result<ProjectManifest, PythonResearchError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("manifest-must-be-utf8"))?;
    let manifest: ProjectManifest = toml::from_str(text)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn manifest_bytes(manifest: &ProjectManifest) -> Result<Vec<u8>, PythonResearchError> {
    manifest.validate()?;
    Ok(toml::to_string(manifest)?.into_bytes())
}

pub fn inspect_project(root: &Path) -> ValidationReport {
    match inspect_project_inner(root) {
        Ok((manifest, files, manifest_hash, source_hash, lock_hash)) => ValidationReport {
            state: ValidationState::Clean,
            manifest: Some(manifest),
            manifest_sha256: Some(manifest_hash),
            source_sha256: Some(source_hash),
            dependency_lock_sha256: Some(lock_hash),
            files,
            issues: Vec::new(),
        },
        Err(error) => ValidationReport {
            state: ValidationState::Invalid,
            manifest: read_manifest_for_diagnostics(root),
            manifest_sha256: read_hash(root.join("adaq-project.toml")).ok(),
            source_sha256: None,
            dependency_lock_sha256: read_hash(root.join("pylock.toml")).ok(),
            files: list_relative_files(root).unwrap_or_default(),
            issues: vec![ValidationIssue {
                code: error_code(&error.0),
                path: None,
                message: error.0,
            }],
        },
    }
}

fn inspect_project_inner(
    root: &Path,
) -> Result<(ProjectManifest, Vec<String>, String, String, String), PythonResearchError> {
    if !root.is_dir() {
        return Err(invalid("project-root-is-not-a-directory"));
    }
    let files = list_relative_files(root)?;
    if files.iter().any(|path| !is_allowed_project_file(path)) {
        return Err(invalid("project-contains-undeclared-file"));
    }
    for required in REQUIRED_FILES {
        if !files.iter().any(|path| path == required) {
            return Err(invalid(format!("project-missing-required-file:{required}")));
        }
    }
    let manifest_bytes = fs::read(root.join("adaq-project.toml"))?;
    check_file_size(&manifest_bytes, "adaq-project.toml")?;
    let manifest = parse_manifest(&manifest_bytes)?;
    for source_file in &manifest.source_files {
        if !files.iter().any(|path| path == source_file) {
            return Err(invalid(format!(
                "declared-source-file-missing:{source_file}"
            )));
        }
    }
    if files
        .iter()
        .filter(|path| path.starts_with("src/") && path.ends_with(".py"))
        .any(|path| {
            !manifest
                .source_files
                .iter()
                .any(|declared| declared == path)
        })
    {
        return Err(invalid("project-contains-undeclared-python-source"));
    }
    let lock_bytes = fs::read(root.join("pylock.toml"))?;
    check_file_size(&lock_bytes, "pylock.toml")?;
    let lock_hash = sha256(&lock_bytes);
    if lock_hash != manifest.dependency_lock_sha256 {
        return Err(invalid("dependency-lock-hash-mismatch"));
    }
    validate_license(&manifest.license, &fs::read(root.join("LICENSE"))?)?;
    for file in &files {
        check_file_size(&fs::read(root.join(file))?, file)?;
    }
    let source_hash = source_hash(root, &files)?;
    validate_entry_point_source(root.join("src/project.py"))?;
    Ok((
        manifest,
        files,
        sha256(&manifest_bytes),
        source_hash,
        lock_hash,
    ))
}

pub fn freeze_revision(
    root: &Path,
    sdk_artifact_sha256: impl Into<String>,
    runtime_artifact_sha256: Option<String>,
) -> Result<ProjectRevision, PythonResearchError> {
    let report = inspect_project(root);
    if !report.valid() {
        return Err(invalid("cannot-freeze-invalid-project"));
    }
    let manifest = report
        .manifest
        .ok_or_else(|| invalid("validated-project-manifest-missing"))?;
    let sdk_artifact_sha256 = sdk_artifact_sha256.into();
    if !is_sha256(&sdk_artifact_sha256) {
        return Err(invalid("sdk-artifact-sha256-invalid"));
    }
    if runtime_artifact_sha256
        .as_ref()
        .is_some_and(|hash| !is_sha256(hash))
    {
        return Err(invalid("runtime-artifact-sha256-invalid"));
    }
    let mut files = BTreeMap::new();
    for path in &report.files {
        files.insert(path.clone(), sha256(&fs::read(root.join(path))?));
    }
    let mut revision = ProjectRevision {
        revision_sha256: String::new(),
        project_id: manifest.project_id,
        manifest_sha256: report
            .manifest_sha256
            .ok_or_else(|| invalid("validated-manifest-hash-missing"))?,
        source_sha256: report
            .source_sha256
            .ok_or_else(|| invalid("validated-source-hash-missing"))?,
        dependency_lock_sha256: report
            .dependency_lock_sha256
            .ok_or_else(|| invalid("validated-lock-hash-missing"))?,
        sdk_artifact_sha256,
        runtime_profile: manifest.runtime_profile,
        runtime_artifact_sha256,
        files,
    };
    let identity = serde_json::to_vec(&revision).map_err(|error| invalid(error.to_string()))?;
    revision.revision_sha256 = sha256(&identity);
    Ok(revision)
}

pub fn deterministic_archive(
    root: &Path,
    revision: &ProjectRevision,
) -> Result<Vec<u8>, PythonResearchError> {
    let report = inspect_project(root);
    if !report.valid() {
        return Err(invalid("cannot-export-invalid-project"));
    }
    let current = freeze_revision(
        root,
        revision.sdk_artifact_sha256.clone(),
        revision.runtime_artifact_sha256.clone(),
    )?;
    if current != *revision {
        return Err(invalid("project-changed-after-revision-freeze"));
    }
    validate_license(
        &report
            .manifest
            .as_ref()
            .ok_or_else(|| invalid("validated-project-manifest-missing"))?
            .license,
        &fs::read(root.join("LICENSE"))?,
    )?;
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644)
        .last_modified_time(zip::DateTime::default());
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for path in &report.files {
        writer.start_file(path, options)?;
        writer.write_all(&fs::read(root.join(path))?)?;
    }
    Ok(writer.finish()?.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedArchive {
    pub files: Vec<String>,
    pub project_id: String,
    pub revision_sha256: String,
    pub untrusted: bool,
    pub manifest: ProjectManifest,
}

pub fn validate_archive(bytes: &[u8]) -> Result<ImportedArchive, PythonResearchError> {
    if bytes.is_empty() {
        return Err(invalid("project-archive-is-empty"));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() == 0 || archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(invalid("project-archive-entry-count-exceeded"));
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut expanded_size = 0_u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let name = validate_archive_path(file.name())?;
        if file.is_dir() {
            return Err(invalid("project-archive-directory-entry-not-allowed"));
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(invalid("project-archive-symbolic-link-not-allowed"));
        }
        if file.unix_mode().is_some_and(|mode| {
            let file_type = mode & 0o170000;
            file_type != 0 && file_type != 0o100000
        }) {
            return Err(invalid("project-archive-special-file-not-allowed"));
        }
        if files.contains_key(&name) || files.keys().any(|path| path.eq_ignore_ascii_case(&name)) {
            return Err(invalid("project-archive-duplicate-or-case-colliding-path"));
        }
        if file.size() > MAX_PROJECT_FILE_BYTES {
            return Err(invalid("project-archive-file-size-exceeded"));
        }
        expanded_size = expanded_size
            .checked_add(file.size())
            .ok_or_else(|| invalid("project-archive-expanded-size-overflow"))?;
        if expanded_size > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err(invalid("project-archive-expanded-size-exceeded"));
        }
        let mut content = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut content)?;
        if content.len() as u64 != file.size() {
            return Err(invalid("project-archive-entry-size-mismatch"));
        }
        files.insert(name, content);
    }
    for required in REQUIRED_FILES {
        if !files.contains_key(required) {
            return Err(invalid(format!(
                "project-archive-missing-required-file:{required}"
            )));
        }
    }
    let manifest = parse_manifest(
        files
            .get("adaq-project.toml")
            .ok_or_else(|| invalid("project-archive-manifest-missing"))?,
    )?;
    let lock_hash = sha256(
        files
            .get("pylock.toml")
            .ok_or_else(|| invalid("project-archive-lock-missing"))?,
    );
    if lock_hash != manifest.dependency_lock_sha256 {
        return Err(invalid("dependency-lock-hash-mismatch"));
    }
    validate_license(
        &manifest.license,
        files
            .get("LICENSE")
            .ok_or_else(|| invalid("project-archive-license-missing"))?,
    )?;
    let allowed = files.keys().all(|path| is_allowed_project_file(path));
    if !allowed {
        return Err(invalid("project-archive-contains-undeclared-file"));
    }
    let temporary = temporary_directory("adaq-python-archive")?;
    let result = (|| {
        write_files(&temporary, &files)?;
        freeze_revision(&temporary, sha256(b"unresolved-sdk-artifact"), None)
    })();
    fs::remove_dir_all(&temporary).ok();
    let revision = result?;
    Ok(ImportedArchive {
        files: files.keys().cloned().collect(),
        project_id: manifest.project_id.clone(),
        revision_sha256: revision.revision_sha256,
        untrusted: true,
        manifest,
    })
}

pub fn import_archive(
    bytes: &[u8],
    destination: &Path,
) -> Result<ImportedArchive, PythonResearchError> {
    let validated = validate_archive(bytes)?;
    if destination.exists() {
        return Err(invalid("project-import-destination-already-exists"));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let temporary = destination.with_extension(format!(
        "staging-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system-clock-before-unix-epoch"))?
            .as_nanos()
    ));
    fs::create_dir_all(&temporary)?;
    let result = (|| {
        for index in 0..archive.len() {
            let mut file = archive.by_index(index)?;
            let name = validate_archive_path(file.name())?;
            let path = temporary.join(&name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::File::create(path)?;
            std::io::copy(&mut file, &mut output)?;
        }
        fs::rename(&temporary, destination)?;
        Ok::<(), PythonResearchError>(())
    })();
    if result.is_err() {
        fs::remove_dir_all(&temporary).ok();
    }
    result?;
    Ok(validated)
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn validate_project_id(value: &str, kind: ProjectKind) -> Result<(), PythonResearchError> {
    if !value.starts_with(kind.prefix())
        || value.len() <= kind.prefix().len()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || value.contains("--")
        || value.ends_with('-')
    {
        return Err(invalid("project-id-kind-prefix-or-format-invalid"));
    }
    Ok(())
}

pub fn validate_entry_point(value: &str) -> Result<(), PythonResearchError> {
    let Some((module, function)) = value.split_once(':') else {
        return Err(invalid("project-entry-point-must-be-module-function"));
    };
    if value.matches(':').count() != 1
        || module.is_empty()
        || module.split('.').any(|part| !is_python_identifier(part))
        || function != "create_project"
    {
        return Err(invalid(
            "project-entry-point-must-be-zero-argument-create-project",
        ));
    }
    Ok(())
}

fn validate_entry_point_source(path: PathBuf) -> Result<(), PythonResearchError> {
    let bytes = fs::read(path)?;
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| invalid("project-entry-point-source-must-be-utf8"))?;
    let mut definitions = 0;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("async def create_project") {
            return Err(invalid("project-entry-point-must-not-be-async"));
        }
        let Some(rest) = trimmed.strip_prefix("def create_project") else {
            continue;
        };
        definitions += 1;
        let Some(arguments) = rest.strip_prefix('(') else {
            return Err(invalid("project-entry-point-definition-is-malformed"));
        };
        let Some(close) = arguments.find(')') else {
            return Err(invalid("project-entry-point-definition-is-malformed"));
        };
        if !arguments[..close].trim().is_empty() {
            return Err(invalid(
                "project-entry-point-must-be-zero-argument-create-project",
            ));
        }
        if !arguments[close + 1..].contains(':') {
            return Err(invalid("project-entry-point-definition-is-malformed"));
        }
    }
    if definitions != 1 {
        return Err(invalid(
            "project-entry-point-create-project-definition-count-invalid",
        ));
    }
    Ok(())
}

pub fn validate_user_id(value: &str) -> Result<(), PythonResearchError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid("python-research-user-id-invalid"));
    }
    Ok(())
}

pub fn validate_project_path(value: &str) -> Result<(), PythonResearchError> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || Path::new(value).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid("project-path-is-unsafe"));
    }
    Ok(())
}

fn validate_archive_path(value: &str) -> Result<String, PythonResearchError> {
    validate_project_path(value)?;
    if value.ends_with('/') || value.contains("//") {
        return Err(invalid("project-archive-path-is-not-a-file"));
    }
    Ok(value.to_owned())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), PythonResearchError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || !value.as_bytes()[0].is_ascii_lowercase()
        || value.contains("--")
        || value.ends_with('-')
    {
        return Err(invalid(format!("{label}-invalid")));
    }
    Ok(())
}

fn is_python_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && (value.as_bytes()[0].is_ascii_alphabetic() || value.as_bytes()[0] == b'_')
}

fn validate_unique_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<(), PythonResearchError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid(format!("duplicate-{label}-id")));
        }
    }
    Ok(())
}

fn validate_license(license: &str, content: &[u8]) -> Result<(), PythonResearchError> {
    if content.is_empty() || content.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(invalid("license-file-is-empty-or-too-large"));
    }
    let text = std::str::from_utf8(content).map_err(|_| invalid("license-file-must-be-utf8"))?;
    let marker = match license {
        "Apache-2.0" => text.contains("Apache License") || text.contains("Apache-2.0"),
        "MIT" => text.contains("Permission is hereby granted, free of charge"),
        "BSD-2-Clause" | "BSD-3-Clause" => {
            text.contains("Redistribution and use in source")
                && text.contains("THIS SOFTWARE IS PROVIDED")
        }
        "MPL-2.0" => text.contains("Mozilla Public License") && text.contains("2.0"),
        "LicenseRef-Proprietary" => text.contains("LicenseRef-Proprietary"),
        _ if license.starts_with("LicenseRef-") => text.contains(license),
        _ => false,
    };
    if marker {
        Ok(())
    } else {
        Err(invalid("license-declaration-does-not-match-license-file"))
    }
}

fn check_file_size(bytes: &[u8], path: &str) -> Result<(), PythonResearchError> {
    if bytes.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(invalid(format!("project-file-size-exceeded:{path}")));
    }
    Ok(())
}

fn is_allowed_project_file(path: &str) -> bool {
    path == "adaq-project.toml"
        || path == "pyproject.toml"
        || path == "pylock.toml"
        || path == "README.md"
        || path == "README.zh-CN.md"
        || path == "LICENSE"
        || (path.starts_with("src/") && path.ends_with(".py"))
}

fn list_relative_files(root: &Path) -> Result<Vec<String>, PythonResearchError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
) -> Result<(), PythonResearchError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(invalid("project-symbolic-link-not-allowed"));
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            #[cfg(unix)]
            if std::os::unix::fs::MetadataExt::nlink(&entry.metadata()?) > 1 {
                return Err(invalid("project-hard-link-not-allowed"));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| invalid(error.to_string()))?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_project_path(&relative)?;
            files.push(relative);
        } else {
            return Err(invalid("project-special-file-not-allowed"));
        }
    }
    Ok(())
}

fn source_hash(root: &Path, files: &[String]) -> Result<String, PythonResearchError> {
    let mut bytes = Vec::new();
    for path in files {
        if path.starts_with("src/") {
            bytes.extend_from_slice(path.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&fs::read(root.join(path))?);
            bytes.push(0);
        }
    }
    Ok(sha256(&bytes))
}

fn read_manifest_for_diagnostics(root: &Path) -> Option<ProjectManifest> {
    fs::read(root.join("adaq-project.toml"))
        .ok()
        .and_then(|bytes| {
            std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| toml::from_str(text).ok())
        })
}

fn read_hash(path: PathBuf) -> Result<String, PythonResearchError> {
    Ok(sha256(&fs::read(path)?))
}

fn error_code(message: &str) -> String {
    message
        .split(':')
        .next()
        .unwrap_or("invalid-python-project")
        .to_owned()
}

fn write_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), PythonResearchError> {
    for (name, content) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

fn temporary_directory(prefix: &str) -> Result<PathBuf, PythonResearchError> {
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system-clock-before-unix-epoch"))?
            .as_nanos()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    use tempfile::tempdir;

    fn write_project(root: &Path, project_id: &str, lock: &[u8]) -> ProjectManifest {
        fs::create_dir_all(root.join("src")).unwrap();
        let manifest = ProjectManifest {
            schema_version: PYTHON_RESEARCH_SCHEMA_VERSION.into(),
            project_id: project_id.into(),
            kind: ProjectKind::Factor,
            mode: Some(ProjectMode::PortableDefinition),
            scope: ProjectScope::CrossSectional,
            entry_point: "project:create_project".into(),
            sdk_profile: "adaq-research-sdk@1".into(),
            runtime_profile: "adaq-python@1".into(),
            source_files: vec!["src/project.py".into()],
            parameters: vec![ParameterSpec {
                id: "lookback".into(),
                value_type: ParameterType::Integer,
                default: "20".into(),
                allowed_values: vec!["5".into(), "20".into(), "60".into()],
            }],
            input_slots: vec![InputSlotSpec {
                id: "close".into(),
                role: "market-close".into(),
                scope: ProjectScope::CrossSectional,
                required: true,
            }],
            outputs: vec![OutputSpec {
                id: "momentum-score".into(),
                value_type: "finite-f64".into(),
                required: true,
            }],
            target: None,
            signal: Some(SignalSpec {
                id: "momentum-score".into(),
                kind: "factor".into(),
                value_scale: "raw".into(),
            }),
            adapter_id: None,
            dependency_lock_sha256: sha256(lock),
            resource_request: ResourceRequest {
                max_wall_ms: 60_000,
                max_memory_bytes: 256 * 1024 * 1024,
                max_threads: 2,
                max_input_rows: 100_000,
                max_output_rows: 100_000,
            },
            license: "Apache-2.0".into(),
        };
        fs::write(
            root.join("adaq-project.toml"),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(root.join("pyproject.toml"), "[project]\nname = 'example'\n").unwrap();
        fs::write(root.join("pylock.toml"), lock).unwrap();
        fs::write(
            root.join("src/project.py"),
            "def create_project():\n    return None\n",
        )
        .unwrap();
        fs::write(root.join("README.md"), "Synthetic Demonstration\n").unwrap();
        fs::write(root.join("LICENSE"), "Apache License, Version 2.0\n").unwrap();
        manifest
    }

    #[test]
    fn validates_exact_manifest_and_rejects_unknown_fields() {
        let lock = b"lock = true\n";
        let directory = tempdir().unwrap();
        let manifest = write_project(directory.path(), "py-factor-example", lock);
        assert_eq!(
            parse_manifest(&fs::read(directory.path().join("adaq-project.toml")).unwrap()).unwrap(),
            manifest
        );
        let mut invalid_manifest = toml::to_string(&manifest).unwrap();
        invalid_manifest.push_str("unknown = true\n");
        let error = parse_manifest(invalid_manifest.as_bytes()).unwrap_err();
        assert!(error.0.contains("unknown field"));
    }

    #[test]
    fn freeze_is_content_addressed_and_does_not_change_after_unrelated_edits() {
        let directory = tempdir().unwrap();
        write_project(directory.path(), "py-factor-example", b"lock = true\n");
        let first = freeze_revision(directory.path(), sha256(b"sdk"), None).unwrap();
        fs::write(directory.path().join("README.md"), "changed presentation\n").unwrap();
        let second_report = inspect_project(directory.path());
        assert!(second_report.valid());
        let second = freeze_revision(directory.path(), sha256(b"sdk"), None).unwrap();
        assert_ne!(first.revision_sha256, second.revision_sha256);
        assert_ne!(first.files.get("README.md"), second.files.get("README.md"));
    }

    #[test]
    fn archive_is_deterministic_and_import_is_inert_and_untrusted() {
        let source = tempdir().unwrap();
        write_project(source.path(), "py-factor-example", b"lock = true\n");
        let revision = freeze_revision(source.path(), sha256(b"sdk"), None).unwrap();
        let first = deterministic_archive(source.path(), &revision).unwrap();
        let second = deterministic_archive(source.path(), &revision).unwrap();
        assert_eq!(first, second);
        let destination = source.path().join("imported");
        let imported = import_archive(&first, &destination).unwrap();
        assert!(imported.untrusted);
        assert!(destination.join("src/project.py").is_file());
    }

    #[test]
    fn hostile_archive_paths_and_links_fail_before_copying() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("../escape", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"no").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let error = validate_archive(&bytes).unwrap_err();
        assert!(error.0.contains("project-path-is-unsafe"));
    }

    #[test]
    fn license_and_project_identity_are_explicit() {
        assert!(validate_project_id("py-model-ridge", ProjectKind::Model).is_ok());
        assert!(validate_project_id("py-factor-Bad", ProjectKind::Factor).is_err());
        assert!(validate_license("LicenseRef-Proprietary", b"LicenseRef-Proprietary\n").is_ok());
        assert!(validate_license("Apache-2.0", b"LicenseRef-Proprietary\n").is_err());
    }

    #[test]
    fn bundled_examples_are_static_valid_and_kind_specific() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/python");
        for (directory, kind, project_id) in [
            (
                "py-factor-cross-sectional-momentum",
                ProjectKind::Factor,
                "py-factor-cross-sectional-momentum",
            ),
            (
                "py-model-qlib-ridge-return",
                ProjectKind::Model,
                "py-model-qlib-ridge-return",
            ),
            (
                "py-strategy-top-n-forecast",
                ProjectKind::Strategy,
                "py-strategy-top-n-forecast",
            ),
        ] {
            let report = inspect_project(&root.join(directory));
            assert!(report.valid(), "{directory}: {:?}", report.issues);
            let manifest = report.manifest.unwrap();
            assert_eq!(manifest.kind, kind);
            assert_eq!(manifest.project_id, project_id);
            if kind == ProjectKind::Model {
                model::validate_model_manifest(&manifest).unwrap();
            }
        }
    }

    #[test]
    fn model_manifest_rejects_extra_outputs() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/python/py-model-qlib-ridge-return");
        let mut manifest = inspect_project(&root).manifest.unwrap();
        manifest.outputs.push(OutputSpec {
            id: "second-forecast".into(),
            value_type: "finite-f64".into(),
            required: true,
        });
        assert!(model::validate_model_manifest(&manifest).is_err());
        let mut manifest = inspect_project(&root).manifest.unwrap();
        manifest.adapter_id = Some("unsupported-model@1".into());
        assert!(model::validate_model_manifest(&manifest).is_err());
    }

    #[test]
    fn project_store_is_user_scoped_and_marks_source_edits_dirty() {
        let source = tempdir().unwrap();
        write_project(source.path(), "py-factor-example", b"lock = true\n");
        let storage = tempdir().unwrap();
        let store = ProjectStore::new(storage.path());
        let first = store
            .create_from_example("alice", source.path(), "py-factor-example")
            .unwrap();
        assert_eq!(first.state, WorkingCopyState::Clean);
        let path = PathBuf::from(&first.path);
        fs::write(
            path.join("src/project.py"),
            "def create_project():\n return None\n",
        )
        .unwrap();
        assert_eq!(
            store.summary("alice", &path).unwrap().state,
            WorkingCopyState::Dirty
        );
        assert!(!store.validate("alice", &first.project_id).unwrap().valid());
        assert_eq!(store.summary("alice", &path).unwrap().state, WorkingCopyState::Dirty);
        assert!(store.list("bob").unwrap().is_empty());
        assert!(
            store
                .create_from_example("alice", source.path(), "py-factor-example")
                .is_err()
        );
    }

    #[test]
    fn project_revision_and_dirty_baseline_survive_store_restart() {
        let source = tempdir().unwrap();
        write_project(source.path(), "py-factor-example", b"lock = true\n");
        let storage = tempdir().unwrap();
        let store = ProjectStore::new(storage.path());
        let copy = store
            .create_from_example("alice", source.path(), "py-factor-example")
            .unwrap();
        let revision = store
            .freeze("alice", &copy.project_id, sha256(b"sdk"), None)
            .unwrap();
        drop(store);
        let restarted = ProjectStore::new(storage.path());
        let summary = restarted.list("alice").unwrap().pop().unwrap();
        assert_eq!(summary.revision_sha256, Some(revision.revision_sha256));
        fs::write(
            PathBuf::from(&copy.path).join("src/project.py"),
            "def create_project():\n return None\n",
        )
        .unwrap();
        assert_eq!(
            restarted
                .summary("alice", Path::new(&copy.path))
                .unwrap()
                .state,
            WorkingCopyState::Dirty
        );
    }

    #[test]
    fn incompatible_metadata_requires_explicit_reset_and_preserves_source_and_exports() {
        let source = tempdir().unwrap();
        write_project(source.path(), "py-factor-example", b"lock = true\n");
        let storage = tempdir().unwrap();
        let store = ProjectStore::new(storage.path());
        let copy = store
            .create_from_example("alice", source.path(), "py-factor-example")
            .unwrap();
        let revision = store
            .freeze("alice", &copy.project_id, sha256(b"sdk"), None)
            .unwrap();
        let exported = storage.path().join("export.zip");
        fs::write(
            &exported,
            store.export("alice", &copy.project_id, &revision).unwrap(),
        )
        .unwrap();
        fs::write(
            storage.path().join(PYTHON_RESEARCH_METADATA_FILE),
            br#"{"schemaVersion":"9.0.0"}"#,
        )
        .unwrap();

        let restarted = ProjectStore::new(storage.path());
        let error = restarted.list("alice").unwrap_err();
        assert!(error.0.contains("Reset Python Research Evidence"));
        let reset = restarted.reset_python_research_evidence("alice").unwrap();
        assert!(reset.preserves_working_copies);
        assert!(reset.preserves_exported_archives);
        assert!(Path::new(&copy.path).is_dir());
        assert!(exported.is_file());
        assert_eq!(restarted.list("alice").unwrap().len(), 1);
    }
}
