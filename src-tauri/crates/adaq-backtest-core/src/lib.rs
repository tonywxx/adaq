mod portfolio;
mod simulation;
mod snapshot;

pub use portfolio::{
    ApprovedPortfolioTarget, Attribution, BacktestDecision, BacktestError, BacktestEvidence,
    ExecutionPlan, PortfolioBacktestRequest, PortfolioMarketDecision, PortfolioOrder,
    PortfolioPosition, PortfolioState, PortfolioTarget, RiskDecision, RiskPolicy, StrategyTarget,
    TopNForecastStrategy, execute_portfolio_backtest,
};

pub use simulation::{
    BacktestMetrics, EquityPoint, ExecutionProfile, Fill, FillPolicy, OrderSide, OrderStatus,
    SimulatedOrder, SimulationError, SimulationResult, SpotSimulator, TargetDecision,
};
pub use snapshot::{
    MarketDataSnapshot, MarketDataUniverseSnapshot, SnapshotDatasetBinding, SnapshotError,
    SnapshotProvenance, SnapshotStore, SnapshotUniverseBinding, UniverseSnapshotComponent,
};
