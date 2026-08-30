use std::collections::{BTreeMap, BTreeSet};

use crate::{BacktestMetrics, EquityPoint};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioTarget {
    pub decision_time_ms: i64,
    pub universe_id: String,
    pub weights: BTreeMap<String, Decimal>,
    pub cash_reserve: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyTarget {
    pub target: PortfolioTarget,
    pub strategy_id: String,
    pub input_provenance: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskDecision {
    Approve,
    Constrain,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedPortfolioTarget {
    pub strategy_target: PortfolioTarget,
    pub approved_target: Option<PortfolioTarget>,
    pub decision: RiskDecision,
    pub reasons: Vec<String>,
    pub risk_policy_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicy {
    pub policy_id: String,
    pub max_instrument_weight: Decimal,
    pub max_turnover: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioPosition {
    pub quantity: Decimal,
    pub price: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioState {
    pub cash: Decimal,
    pub positions: BTreeMap<String, PortfolioPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub decision_time_ms: i64,
    pub approved_target: PortfolioTarget,
    pub orders: Vec<PortfolioOrder>,
    pub expected_cost: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOrder {
    pub instrument_id: String,
    pub quantity: Decimal,
    pub side: OrderSide,
    pub price: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestDecision {
    pub strategy: StrategyTarget,
    pub risk: ApprovedPortfolioTarget,
    pub execution: ExecutionPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Attribution {
    pub by_instrument: BTreeMap<String, Decimal>,
    pub cash_return: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestEvidence {
    pub initial_capital: Decimal,
    pub final_equity: Decimal,
    pub total_costs: Decimal,
    pub turnover: Decimal,
    pub capacity: Decimal,
    pub decisions: Vec<BacktestDecision>,
    pub attribution: Attribution,
    #[serde(default)]
    pub equity: Vec<EquityPoint>,
    #[serde(default)]
    pub metrics: Option<BacktestMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioBacktestRequest {
    pub initial_capital: Decimal,
    pub risk_policy: RiskPolicy,
    pub execution_cost_rate: Decimal,
    pub decisions: Vec<PortfolioMarketDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioMarketDecision {
    pub time_ms: i64,
    pub prices: BTreeMap<String, Decimal>,
    pub strategy_target: StrategyTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioExecutionStep {
    pub decision: BacktestDecision,
    pub turnover: Decimal,
    pub cost: Decimal,
    pub attribution: Attribution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestError(pub String);

impl std::fmt::Display for BacktestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BacktestError {}

impl PortfolioTarget {
    pub fn validate(&self, universe: &BTreeSet<String>) -> Result<(), BacktestError> {
        if self.universe_id.trim().is_empty()
            || universe.is_empty()
            || self.weights.len() != universe.len()
        {
            return Err(BacktestError(
                "portfolio-target-universe-is-incomplete".into(),
            ));
        }
        if self.weights.keys().collect::<BTreeSet<_>>() != universe.iter().collect::<BTreeSet<_>>()
        {
            return Err(BacktestError("portfolio-target-universe-mismatch".into()));
        }
        if self.cash_reserve < Decimal::ZERO || self.weights.values().any(|w| *w < Decimal::ZERO) {
            return Err(BacktestError("portfolio-target-weight-is-negative".into()));
        }
        let total = self
            .weights
            .values()
            .copied()
            .fold(self.cash_reserve, |a, b| a + b);
        if total != Decimal::ONE {
            return Err(BacktestError("portfolio-target-does-not-sum-to-one".into()));
        }
        Ok(())
    }
}

impl RiskPolicy {
    pub fn apply(
        &self,
        target: &PortfolioTarget,
        state: &PortfolioState,
        universe: &BTreeSet<String>,
    ) -> Result<ApprovedPortfolioTarget, BacktestError> {
        target.validate(universe)?;
        if self.max_instrument_weight < Decimal::ZERO || self.max_instrument_weight > Decimal::ONE {
            return Err(BacktestError("risk-policy-max-weight-is-invalid".into()));
        }
        let mut approved = target.clone();
        let mut reasons = Vec::new();
        for weight in approved.weights.values_mut() {
            if *weight > self.max_instrument_weight {
                *weight = self.max_instrument_weight;
                reasons.push("max-instrument-weight".into());
            }
        }
        approved.cash_reserve = Decimal::ONE - approved.weights.values().copied().sum::<Decimal>();
        let turnover = turnover(target, state);
        if let Some(limit) = self.max_turnover {
            if limit < Decimal::ZERO {
                return Err(BacktestError("risk-policy-max-turnover-is-invalid".into()));
            }
            if turnover > limit {
                return Ok(ApprovedPortfolioTarget {
                    strategy_target: target.clone(),
                    approved_target: None,
                    decision: RiskDecision::Reject,
                    reasons: vec!["max-turnover".into()],
                    risk_policy_id: self.policy_id.clone(),
                });
            }
        }
        let decision = if reasons.is_empty() {
            RiskDecision::Approve
        } else {
            RiskDecision::Constrain
        };
        Ok(ApprovedPortfolioTarget {
            strategy_target: target.clone(),
            approved_target: Some(approved),
            decision,
            reasons,
            risk_policy_id: self.policy_id.clone(),
        })
    }
}

pub struct TopNForecastStrategy;
impl TopNForecastStrategy {
    pub fn target(
        decision_time_ms: i64,
        universe_id: &str,
        forecasts: &BTreeMap<String, Decimal>,
        top_n: usize,
    ) -> Result<PortfolioTarget, BacktestError> {
        if !matches!(top_n, 1 | 3 | 5) {
            return Err(BacktestError("strategy-top-n-is-not-in-fixed-grid".into()));
        }
        if forecasts.is_empty() || top_n > forecasts.len() {
            return Err(BacktestError("strategy-forecast-input-is-missing".into()));
        }
        let mut selected = forecasts.iter().collect::<Vec<_>>();
        selected.sort_by(|(a_id, a), (b_id, b)| b.cmp(a).then_with(|| a_id.cmp(b_id)));
        let selected = selected.into_iter().take(top_n).collect::<Vec<_>>();
        let weight = Decimal::ONE / Decimal::from(selected.len() as u32);
        let mut weights = forecasts
            .keys()
            .map(|id| (id.clone(), Decimal::ZERO))
            .collect::<BTreeMap<_, _>>();
        for (index, (id, _)) in selected.iter().enumerate() {
            let selected_weight = if index + 1 == selected.len() {
                Decimal::ONE - weight * Decimal::from((selected.len() - 1) as u32)
            } else {
                weight
            };
            weights.insert((*id).clone(), selected_weight);
        }
        Ok(PortfolioTarget {
            decision_time_ms,
            universe_id: universe_id.into(),
            weights,
            cash_reserve: Decimal::ZERO,
        })
    }
}

pub fn execute_portfolio_backtest(
    request: PortfolioBacktestRequest,
) -> Result<BacktestEvidence, BacktestError> {
    if request.initial_capital <= Decimal::ZERO || request.execution_cost_rate < Decimal::ZERO {
        return Err(BacktestError("portfolio-backtest-input-is-invalid".into()));
    }
    let mut state = PortfolioState {
        cash: request.initial_capital,
        positions: BTreeMap::new(),
    };
    let mut decisions = Vec::new();
    let mut total_costs = Decimal::ZERO;
    let mut total_turnover = Decimal::ZERO;
    let mut attribution = Attribution::default();
    let mut equity = Vec::with_capacity(request.decisions.len());
    let mut peak = request.initial_capital;
    let mut exposed_points = 0usize;
    for market in request.decisions {
        let step = apply_portfolio_market_decision(
            &mut state,
            market,
            request.initial_capital,
            &request.risk_policy,
            request.execution_cost_rate,
        )?;
        total_turnover += step.turnover;
        total_costs += step.cost;
        for (instrument, value) in step.attribution.by_instrument {
            *attribution.by_instrument.entry(instrument).or_default() += value;
        }
        decisions.push(step.decision);
        let current_equity = portfolio_equity(&state);
        peak = peak.max(current_equity);
        if state
            .positions
            .values()
            .any(|position| !position.quantity.is_zero())
        {
            exposed_points += 1;
        }
        equity.push(EquityPoint {
            open_time_ms: decisions
                .last()
                .map(|decision| decision.execution.decision_time_ms)
                .unwrap_or_default(),
            equity: current_equity,
            drawdown: if peak.is_zero() {
                Decimal::ZERO
            } else {
                (current_equity - peak) / peak
            },
        });
    }
    let final_equity = portfolio_equity(&state);
    let metrics = portfolio_metrics(
        request.initial_capital,
        final_equity,
        total_costs,
        total_turnover,
        &equity,
        decisions
            .iter()
            .map(|decision| decision.execution.orders.len())
            .sum(),
        exposed_points,
    );
    Ok(BacktestEvidence {
        initial_capital: request.initial_capital,
        final_equity,
        total_costs,
        turnover: total_turnover,
        capacity: request.initial_capital,
        decisions,
        attribution,
        equity,
        metrics: Some(metrics),
    })
}

pub fn apply_portfolio_market_decision(
    state: &mut PortfolioState,
    market: PortfolioMarketDecision,
    initial_capital: Decimal,
    risk_policy: &RiskPolicy,
    execution_cost_rate: Decimal,
) -> Result<PortfolioExecutionStep, BacktestError> {
    if initial_capital <= Decimal::ZERO || execution_cost_rate < Decimal::ZERO {
        return Err(BacktestError("portfolio-backtest-input-is-invalid".into()));
    }
    if market.strategy_target.target.decision_time_ms != market.time_ms {
        return Err(BacktestError("portfolio-decision-time-mismatch".into()));
    }
    mark_portfolio_to_market(state, &market.prices)?;
    let universe = market
        .strategy_target
        .target
        .weights
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let risk = risk_policy.apply(&market.strategy_target.target, state, &universe)?;
    let Some(approved) = risk.approved_target.clone() else {
        return Ok(PortfolioExecutionStep {
            decision: BacktestDecision {
                strategy: market.strategy_target,
                risk,
                execution: ExecutionPlan {
                    decision_time_ms: market.time_ms,
                    approved_target: PortfolioTarget {
                        decision_time_ms: market.time_ms,
                        universe_id: String::new(),
                        weights: BTreeMap::new(),
                        cash_reserve: Decimal::ONE,
                    },
                    orders: Vec::new(),
                    expected_cost: Decimal::ZERO,
                },
            },
            turnover: Decimal::ZERO,
            cost: Decimal::ZERO,
            attribution: Attribution::default(),
        });
    };
    let portfolio_value = portfolio_equity(state);
    if portfolio_value <= Decimal::ZERO {
        return Err(BacktestError("portfolio-state-equity-is-invalid".into()));
    }
    let mut orders = Vec::new();
    for (id, weight) in &approved.weights {
        let price = *market
            .prices
            .get(id)
            .ok_or_else(|| BacktestError("backtest-price-is-missing".into()))?;
        if price <= Decimal::ZERO {
            return Err(BacktestError("backtest-price-is-invalid".into()));
        }
        let current = state
            .positions
            .get(id)
            .map(|position| position.quantity * price)
            .unwrap_or_default();
        let difference = portfolio_value * *weight - current;
        if !difference.is_zero() {
            orders.push(PortfolioOrder {
                instrument_id: id.clone(),
                quantity: (difference / price).abs(),
                side: if difference.is_sign_positive() {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                },
                price,
            });
        }
    }
    let turnover = orders
        .iter()
        .map(|order| order.quantity * order.price)
        .sum::<Decimal>()
        / portfolio_value;
    let cost = turnover * portfolio_value * execution_cost_rate;
    let mut attribution = Attribution::default();
    for order in &orders {
        let signed = if order.side == OrderSide::Buy {
            order.quantity
        } else {
            -order.quantity
        };
        state
            .positions
            .entry(order.instrument_id.clone())
            .and_modify(|position| position.quantity += signed)
            .or_insert(PortfolioPosition {
                quantity: signed,
                price: order.price,
            });
        state.cash -= signed * order.price;
        *attribution
            .by_instrument
            .entry(order.instrument_id.clone())
            .or_default() += signed * order.price;
    }
    state.cash -= cost;
    mark_portfolio_to_market(state, &market.prices)?;
    Ok(PortfolioExecutionStep {
        decision: BacktestDecision {
            strategy: market.strategy_target,
            risk,
            execution: ExecutionPlan {
                decision_time_ms: market.time_ms,
                approved_target: approved,
                orders,
                expected_cost: cost,
            },
        },
        turnover,
        cost,
        attribution,
    })
}

pub fn mark_portfolio_to_market(
    state: &mut PortfolioState,
    prices: &BTreeMap<String, Decimal>,
) -> Result<(), BacktestError> {
    for (id, position) in &mut state.positions {
        if position.quantity.is_zero() {
            continue;
        }
        let price = prices
            .get(id)
            .copied()
            .ok_or_else(|| BacktestError("backtest-price-is-missing".into()))?;
        if price <= Decimal::ZERO {
            return Err(BacktestError("backtest-price-is-invalid".into()));
        }
        position.price = price;
    }
    Ok(())
}

fn portfolio_equity(state: &PortfolioState) -> Decimal {
    state.cash
        + state
            .positions
            .values()
            .map(|position| position.quantity * position.price)
            .sum::<Decimal>()
}

fn portfolio_metrics(
    initial: Decimal,
    final_equity: Decimal,
    total_costs: Decimal,
    turnover: Decimal,
    equity: &[EquityPoint],
    order_count: usize,
    exposed_points: usize,
) -> BacktestMetrics {
    let total_return = if initial.is_zero() {
        Decimal::ZERO
    } else {
        final_equity / initial - Decimal::ONE
    };
    BacktestMetrics {
        initial_equity: initial,
        final_equity,
        total_return,
        cagr: Decimal::ZERO,
        annualized_volatility: Decimal::ZERO,
        sharpe: Decimal::ZERO,
        sortino: Decimal::ZERO,
        max_drawdown: equity
            .iter()
            .map(|point| point.drawdown)
            .min()
            .unwrap_or_default(),
        calmar: Decimal::ZERO,
        realized_pnl: Decimal::ZERO,
        unrealized_pnl: final_equity - initial + total_costs,
        total_fees: total_costs,
        turnover,
        fill_count: order_count,
        realized_trade_count: 0,
        win_rate: Decimal::ZERO,
        profit_factor: Decimal::ZERO,
        average_win: Decimal::ZERO,
        average_loss: Decimal::ZERO,
        exposure_time: if equity.is_empty() {
            Decimal::ZERO
        } else {
            Decimal::from(exposed_points) / Decimal::from(equity.len())
        },
        benchmark_return: Decimal::ZERO,
        excess_return: total_return,
    }
}

fn turnover(target: &PortfolioTarget, state: &PortfolioState) -> Decimal {
    let equity = portfolio_equity(state);
    if equity <= Decimal::ZERO {
        return Decimal::ONE;
    }
    target
        .weights
        .iter()
        .map(|(id, weight)| {
            (state
                .positions
                .get(id)
                .map(|p| p.quantity * p.price)
                .unwrap_or_default()
                / equity
                - *weight)
                .abs()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forecasts() -> BTreeMap<String, Decimal> {
        [("AAA", "0.9"), ("BBB", "0.8"), ("CCC", "0.7")]
            .into_iter()
            .map(|(id, v)| (id.into(), v.parse().unwrap()))
            .collect()
    }
    #[test]
    fn top_n_is_deterministic_and_complete() {
        let t = TopNForecastStrategy::target(1, "u1", &forecasts(), 3).unwrap();
        assert!(t.weights["AAA"] > Decimal::ZERO);
        assert_eq!(t.weights["BBB"], t.weights["AAA"]);
        assert_eq!(
            t.weights.values().sum::<Decimal>() + t.cash_reserve,
            Decimal::ONE
        );
    }
    #[test]
    fn target_rejects_missing_members_and_bad_sum() {
        let mut t = TopNForecastStrategy::target(1, "u1", &forecasts(), 1).unwrap();
        t.weights.remove("CCC");
        assert!(
            t.validate(
                &["AAA", "BBB", "CCC"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
            .is_err()
        );
    }
    #[test]
    fn top_n_only_uses_the_host_owned_grid() {
        assert_eq!(
            TopNForecastStrategy::target(1, "u1", &forecasts(), 2)
                .unwrap_err()
                .0,
            "strategy-top-n-is-not-in-fixed-grid"
        );
        assert!(TopNForecastStrategy::target(1, "u1", &forecasts(), 3).is_ok());
    }
    #[test]
    fn missing_forecast_input_is_recorded_before_strategy_execution() {
        assert_eq!(
            TopNForecastStrategy::target(1, "u1", &BTreeMap::new(), 3)
                .unwrap_err()
                .0,
            "strategy-forecast-input-is-missing"
        );
    }
    #[test]
    fn risk_constrains_without_reallocating() {
        let t = TopNForecastStrategy::target(1, "u1", &forecasts(), 1).unwrap();
        let r = RiskPolicy {
            policy_id: "r1".into(),
            max_instrument_weight: Decimal::new(2, 1),
            max_turnover: None,
        }
        .apply(
            &t,
            &PortfolioState::default(),
            &t.weights.keys().cloned().collect(),
        )
        .unwrap();
        assert_eq!(r.decision, RiskDecision::Constrain);
        assert_eq!(
            r.approved_target.unwrap().weights["AAA"],
            Decimal::new(2, 1)
        );
    }

    #[test]
    fn risk_turnover_uses_current_portfolio_weights() {
        let target = TopNForecastStrategy::target(1, "u1", &forecasts(), 3).unwrap();
        let positions = target
            .weights
            .iter()
            .map(|(id, weight)| {
                (
                    id.clone(),
                    PortfolioPosition {
                        quantity: *weight * Decimal::new(10_000, 0),
                        price: Decimal::ONE,
                    },
                )
            })
            .collect();
        let risk = RiskPolicy {
            policy_id: "r1".into(),
            max_instrument_weight: Decimal::ONE,
            max_turnover: Some(Decimal::ZERO),
        }
        .apply(
            &target,
            &PortfolioState {
                cash: Decimal::ZERO,
                positions,
            },
            &target.weights.keys().cloned().collect(),
        )
        .unwrap();
        assert_eq!(risk.decision, RiskDecision::Approve);
    }
}
