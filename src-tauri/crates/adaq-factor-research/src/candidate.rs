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
    ComponentKind, ComponentManifest, ComponentPackage, ComponentParameterValue, ComponentTemplate,
    FactorScope as PackageFactorScope, FeatureSlotDefinition, FeatureSlotSource, ModelOutput,
    ParameterDefinition, ParameterType, RunLimits, create_project, verify_package,
};
use adaq_feature_engine::{
    FeatureEngineIdentity, FeaturePlan, FeatureSource, FrozenBuiltInParameter,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CandidateBuildProvenance, ContractError, DeclarativeFactorDefinition, FACTOR_ABI_VERSION,
    FactorCandidate, FactorCandidateDraft, FactorCandidateSource, FactorFeatureSlot, FactorOutput,
    FactorParameter, FactorResourcePolicy, FactorScope, PythonFactorBinding, is_lower_kebab,
    is_sha256,
};

const FIXED_BUILD_COMMANDS: &[&str] = &[
    "cargo test --offline --locked",
    "rustup run stable cargo component build --offline --locked --release --target wasm32-unknown-unknown",
];
pub const DECLARATIVE_GENERATOR_ID: &str = "adaq-factor-rust-sdk-generator@1";
pub const DECLARATIVE_BUILD_COMMANDS: &[&str] = &[
    "cargo generate-lockfile --offline",
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonFactorDraft {
    pub user_id: Uuid,
    pub candidate_id: Uuid,
    pub revision: u64,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub binding: PythonFactorBinding,
    pub presentation: FactorPresentationMetadata,
}

impl PythonFactorDraft {
    pub fn publish(self) -> Result<(FactorCandidate, FactorPresentationRecord), ContractError> {
        self.presentation.validate()?;
        if self.user_id.is_nil() {
            return Err(ContractError::Invalid(
                "Python Factor User identity is invalid".into(),
            ));
        }
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: self.candidate_id,
            revision: self.revision,
            scope: self.scope,
            feature_slots: self.feature_slots,
            parameters: self.parameters,
            outputs: self.outputs,
            source: FactorCandidateSource::Python {
                binding: self.binding,
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

pub fn generate_declarative_candidate_package(
    attempt_id: Uuid,
    user_id: Uuid,
    candidate: &FactorCandidate,
    component_name: &str,
    plan_json: &[u8],
    feature_engine_identity: &FeatureEngineIdentity,
    resource_policy: FactorResourcePolicy,
) -> Result<CandidateBuildResult, String> {
    if attempt_id.is_nil() || user_id.is_nil() || component_name.trim().is_empty() {
        return Err("Declarative Candidate generator identity is invalid".into());
    }
    candidate.validate().map_err(string)?;
    let definition = match &candidate.source {
        FactorCandidateSource::Declarative { definition } => definition,
        FactorCandidateSource::Custom { .. } | FactorCandidateSource::Python { .. } => {
            return Err("only Declarative Factors use the Host SDK generator".into());
        }
    };
    let plan = FeaturePlan::load_for_engine(plan_json, feature_engine_identity)
        .map_err(|error| format!("Feature Plan evidence is invalid: {error}"))?;
    if definition.feature_plan_hash != plan.plan_hash() {
        return Err("Declarative Factor Feature Plan hash differs from frozen evidence".into());
    }
    let manifest = ComponentManifest {
        manifest_schema_version: Version::new(1, 0, 0),
        component_id: candidate.candidate_id,
        version: Version::new(0, 1, 0),
        name: component_name.to_owned(),
        kind: ComponentKind::Factor,
        factor_scope: Some(match candidate.scope {
            FactorScope::TimeSeries => PackageFactorScope::TimeSeries,
            FactorScope::CrossSectional => PackageFactorScope::CrossSectional,
        }),
        sdk_version: Version::parse(adaq_component_sdk::SDK_VERSION).map_err(string)?,
        abi_version: Version::parse(adaq_component_sdk::FACTOR_ABI_VERSION).map_err(string)?,
        wasm_sha256: String::new(),
        parameters: candidate
            .parameters
            .iter()
            .map(manifest_parameter)
            .collect(),
        feature_slots: candidate
            .feature_slots
            .iter()
            .map(|slot| {
                let plan_slot = plan
                    .slots()
                    .iter()
                    .find(|plan_slot| plan_slot.name == slot.name)
                    .ok_or_else(|| {
                        format!(
                            "Declarative Factor Feature Slot {} is absent from frozen Plan",
                            slot.name
                        )
                    })?;
                Ok(FeatureSlotDefinition {
                    name: slot.name.clone(),
                    source: manifest_slot_source(&plan_slot.source)?,
                })
            })
            .collect::<Result<_, String>>()?,
        output_names: candidate
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect(),
        dependencies: Vec::new(),
        warmup_bars: plan.effective_warmup_bars(),
        model_scope: None,
        model_outputs: Vec::new(),
        model_artifact: None,
    };
    let output_slots = candidate
        .outputs
        .iter()
        .map(|output| {
            let binding = definition
                .outputs
                .iter()
                .find(|binding| binding.output_name == output.name)
                .ok_or_else(|| format!("Declarative Factor output {} is not bound", output.name))?;
            let index = candidate
                .feature_slots
                .iter()
                .position(|slot| slot.name == binding.feature_slot)
                .ok_or_else(|| {
                    format!(
                        "Declarative Factor output {} references an unknown Feature Slot",
                        output.name
                    )
                })?;
            Ok((output.name.clone(), index))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let source =
        render_declarative_factor_source(candidate, &output_slots, plan.effective_warmup_bars());
    let definition_json = crate::canonical_json(definition).map_err(string)?;
    let source_sha256 = adaq_feature_engine::sha256(&definition_json);
    let project_name = format!("adaq-factor-{}", attempt_id.simple());
    let local_sdk_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../adaq-component-sdk");
    let sdk_path = local_sdk_path.is_dir().then_some(local_sdk_path);
    let project = create_project(
        ComponentTemplate::Factor,
        &project_name,
        &std::env::temp_dir(),
        sdk_path.as_deref(),
    )?;
    let result = (|| {
        fs::write(project.join("src/lib.rs"), source).map_err(string)?;
        fs::write(
            project.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(string)?,
        )
        .map_err(string)?;
        let lock_diagnostics = generate_lockfile(&project)?;
        let build = adaq_component_tooling::build_project_offline_with_diagnostics(&project)?;
        let bytes = fs::read(&build.package_path).map_err(string)?;
        let package = ComponentPackage::read(&bytes).map_err(string)?;
        verify_package(&package)?;
        if package.manifest.component_id != candidate.candidate_id
            || package.manifest.factor_scope != manifest.factor_scope
            || package.manifest.feature_slots != manifest.feature_slots
            || package.manifest.output_names != manifest.output_names
            || package.manifest.warmup_bars != manifest.warmup_bars
        {
            return Err("generated Factor package does not match its frozen contract".into());
        }
        let diagnostics = [lock_diagnostics, build.diagnostics]
            .into_iter()
            .filter(|diagnostic| !diagnostic.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let provenance = CandidateBuildProvenance {
            attempt_id,
            source_sha256,
            sdk_version: adaq_component_sdk::SDK_VERSION.into(),
            abi_version: FACTOR_ABI_VERSION.into(),
            toolchain: "stable".into(),
            compiler: compiler_identity()?,
            target: "wasm32-unknown-unknown".into(),
            commands: DECLARATIVE_BUILD_COMMANDS
                .iter()
                .map(|command| (*command).into())
                .collect(),
            environment: BTreeMap::from([
                ("generator".into(), DECLARATIVE_GENERATOR_ID.into()),
                ("network".into(), "disabled: offline Cargo".into()),
            ]),
            resource_policy,
            diagnostic_log_sha256: Some(adaq_feature_engine::sha256(diagnostics.as_bytes())),
            package_sha256: package.archive_sha256.clone(),
        };
        Ok(CandidateBuildResult {
            package,
            package_bytes: bytes,
            provenance,
            diagnostics,
        })
    })();
    let _ = fs::remove_dir_all(&project);
    result
}

fn generate_lockfile(root: &std::path::Path) -> Result<String, String> {
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .env("CARGO_NET_OFFLINE", "true")
        .current_dir(root)
        .output()
        .map_err(string)?;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(diagnostics)
    } else {
        Err(format!(
            "cargo generate-lockfile --offline failed with {}: {}",
            output.status,
            safe_diagnostic(&diagnostics)
        ))
    }
}

fn manifest_parameter(parameter: &FactorParameter) -> ParameterDefinition {
    ParameterDefinition {
        name: parameter.name.clone(),
        parameter_type: match parameter.parameter_type {
            crate::FactorParameterType::Decimal => ParameterType::Decimal,
            crate::FactorParameterType::Integer => ParameterType::Integer,
            crate::FactorParameterType::Boolean => ParameterType::Boolean,
            crate::FactorParameterType::Text => ParameterType::String,
        },
        default_value: parameter.default_value.clone(),
        allowed_values: parameter.allowed_values.clone(),
    }
}

fn manifest_slot_source(source: &FeatureSource) -> Result<FeatureSlotSource, String> {
    match source {
        FeatureSource::Market { field } => Ok(FeatureSlotSource::Market { field: *field }),
        FeatureSource::External {
            dependency_alias,
            output,
        } => Ok(FeatureSlotSource::External {
            dependency_alias: dependency_alias.clone(),
            output: output.clone(),
        }),
        FeatureSource::BuiltIn {
            indicator,
            output,
            real_inputs,
            parameters,
        } => Ok(FeatureSlotSource::BuiltIn {
            indicator: indicator.clone(),
            output: output.clone(),
            inputs: real_inputs
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    Ok((
                        format!("real-{index}"),
                        serde_json::to_value(field).map_err(string)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
            parameters: parameters
                .iter()
                .map(|(name, value)| {
                    let value = match value {
                        FrozenBuiltInParameter::Integer(value) => serde_json::json!(value),
                        FrozenBuiltInParameter::Real(value)
                        | FrozenBuiltInParameter::Enum(value) => serde_json::json!(value),
                    };
                    (name.clone(), value)
                })
                .collect(),
        }),
        FeatureSource::Signal { contract, .. } => {
            let output: ModelOutput = serde_json::from_value(contract.clone()).map_err(string)?;
            Ok(FeatureSlotSource::Signal {
                prediction_kind: output.prediction_kind,
                forecast_target: output.forecast_target,
                value_scale: output.value_scale,
                horizon_bars: output.horizon_bars,
            })
        }
    }
}

fn rust_string(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(string)
}

fn render_declarative_factor_source(
    candidate: &FactorCandidate,
    output_slots: &[(String, usize)],
    warmup_bars: u32,
) -> String {
    let feature_slots = candidate
        .feature_slots
        .iter()
        .map(|slot| {
            format!(
                "FeatureSlot {{ name: {}.into() }}",
                rust_string(&slot.name).expect("validated slot names serialize")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let parameters = candidate
        .parameters
        .iter()
        .map(|parameter| {
            let definition = manifest_parameter(parameter);
            let parameter_type = match definition.parameter_type {
                ParameterType::Decimal => "Decimal",
                ParameterType::Integer => "Integer",
                ParameterType::Boolean => "Boolean",
                ParameterType::String => "Text",
            };
            format!(
                "ParameterDefinition {{ name: {}.into(), parameter_type: ParameterType::{}, default_value: {}.into() }}",
                rust_string(&parameter.name).expect("validated parameter names serialize"),
                parameter_type,
                rust_string(&parameter.default_value).expect("validated defaults serialize")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let output_names = candidate
        .outputs
        .iter()
        .map(|output| {
            format!(
                "{}.into()",
                rust_string(&output.name).expect("validated output names serialize")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let (module, row_type, scope, cell_import) = match candidate.scope {
        FactorScope::TimeSeries => ("time_series", "TimeSeriesRow", "TimeSeries", ""),
        FactorScope::CrossSectional => (
            "cross_sectional",
            "CrossSectionalRow",
            "CrossSectional",
            "FeatureCell, ",
        ),
    };
    let mut source = format!(
        "use adaq_component_sdk::factor::{module}::{{FactorResult, FactorSchema, FactorScope, {cell_import}FeatureSlot, Guest, GuestInstance, Instance as FactorInstance, NamedScalar, ParameterDefinition, ParameterType, ParameterValue, {row_type}}};\n\nstruct Component;\nstruct Instance;\n\nimpl Guest for Component {{\n    type Instance = Instance;\n\n    fn describe() -> Result<FactorSchema, String> {{\n        Ok(FactorSchema {{\n            scope: FactorScope::{scope},\n            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),\n            feature_slots: vec![{feature_slots}],\n            parameters: vec![{parameters}],\n            output_names: vec![{output_names}],\n            warmup_bars: {warmup_bars},\n        }})\n    }}\n\n    fn create(\n        feature_slots: Vec<FeatureSlot>,\n        parameters: Vec<ParameterValue>,\n    ) -> Result<FactorInstance, String> {{\n        if feature_slots.len() != {slot_count} || parameters.len() != {parameter_count} {{\n            return Err(\"generated Factor contract length mismatch\".into());\n        }}\n        Ok(FactorInstance::new(Instance))\n    }}\n}}\n\nimpl GuestInstance for Instance {{\n    fn process(&self, rows: Vec<{row_type}>) -> Result<Vec<FactorResult>, String> {{\n        rows.into_iter()\n            .map(|row| {{\n                let values = ",
        module = module,
        row_type = row_type,
        scope = scope,
        cell_import = cell_import,
        feature_slots = feature_slots,
        parameters = parameters,
        output_names = output_names,
        warmup_bars = warmup_bars,
        slot_count = candidate.feature_slots.len(),
        parameter_count = candidate.parameters.len(),
    );
    if candidate.scope == FactorScope::TimeSeries {
        source.push_str("vec![\n");
        for (output, index) in output_slots {
            source.push_str(&format!(
                "                    NamedScalar {{ name: {}.into(), value: row.slots[{}].value }},\n",
                rust_string(output).expect("validated output names serialize"),
                index
            ));
        }
        source.push_str(
            "];\n                Ok(FactorResult { instrument_id: row.instrument_id, observation_time_ms: row.observation_time_ms, values: Some(values) })\n            })\n            .collect()\n    }\n}\n\nadaq_component_sdk::factor::time_series::bindings::export_factor!(\n    Component with_types_in adaq_component_sdk::factor::time_series::bindings\n);\n",
        );
    } else {
        source.push_str("[\n");
        for (output, index) in output_slots {
            source.push_str(&format!(
                "                    match row.slots.get({}) {{ Some(FeatureCell::Available(value)) => Some(NamedScalar {{ name: {}.into(), value: value.value }}), Some(FeatureCell::Unavailable(_)) | None => None }},\n",
                index,
                rust_string(output).expect("validated output names serialize")
            ));
        }
        source.push_str(
            "].into_iter().collect::<Option<Vec<_>>>();\n                Ok(FactorResult { instrument_id: row.instrument_id, observation_time_ms: row.observation_time_ms, values })\n            })\n            .collect()\n    }\n}\n\nadaq_component_sdk::factor::cross_sectional::bindings::export_factor!(\n    Component with_types_in adaq_component_sdk::factor::cross_sectional::bindings\n);\n",
        );
    }
    source
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

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
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
        assert_eq!(
            result.attempt.status,
            CandidateBuildStatus::Completed,
            "candidate build diagnostic: {:?}",
            result.attempt.diagnostic
        );
        assert_eq!(
            result.result.as_ref().unwrap().provenance.attempt_id,
            attempt_id
        );
        assert!(result.attempt.diagnostic.is_some());
    }

    #[test]
    fn declarative_generator_freezes_sdk_identity_and_output_contract() {
        let engine_identity = FeatureEngineIdentity::for_tests();
        let plan = FeaturePlan::freeze(adaq_feature_engine::FeaturePlanDraft {
            slots: vec![adaq_feature_engine::FeatureSlot {
                name: "close".into(),
                source: FeatureSource::Market {
                    field: adaq_feature_engine::MarketField::Close,
                },
                warmup_bars: 0,
            }],
            engine_identity: engine_identity.clone(),
            ..Default::default()
        })
        .unwrap();
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: Vec::new(),
            outputs: vec![FactorOutput {
                name: "close".into(),
            }],
            source: FactorCandidateSource::Declarative {
                definition: DeclarativeFactorDefinition {
                    feature_plan_hash: plan.plan_hash().into(),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    outputs: vec![crate::DeclarativeFactorOutputBinding {
                        output_name: "close".into(),
                        feature_slot: "close".into(),
                    }],
                },
            },
        })
        .unwrap();
        let result = generate_declarative_candidate_package(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &candidate,
            "close-factor",
            &plan.to_json(),
            &engine_identity,
            FactorResourcePolicy {
                fuel_per_call: 1_000_000,
                memory_bytes: 64 * 1024 * 1024,
            },
        )
        .unwrap();
        assert_eq!(result.package.manifest.component_id, candidate.candidate_id);
        assert_eq!(result.package.manifest.output_names, vec!["close"]);
        assert_eq!(
            result.provenance.environment.get("generator"),
            Some(&DECLARATIVE_GENERATOR_ID.to_owned())
        );
        assert!(
            result
                .provenance
                .commands
                .iter()
                .any(|command| command.contains("cargo component build"))
        );
    }
}
