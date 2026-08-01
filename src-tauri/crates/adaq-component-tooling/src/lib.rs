mod cli;
mod conformance;
mod package;
mod plan;
mod runtime;
mod templates;

pub use cli::{build_project, run_cli};
pub use conformance::{component_parameters, verify_package};
pub use package::{
    BuiltinForecastTarget, ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage,
    FeatureSlotDefinition, FeatureSlotSource, ForecastTarget, ForecastTargetValueType,
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
pub use runtime::{ComponentParameterValue, FactorSchema, RunLimits, WasmLoader};
pub use templates::{ComponentTemplate, create_project};
