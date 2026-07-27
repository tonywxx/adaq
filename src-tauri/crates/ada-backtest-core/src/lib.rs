mod package;
mod simulation;
mod snapshot;

pub use package::{
    ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage, PackageError,
    ParameterDefinition, ParameterType, pack_component,
};
pub use simulation::{
    BacktestMetrics, EquityPoint, ExecutionProfile, Fill, FillPolicy, OrderSide, OrderStatus,
    SimulatedOrder, SimulationError, SimulationResult, SpotSimulator, TargetDecision,
};
pub use snapshot::{MarketDataSnapshot, SnapshotError, SnapshotStore};
