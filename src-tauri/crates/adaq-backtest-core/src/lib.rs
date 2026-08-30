mod portfolio;
mod simulation;
mod snapshot;
mod strategy;

pub use portfolio::{
    ApprovedPortfolioTarget, Attribution, BacktestDecision, BacktestError, BacktestEvidence,
    ExecutionPlan, PortfolioBacktestRequest, PortfolioExecutionStep, PortfolioMarketDecision,
    PortfolioOrder, PortfolioPosition, PortfolioState, PortfolioTarget, RiskDecision, RiskPolicy,
    StrategyTarget, TopNForecastStrategy, apply_portfolio_market_decision,
    execute_portfolio_backtest, mark_portfolio_to_market,
};

pub use simulation::{
    BacktestMetrics, EquityPoint, ExecutionProfile, Fill, FillPolicy, OrderSide, OrderStatus,
    SimulatedOrder, SimulationError, SimulationResult, SpotSimulator, TargetDecision,
};
pub use snapshot::{
    MarketDataSnapshot, MarketDataUniverseSnapshot, SnapshotDatasetBinding, SnapshotError,
    SnapshotProvenance, SnapshotStore, SnapshotUniverseBinding, UniverseSnapshotComponent,
};
pub use strategy::{
    EvaluationWindow, StrategyAttempt, StrategyAttemptStatus, StrategyBinding, StrategyError,
    StrategyEvidence, StrategyProject, StrategyScope, StrategyWindow,
};
