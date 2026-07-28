mod cli;
mod conformance;
mod package;
mod plan;
mod runtime;
mod templates;

pub use cli::{build_project, run_cli};
pub use conformance::{component_parameters, verify_package};
pub use package::{
    ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage, FeatureSlotDefinition,
    FeatureSlotSource, MarketField, PackageError, ParameterDefinition, ParameterType,
    pack_component,
};
pub use plan::{
    EngineIdentity, FactorInstancePlanInput, FrozenFactorView, FrozenIndicatorPlan,
    FrozenSourceView, PlanIssue, PlanLoadError, PlanValidationError, validate_and_freeze,
    validate_and_freeze_with_factors,
};
pub use runtime::{ComponentParameterValue, FactorSchema, RunLimits, WasmLoader};
pub use templates::{ComponentTemplate, create_project};
