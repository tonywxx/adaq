use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use adaq_component_tooling::{
    ComponentKind, ComponentPackage, ComponentParameterValue, FactorScope as PackageFactorScope,
    RunLimits, verify_package,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CandidateBuildProvenance, ContractError, DeclarativeFactorDefinition, FACTOR_ABI_VERSION,
    FactorCandidate, FactorCandidateDraft, FactorCandidateSource, FactorFeatureSlot, FactorOutput,
    FactorParameter, FactorResourcePolicy, FactorScope, is_lower_kebab, is_sha256,
};

const FIXED_BUILD_COMMANDS: &[&str] = &[
    "cargo test --offline --locked",
    "rustup run stable cargo component build --offline --locked --release --target wasm32-unknown-unknown",
];
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorPresentationMetadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl FactorPresentationMetadata {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.name.trim().is_empty() || self.tags.iter().any(|tag| !is_lower_kebab(tag)) {
            return Err(ContractError::Invalid(
                "Factor presentation metadata must have a name and lower-kebab tags".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorPresentationRecord {
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub metadata: FactorPresentationMetadata,
}

impl FactorPresentationRecord {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.user_id.is_nil() || !is_sha256(&self.candidate_hash) {
            return Err(ContractError::Invalid(
                "Factor presentation record identity is invalid".into(),
            ));
        }
        self.metadata.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarativeFactorDraft {
    pub user_id: Uuid,
    pub candidate_id: Uuid,
    pub revision: u64,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub definition: DeclarativeFactorDefinition,
    pub presentation: FactorPresentationMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomFactorDraft {
    pub user_id: Uuid,
    pub candidate_id: Uuid,
    pub revision: u64,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub build: CandidateBuildProvenance,
    pub presentation: FactorPresentationMetadata,
}

impl CustomFactorDraft {
    pub fn publish(self) -> Result<(FactorCandidate, FactorPresentationRecord), ContractError> {
        self.presentation.validate()?;
        if self.user_id.is_nil() {
            return Err(ContractError::Invalid(
                "Custom Factor User identity is invalid".into(),
            ));
        }
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: self.candidate_id,
            revision: self.revision,
            scope: self.scope,
            feature_slots: self.feature_slots,
            parameters: self.parameters,
            outputs: self.outputs,
            source: FactorCandidateSource::Custom { build: self.build },
        })?;
        let presentation = FactorPresentationRecord {
            user_id: self.user_id,
            candidate_hash: candidate.candidate_hash.clone(),
            metadata: self.presentation,
        };
        Ok((candidate, presentation))
    }
}

impl DeclarativeFactorDraft {
    pub fn publish(self) -> Result<(FactorCandidate, FactorPresentationRecord), ContractError> {
        self.presentation.validate()?;
        if self.user_id.is_nil() {
            return Err(ContractError::Invalid(
                "Declarative Factor User identity is invalid".into(),
            ));
        }
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: self.candidate_id,
            revision: self.revision,
            scope: self.scope,
            feature_slots: self.feature_slots,
            parameters: self.parameters,
            outputs: self.outputs,
            source: FactorCandidateSource::Declarative {
                definition: self.definition,
            },
        })?;
        let presentation = FactorPresentationRecord {
            user_id: self.user_id,
            candidate_hash: candidate.candidate_hash.clone(),
            metadata: self.presentation,
        };
        Ok((candidate, presentation))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateBuildRequest {
    pub attempt_id: Uuid,
    pub user_id: Uuid,
    pub project_root: PathBuf,
    pub source_sha256: String,
    pub sdk_version: String,
    pub toolchain: String,
    pub target: String,
    pub resource_policy: FactorResourcePolicy,
}

impl CandidateBuildRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.attempt_id.is_nil()
            || self.user_id.is_nil()
            || !self.project_root.is_dir()
            || !is_sha256(&self.source_sha256)
            || self.sdk_version.is_empty()
            || self.toolchain != "stable"
            || self.target != "wasm32-unknown-unknown"
            || self.resource_policy.fuel_per_call == 0
            || self.resource_policy.memory_bytes == 0
        {
            return Err(ContractError::Invalid(
                "Custom Candidate Build request is incomplete or unsafe".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateBuildStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl CandidateBuildStatus {
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateBuildAttempt {
    pub attempt_id: Uuid,
    pub user_id: Uuid,
    pub status: CandidateBuildStatus,
    pub source_attempt_id: Option<Uuid>,
    pub package_sha256: Option<String>,
    pub diagnostic: Option<String>,
}

impl CandidateBuildAttempt {
    pub fn new(attempt_id: Uuid, user_id: Uuid) -> Result<Self, ContractError> {
        if attempt_id.is_nil() || user_id.is_nil() {
            return Err(ContractError::Invalid(
                "Candidate Build Attempt identity is invalid".into(),
            ));
        }
        Ok(Self {
            attempt_id,
            user_id,
            status: CandidateBuildStatus::Pending,
            source_attempt_id: None,
            package_sha256: None,
            diagnostic: None,
        })
    }

    pub fn transition(&mut self, next: CandidateBuildStatus) -> Result<(), ContractError> {
        if !matches!(
            (self.status, next),
            (
                CandidateBuildStatus::Pending,
                CandidateBuildStatus::Running
                    | CandidateBuildStatus::Failed
                    | CandidateBuildStatus::Cancelled
            ) | (
                CandidateBuildStatus::Running,
                CandidateBuildStatus::Completed
                    | CandidateBuildStatus::Failed
                    | CandidateBuildStatus::Cancelled
            )
        ) {
            return Err(ContractError::Invalid(
                "invalid Candidate Build Attempt transition".into(),
            ));
        }
        self.status = next;
        Ok(())
    }

    pub fn retry(&self, attempt_id: Uuid) -> Result<Self, ContractError> {
        if !matches!(
            self.status,
            CandidateBuildStatus::Failed | CandidateBuildStatus::Cancelled
        ) || attempt_id.is_nil()
        {
            return Err(ContractError::Invalid(
                "only a terminal Candidate Build Attempt can be retried".into(),
            ));
        }
        Ok(Self {
            attempt_id,
            user_id: self.user_id,
            status: CandidateBuildStatus::Pending,
            source_attempt_id: Some(self.attempt_id),
            package_sha256: None,
            diagnostic: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateBuildResult {
    pub package: ComponentPackage,
    pub package_bytes: Vec<u8>,
    pub provenance: CandidateBuildProvenance,
    pub diagnostics: String,
}

pub struct CandidateBuildWorker {
    pub attempt: CandidateBuildAttempt,
    cancelled: Arc<AtomicBool>,
    handle: JoinHandle<Result<CandidateBuildResult, String>>,
}

impl CandidateBuildWorker {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn join(self) -> CandidateBuildAttemptResult {
        let mut attempt = self.attempt;
        let was_cancelled = self.cancelled.load(Ordering::Acquire);
        let result = self
            .handle
            .join()
            .unwrap_or_else(|_| Err("candidate-build-worker-panicked".into()));
        match result {
            Ok(result) if !was_cancelled && !self.cancelled.load(Ordering::Acquire) => {
                let _ = attempt.transition(CandidateBuildStatus::Completed);
                attempt.package_sha256 = Some(result.provenance.package_sha256.clone());
                attempt.diagnostic = Some(safe_diagnostic(&result.diagnostics));
                CandidateBuildAttemptResult {
                    attempt,
                    result: Some(result),
                }
            }
            Ok(_) => {
                let _ = attempt.transition(CandidateBuildStatus::Cancelled);
                CandidateBuildAttemptResult {
                    attempt,
                    result: None,
                }
            }
            Err(_error) if was_cancelled || self.cancelled.load(Ordering::Acquire) => {
                let _ = attempt.transition(CandidateBuildStatus::Cancelled);
                attempt.diagnostic = Some("candidate-build-cancelled".into());
                CandidateBuildAttemptResult {
                    attempt,
                    result: None,
                }
            }
            Err(error) => {
                let _ = attempt.transition(CandidateBuildStatus::Failed);
                attempt.diagnostic = Some(safe_diagnostic(&error));
                CandidateBuildAttemptResult {
                    attempt,
                    result: None,
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateBuildAttemptResult {
    pub attempt: CandidateBuildAttempt,
    pub result: Option<CandidateBuildResult>,
}

pub fn spawn_controlled_candidate_build(
    request: CandidateBuildRequest,
) -> Result<CandidateBuildWorker, ContractError> {
    request.validate()?;
    let attempt = CandidateBuildAttempt::new(request.attempt_id, request.user_id)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let handle = thread::Builder::new()
        .name("adaq-factor-candidate-build".into())
        .spawn(move || {
            if worker_cancelled.load(Ordering::Acquire) {
                return Err("candidate-build-cancelled".into());
            }
            build_candidate_package(&request)
        })
        .map_err(|error| ContractError::Invalid(format!("cannot start build worker: {error}")))?;
    let mut attempt = attempt;
    attempt.transition(CandidateBuildStatus::Running)?;
    Ok(CandidateBuildWorker {
        attempt,
        cancelled,
        handle,
    })
}

fn build_candidate_package(
    request: &CandidateBuildRequest,
) -> Result<CandidateBuildResult, String> {
    let source_sha256 = project_source_sha256(&request.project_root)?;
    if source_sha256 != request.source_sha256 {
        return Err("Custom Candidate source hash does not match the requested project".into());
    }
    reject_custom_build_scripts(&request.project_root)?;
    reject_custom_ambient_capabilities(&request.project_root)?;
    let build =
        adaq_component_tooling::build_project_offline_with_diagnostics(&request.project_root)?;
    let package_path = build.package_path;
    let bytes = fs::read(&package_path).map_err(|error| error.to_string())?;
    let package = ComponentPackage::read(&bytes).map_err(|error| error.to_string())?;
    verify_package(&package)?;
    let compiler = compiler_identity()?;
    if package.manifest.kind != ComponentKind::Factor
        || package.manifest.abi_version.to_string() != FACTOR_ABI_VERSION
        || package.manifest.sdk_version.to_string() != request.sdk_version
        || !matches!(
            (package.manifest.factor_scope, request.target.as_str()),
            (
                Some(PackageFactorScope::TimeSeries | PackageFactorScope::CrossSectional),
                "wasm32-unknown-unknown"
            )
        )
    {
        return Err("custom build did not produce a Factor ABI v2 package".into());
    }
    let provenance = CandidateBuildProvenance {
        attempt_id: request.attempt_id,
        source_sha256: request.source_sha256.clone(),
        sdk_version: request.sdk_version.clone(),
        abi_version: FACTOR_ABI_VERSION.into(),
        toolchain: request.toolchain.clone(),
        compiler,
        target: request.target.clone(),
        commands: FIXED_BUILD_COMMANDS
            .iter()
            .map(|command| (*command).into())
            .collect(),
        environment: BTreeMap::from([(
            String::from("network"),
            String::from("disabled: offline Cargo and ambient-capability preflight"),
        )]),
        resource_policy: request.resource_policy,
        diagnostic_log_sha256: Some(adaq_feature_engine::sha256(build.diagnostics.as_bytes())),
        package_sha256: package.archive_sha256.clone(),
    };
    Ok(CandidateBuildResult {
        package,
        package_bytes: bytes,
        provenance,
        diagnostics: build.diagnostics,
    })
}

fn compiler_identity() -> Result<String, String> {
    let output = Command::new("rustup")
        .args(["run", "stable", "rustc", "--version", "--verbose"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "stable rustc identity failed with {}",
            output.status
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn reject_custom_build_scripts(root: &std::path::Path) -> Result<(), String> {
    let cargo_toml =
        std::fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    if cargo_toml
        .lines()
        .any(|line| line.trim_start().starts_with("build ="))
    {
        return Err("Custom Candidate projects may not declare Cargo build scripts".into());
    }
    if has_file_named(root, "build.rs")? {
        return Err("Custom Candidate projects may not contain build.rs scripts".into());
    }
    Ok(())
}

fn reject_custom_ambient_capabilities(root: &std::path::Path) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &[
        "std::net",
        "std::process",
        "std::fs",
        "std::env",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "ureq",
        "hyper",
    ];
    let mut files = Vec::new();
    collect_source_files(root, root, &mut files)?;
    if files.iter().any(|(path, contents)| {
        path.ends_with(".rs")
            && String::from_utf8_lossy(contents)
                .lines()
                .any(|line| FORBIDDEN.iter().any(|token| line.contains(token)))
    }) {
        return Err(
            "Custom Factor projects may not use ambient filesystem, process, environment, or network capabilities".into(),
        );
    }
    Ok(())
}

fn has_file_named(directory: &std::path::Path, filename: &str) -> Result<bool, String> {
    for entry in std::fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
    {
        let path = entry.path();
        if entry.file_name() == filename {
            return Ok(true);
        }
        if path.is_dir()
            && !matches!(entry.file_name().to_str(), Some("target" | "dist" | ".git"))
            && has_file_named(&path, filename)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn project_source_sha256(root: &std::path::Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_source_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut bytes = Vec::new();
    for (path, contents) in files {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&contents);
        bytes.push(0);
    }
    Ok(adaq_feature_engine::sha256(&bytes))
}

fn collect_source_files(
    root: &std::path::Path,
    directory: &std::path::Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if matches!(name.to_str(), Some("target" | "dist" | ".git")) {
            continue;
        }
        if path.is_dir() {
            collect_source_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            files.push((
                relative,
                std::fs::read(path).map_err(|error| error.to_string())?,
            ));
        }
    }
    Ok(())
}

pub fn component_parameter_values(
    values: &[crate::FactorParameterValue],
) -> Vec<ComponentParameterValue> {
    values
        .iter()
        .map(|value| match value {
            crate::FactorParameterValue::Decimal(value) => {
                ComponentParameterValue::Decimal(value.clone())
            }
            crate::FactorParameterValue::Integer(value) => ComponentParameterValue::Integer(*value),
            crate::FactorParameterValue::Boolean(value) => ComponentParameterValue::Boolean(*value),
            crate::FactorParameterValue::Text(value) => {
                ComponentParameterValue::String(value.clone())
            }
        })
        .collect()
}

pub fn run_limits(policy: FactorResourcePolicy) -> Result<RunLimits, ContractError> {
    let memory_bytes = usize::try_from(policy.memory_bytes)
        .map_err(|_| ContractError::Invalid("Factor memory policy does not fit the host".into()))?;
    Ok(RunLimits {
        fuel_per_call: policy.fuel_per_call,
        memory_bytes,
        max_bars: 1_000_000,
    })
}

pub(crate) fn safe_diagnostic(message: &str) -> String {
    let mut diagnostic = message
        .lines()
        .map(|line| {
            line.replace("CARGO_HOME", "<private>")
                .replace("HOME", "<private>")
                .replace("/Users/", "<private>/")
                .replace("/home/", "<private>/")
                .replace("C:\\Users\\", "<private>\\")
        })
        .collect::<Vec<_>>()
        .join("\n");
    if diagnostic.len() > MAX_DIAGNOSTIC_BYTES {
        diagnostic.truncate(MAX_DIAGNOSTIC_BYTES);
        diagnostic.push_str("…");
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_metadata_is_outside_candidate_semantic_hash() {
        let base = DeclarativeFactorDraft {
            user_id: Uuid::new_v4(),
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![FactorFeatureSlot {
                name: "signal".into(),
            }],
            parameters: vec![],
            outputs: vec![FactorOutput {
                name: "signal".into(),
            }],
            definition: DeclarativeFactorDefinition {
                feature_plan_hash: "a".repeat(64),
                operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                    .into(),
                outputs: vec![crate::DeclarativeFactorOutputBinding {
                    output_name: "signal".into(),
                    feature_slot: "signal".into(),
                }],
            },
            presentation: FactorPresentationMetadata {
                name: "First".into(),
                description: "one".into(),
                tags: vec!["trend".into()],
            },
        };
        let (first, _) = base.clone().publish().unwrap();
        let mut second_draft = base;
        second_draft.presentation.name = "Second".into();
        second_draft.presentation.description = "two".into();
        let (second, _) = second_draft.publish().unwrap();
        assert_eq!(first.candidate_hash, second.candidate_hash);
    }

    #[test]
    fn build_attempts_are_retryable_and_diagnostics_are_bounded() {
        let mut attempt = CandidateBuildAttempt::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
        attempt.transition(CandidateBuildStatus::Running).unwrap();
        attempt.transition(CandidateBuildStatus::Failed).unwrap();
        let retry = attempt.retry(Uuid::new_v4()).unwrap();
        assert_eq!(retry.source_attempt_id, Some(attempt.attempt_id));
        assert!(
            safe_diagnostic(&"x".repeat(MAX_DIAGNOSTIC_BYTES + 20)).len()
                <= MAX_DIAGNOSTIC_BYTES + 3
        );
    }

    #[test]
    fn controlled_build_records_attempt_bound_provenance_and_logs() {
        let project_root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/factor");
        let attempt_id = Uuid::new_v4();
        let worker = spawn_controlled_candidate_build(CandidateBuildRequest {
            attempt_id,
            user_id: Uuid::new_v4(),
            source_sha256: project_source_sha256(&project_root).unwrap(),
            project_root,
            sdk_version: "0.1.0".into(),
            toolchain: "stable".into(),
            target: "wasm32-unknown-unknown".into(),
            resource_policy: FactorResourcePolicy {
                fuel_per_call: 1_000_000,
                memory_bytes: 64 * 1024 * 1024,
            },
        })
        .unwrap();
        let result = worker.join();
        assert_eq!(result.attempt.status, CandidateBuildStatus::Completed);
        assert_eq!(
            result.result.as_ref().unwrap().provenance.attempt_id,
            attempt_id
        );
        assert!(result.attempt.diagnostic.is_some());
    }
}
