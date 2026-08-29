mod cli;
mod conformance;
mod model_export;
mod model_template;
mod package;
mod plan;
mod qualification;
mod runtime;
mod templates;

pub use cli::{
    ComponentBuildOutput, build_project, build_project_offline_with_diagnostics, run_cli,
};
pub use conformance::{component_parameters, verify_package};
pub use model_export::{
    MODEL_EXPORTER_ID, MODEL_HORIZON_BARS, MODEL_OUTPUT_NAME, MODEL_TARGET_ID, WASI_MODEL_PROFILE,
    export_linear_model_component, linear_model_binding,
};
pub use package::{
    BuiltinForecastTarget, ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage,
    FactorScope, FeatureSlotDefinition, FeatureSlotSource, ForecastTarget, ForecastTargetValueType,
    ForecastValueScale, MarketField, ModelArtifact, ModelOutput, ModelScope, PackageError,
    ParameterDefinition, ParameterType, PredictionKind, StrategyArchitecture,
    check_manifest_compatibility, pack_component, strategy_architecture, validate_model_outputs,
};
pub use plan::{
    EngineIdentity, FactorInstancePlanInput, FrozenBuiltInParameter, FrozenFactorView,
    FrozenFeaturePlan, FrozenSourceView, PlanIssue, PlanLoadError, PlanValidationError,
    SignalPlanInput, builtin_engine_market_field, native_engine_identity,
    validate_and_freeze_feature_plan,
    validate_and_freeze_feature_plan_with_bindings_and_parameters,
    validate_and_freeze_feature_plan_with_factors,
    validate_and_freeze_feature_plan_with_factors_and_parameters,
};
pub use qualification::{
    QualificationAttempt, QualificationEvidence, QualificationGate, qualify_package,
    qualify_package_with_limits,
};
pub use runtime::{
    ComponentParameterValue, FactorParameterSchema, FactorSchema, RunLimits, WasmLoader,
};
pub use templates::{ComponentTemplate, create_project};
