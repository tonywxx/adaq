use std::collections::BTreeMap;

use ada_data_core::{BarGap, OhlcvBar};
use rust_decimal::{
    Decimal,
    prelude::{FromPrimitive, ToPrimitive},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillPolicy {
    Maker,
    Taker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status", content = "reason")]
pub enum OrderStatus {
    Pending,
    Filled,
    Replaced,
    Cancelled(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionProfile {
    #[serde(with = "rust_decimal::serde::str")]
    pub maker_fee_rate: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub taker_fee_rate: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub adverse_slippage_rate: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub rebalance_threshold: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub price_increment: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity_increment: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_quantity: Decimal,
    #[serde(default, with = "rust_decimal::serde::str")]
    pub risk_free_rate: Decimal,
    pub fill_policy: FillPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDecision {
    pub open_time_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub target_exposure: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedOrder {
    pub order_id: u64,
    pub created_time_ms: i64,
    pub side: OrderSide,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub limit_price: Decimal,
    pub policy: FillPolicy,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub order_id: u64,
    pub open_time_ms: i64,
    pub side: OrderSide,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    #[serde(default, with = "rust_decimal::serde::str")]
    pub requested_quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub fee: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub realized_pnl: Decimal,
    pub role: FillPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityPoint {
    pub open_time_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub equity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub drawdown: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationResult {
    pub orders: Vec<SimulatedOrder>,
    pub fills: Vec<Fill>,
    pub equity: Vec<EquityPoint>,
    pub benchmark_equity: Vec<EquityPoint>,
    pub metrics: BacktestMetrics,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_cash: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_base_quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_fees: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestMetrics {
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_equity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_equity: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_return: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub cagr: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub annualized_volatility: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub sharpe: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub sortino: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub max_drawdown: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub calmar: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub realized_pnl: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub unrealized_pnl: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_fees: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub turnover: Decimal,
    pub fill_count: usize,
    pub realized_trade_count: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub win_rate: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub profit_factor: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_win: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_loss: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub exposure_time: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub benchmark_return: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub excess_return: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationError(pub String);

impl std::fmt::Display for SimulationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SimulationError {}

pub struct SpotSimulator;

impl SpotSimulator {
    pub fn execute(
        bars: &[OhlcvBar],
        gaps: &[BarGap],
        decisions: &[TargetDecision],
        initial_quote_allocation: Decimal,
        profile: &ExecutionProfile,
    ) -> Result<SimulationResult, SimulationError> {
        validate(bars, decisions, initial_quote_allocation, profile)?;
        let decisions = decisions
            .iter()
            .map(|decision| (decision.open_time_ms, *decision))
            .collect::<BTreeMap<_, _>>();
        let mut orders: Vec<SimulatedOrder> = Vec::new();
        let mut fills = Vec::new();
        let mut equity_points = Vec::with_capacity(bars.len());
        let mut cash = initial_quote_allocation;
        let mut base = Decimal::ZERO;
        let mut pending: Option<usize> = None;
        let mut total_fees = Decimal::ZERO;
        let mut cost_basis = Decimal::ZERO;
        let mut peak = initial_quote_allocation;
        let mut exposed_bars = 0usize;

        for (bar_index, bar) in bars.iter().enumerate() {
            if let Some(order_index) = pending {
                let crosses_gap = bar_index > 0
                    && gaps.iter().any(|gap| {
                        gap.start_time_ms > bars[bar_index - 1].open_time_ms
                            && gap.end_time_ms <= bar.open_time_ms
                    });
                if crosses_gap {
                    orders[order_index].status = OrderStatus::Cancelled("Bar Gap".to_owned());
                    pending = None;
                } else if fill_price(&orders[order_index], bar, profile).is_some() {
                    let fill = apply_fill(
                        &mut orders[order_index],
                        bar,
                        profile,
                        &mut cash,
                        &mut base,
                        &mut cost_basis,
                    )?;
                    if let Some(fill) = fill {
                        total_fees = checked_add(total_fees, fill.fee)?;
                        fills.push(fill);
                    }
                    pending = None;
                }
            }

            let equity = checked_add(cash, checked_mul(base, bar.close)?)?;
            if base > Decimal::ZERO {
                exposed_bars += 1;
            }
            peak = peak.max(equity);
            let drawdown = if peak.is_zero() {
                Decimal::ZERO
            } else {
                checked_div(checked_sub(equity, peak)?, peak)?
            };
            equity_points.push(EquityPoint {
                open_time_ms: bar.open_time_ms,
                equity,
                drawdown,
            });

            if let Some(decision) = decisions.get(&bar.open_time_ms) {
                if let Some(order_index) = pending.take() {
                    orders[order_index].status = OrderStatus::Replaced;
                }
                if let Some(order) = create_order(
                    orders.len() as u64 + 1,
                    bar,
                    *decision,
                    equity,
                    cash,
                    base,
                    profile,
                )? {
                    orders.push(order);
                    pending = Some(orders.len() - 1);
                }
            }
        }
        if let Some(index) = pending {
            orders[index].status = OrderStatus::Cancelled("Run ended".to_owned());
        }
        let benchmark_equity = benchmark(bars, initial_quote_allocation, profile)?;
        let metrics = calculate_metrics(
            initial_quote_allocation,
            &equity_points,
            &benchmark_equity,
            &fills,
            base,
            cost_basis,
            bars.last().expect("validated non-empty").close,
            total_fees,
            profile.risk_free_rate,
            exposed_bars,
        )?;
        Ok(SimulationResult {
            orders,
            fills,
            equity: equity_points,
            benchmark_equity,
            metrics,
            final_cash: cash,
            final_base_quantity: base,
            total_fees,
        })
    }
}

fn create_order(
    order_id: u64,
    bar: &OhlcvBar,
    decision: TargetDecision,
    equity: Decimal,
    cash: Decimal,
    base: Decimal,
    profile: &ExecutionProfile,
) -> Result<Option<SimulatedOrder>, SimulationError> {
    let current_notional = checked_mul(base, bar.close)?;
    let desired_notional = checked_mul(equity, decision.target_exposure)?;
    let difference = checked_sub(desired_notional, current_notional)?;
    if equity.is_zero() || checked_div(difference.abs(), equity)? < profile.rebalance_threshold {
        return Ok(None);
    }
    let side = if difference.is_sign_positive() {
        OrderSide::Buy
    } else {
        OrderSide::Sell
    };
    let limit_price = match side {
        OrderSide::Buy => floor_to(bar.close, profile.price_increment)?,
        OrderSide::Sell => ceil_to(bar.close, profile.price_increment)?,
    };
    let fee = match profile.fill_policy {
        FillPolicy::Maker => profile.maker_fee_rate,
        FillPolicy::Taker => profile.taker_fee_rate,
    };
    let raw_quantity = match side {
        OrderSide::Buy => {
            let requested = checked_div(difference, limit_price)?;
            let affordable = checked_div(cash, checked_mul(limit_price, Decimal::ONE + fee)?)?;
            requested.min(affordable)
        }
        OrderSide::Sell => checked_div(difference.abs(), limit_price)?.min(base),
    };
    let quantity = floor_to(raw_quantity, profile.quantity_increment)?;
    if quantity < profile.minimum_quantity {
        return Ok(None);
    }
    Ok(Some(SimulatedOrder {
        order_id,
        created_time_ms: bar.open_time_ms,
        side,
        quantity,
        limit_price,
        policy: profile.fill_policy,
        status: OrderStatus::Pending,
    }))
}

fn fill_price(
    order: &SimulatedOrder,
    bar: &OhlcvBar,
    profile: &ExecutionProfile,
) -> Option<Decimal> {
    match (order.policy, order.side) {
        (FillPolicy::Taker, OrderSide::Buy) => ceil_to(
            bar.open * (Decimal::ONE + profile.adverse_slippage_rate),
            profile.price_increment,
        )
        .ok(),
        (FillPolicy::Taker, OrderSide::Sell) => floor_to(
            bar.open * (Decimal::ONE - profile.adverse_slippage_rate),
            profile.price_increment,
        )
        .ok(),
        (FillPolicy::Maker, OrderSide::Buy) if bar.low <= order.limit_price => {
            Some(bar.open.min(order.limit_price))
        }
        (FillPolicy::Maker, OrderSide::Sell) if bar.high >= order.limit_price => {
            Some(bar.open.max(order.limit_price))
        }
        _ => None,
    }
}

fn apply_fill(
    order: &mut SimulatedOrder,
    bar: &OhlcvBar,
    profile: &ExecutionProfile,
    cash: &mut Decimal,
    base: &mut Decimal,
    cost_basis: &mut Decimal,
) -> Result<Option<Fill>, SimulationError> {
    let price = fill_price(order, bar, profile)
        .ok_or_else(|| SimulationError("Order is not fillable".into()))?;
    let fee_rate = match order.policy {
        FillPolicy::Maker => profile.maker_fee_rate,
        FillPolicy::Taker => profile.taker_fee_rate,
    };
    let quantity = match order.side {
        OrderSide::Buy => floor_to(
            order.quantity.min(checked_div(
                *cash,
                checked_mul(price, Decimal::ONE + fee_rate)?,
            )?),
            profile.quantity_increment,
        )?,
        OrderSide::Sell => floor_to(order.quantity.min(*base), profile.quantity_increment)?,
    };
    if quantity < profile.minimum_quantity {
        order.status = OrderStatus::Cancelled("Insufficient balance".into());
        return Ok(None);
    }
    let notional = checked_mul(price, quantity)?;
    let fee = checked_mul(notional, fee_rate)?;
    let realized_pnl = match order.side {
        OrderSide::Buy => {
            let cost = checked_add(notional, fee)?;
            *cash = checked_sub(*cash, cost)?;
            *base = checked_add(*base, quantity)?;
            *cost_basis = checked_add(*cost_basis, cost)?;
            Decimal::ZERO
        }
        OrderSide::Sell => {
            let average_cost = if base.is_zero() {
                Decimal::ZERO
            } else {
                checked_div(*cost_basis, *base)?
            };
            let removed_cost = checked_mul(average_cost, quantity)?;
            *base = checked_sub(*base, quantity)?;
            let proceeds = checked_sub(notional, fee)?;
            *cash = checked_add(*cash, proceeds)?;
            *cost_basis = checked_sub(*cost_basis, removed_cost)?.max(Decimal::ZERO);
            checked_sub(proceeds, removed_cost)?
        }
    };
    order.status = OrderStatus::Filled;
    Ok(Some(Fill {
        order_id: order.order_id,
        open_time_ms: bar.open_time_ms,
        side: order.side,
        price,
        quantity,
        requested_quantity: order.quantity,
        fee,
        realized_pnl,
        role: order.policy,
    }))
}

fn validate(
    bars: &[OhlcvBar],
    decisions: &[TargetDecision],
    allocation: Decimal,
    profile: &ExecutionProfile,
) -> Result<(), SimulationError> {
    if bars.is_empty() || bars.len() > 1_000_000 || allocation <= Decimal::ZERO {
        return Err(SimulationError(
            "Backtest input or allocation is invalid".into(),
        ));
    }
    if profile.price_increment <= Decimal::ZERO
        || profile.quantity_increment <= Decimal::ZERO
        || profile.minimum_quantity < Decimal::ZERO
        || profile.maker_fee_rate < Decimal::ZERO
        || profile.taker_fee_rate < Decimal::ZERO
        || profile.adverse_slippage_rate < Decimal::ZERO
        || profile.rebalance_threshold < Decimal::ZERO
        || profile.risk_free_rate < Decimal::ZERO
    {
        return Err(SimulationError("Execution Profile is invalid".into()));
    }
    if decisions.iter().any(|decision| {
        decision.target_exposure < Decimal::ZERO || decision.target_exposure > Decimal::ONE
    }) {
        return Err(SimulationError(
            "Spot Target Exposure must be within [0,1]".into(),
        ));
    }
    Ok(())
}

fn benchmark(
    bars: &[OhlcvBar],
    allocation: Decimal,
    profile: &ExecutionProfile,
) -> Result<Vec<EquityPoint>, SimulationError> {
    let first = &bars[0];
    let price = ceil_to(
        checked_mul(first.open, Decimal::ONE + profile.adverse_slippage_rate)?,
        profile.price_increment,
    )?;
    let quantity = floor_to(
        checked_div(
            allocation,
            checked_mul(price, Decimal::ONE + profile.taker_fee_rate)?,
        )?,
        profile.quantity_increment,
    )?;
    let cost = checked_mul(price, quantity)?;
    let fee = checked_mul(cost, profile.taker_fee_rate)?;
    let cash = checked_sub(allocation, checked_add(cost, fee)?)?;
    let mut peak = allocation;
    bars.iter()
        .map(|bar| {
            let equity = checked_add(cash, checked_mul(quantity, bar.close)?)?;
            peak = peak.max(equity);
            Ok(EquityPoint {
                open_time_ms: bar.open_time_ms,
                equity,
                drawdown: checked_div(checked_sub(equity, peak)?, peak)?,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn calculate_metrics(
    initial: Decimal,
    equity: &[EquityPoint],
    benchmark: &[EquityPoint],
    fills: &[Fill],
    final_base: Decimal,
    cost_basis: Decimal,
    final_price: Decimal,
    total_fees: Decimal,
    risk_free_rate: Decimal,
    exposed_bars: usize,
) -> Result<BacktestMetrics, SimulationError> {
    let final_equity = equity.last().expect("validated non-empty").equity;
    let total_return = checked_sub(checked_div(final_equity, initial)?, Decimal::ONE)?;
    let duration_ms = equity.last().unwrap().open_time_ms - equity.first().unwrap().open_time_ms;
    let years = duration_ms.max(1) as f64 / (365.0 * 86_400_000.0);
    let returns = equity
        .windows(2)
        .filter_map(|points| {
            let previous = points[0].equity.to_f64()?;
            let current = points[1].equity.to_f64()?;
            (previous != 0.0).then_some(current / previous - 1.0)
        })
        .collect::<Vec<_>>();
    let periods_per_year = if returns.is_empty() {
        0.0
    } else {
        returns.len() as f64 / years
    };
    let mean_return = mean(&returns);
    let volatility = standard_deviation(&returns, mean_return) * periods_per_year.sqrt();
    let annual_return = mean_return * periods_per_year;
    let risk_free = risk_free_rate.to_f64().unwrap_or(0.0);
    let downside = returns
        .iter()
        .copied()
        .filter(|value| *value < 0.0)
        .collect::<Vec<_>>();
    let downside_deviation = standard_deviation(&downside, 0.0) * periods_per_year.sqrt();
    let cagr = if years > 0.0 && final_equity > Decimal::ZERO {
        (final_equity.to_f64().unwrap_or(0.0) / initial.to_f64().unwrap_or(1.0)).powf(1.0 / years)
            - 1.0
    } else {
        0.0
    };
    let max_drawdown = equity
        .iter()
        .map(|point| point.drawdown)
        .min()
        .unwrap_or_default();
    let realized = fills.iter().map(|fill| fill.realized_pnl).sum::<Decimal>();
    let wins = fills
        .iter()
        .filter(|fill| fill.realized_pnl > Decimal::ZERO)
        .map(|fill| fill.realized_pnl)
        .collect::<Vec<_>>();
    let losses = fills
        .iter()
        .filter(|fill| fill.realized_pnl < Decimal::ZERO)
        .map(|fill| fill.realized_pnl)
        .collect::<Vec<_>>();
    let realized_count = wins.len() + losses.len();
    let gross_profit = wins.iter().copied().sum::<Decimal>();
    let gross_loss = losses.iter().copied().sum::<Decimal>().abs();
    let turnover_notional = fills.iter().try_fold(Decimal::ZERO, |sum, fill| {
        checked_add(sum, checked_mul(fill.price, fill.quantity)?)
    })?;
    let benchmark_return = checked_sub(
        checked_div(
            benchmark.last().expect("benchmark is non-empty").equity,
            initial,
        )?,
        Decimal::ONE,
    )?;
    Ok(BacktestMetrics {
        initial_equity: initial,
        final_equity,
        total_return,
        cagr: analytical(cagr),
        annualized_volatility: analytical(volatility),
        sharpe: analytical(if volatility == 0.0 {
            0.0
        } else {
            (annual_return - risk_free) / volatility
        }),
        sortino: analytical(if downside_deviation == 0.0 {
            0.0
        } else {
            (annual_return - risk_free) / downside_deviation
        }),
        max_drawdown,
        calmar: analytical(if max_drawdown.is_zero() {
            0.0
        } else {
            cagr / max_drawdown.abs().to_f64().unwrap_or(1.0)
        }),
        realized_pnl: realized,
        unrealized_pnl: checked_sub(checked_mul(final_base, final_price)?, cost_basis)?,
        total_fees,
        turnover: checked_div(turnover_notional, initial)?,
        fill_count: fills.len(),
        realized_trade_count: realized_count,
        win_rate: if realized_count == 0 {
            Decimal::ZERO
        } else {
            checked_div(Decimal::from(wins.len()), Decimal::from(realized_count))?
        },
        profit_factor: if gross_loss.is_zero() {
            Decimal::ZERO
        } else {
            checked_div(gross_profit, gross_loss)?
        },
        average_win: average_decimal(&wins)?,
        average_loss: average_decimal(&losses)?,
        exposure_time: checked_div(Decimal::from(exposed_bars), Decimal::from(equity.len()))?,
        benchmark_return,
        excess_return: checked_sub(total_return, benchmark_return)?,
    })
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn standard_deviation(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    (values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64)
        .sqrt()
}

fn analytical(value: f64) -> Decimal {
    Decimal::from_f64(if value.is_finite() { value } else { 0.0 })
        .unwrap_or_default()
        .round_dp(8)
}

fn average_decimal(values: &[Decimal]) -> Result<Decimal, SimulationError> {
    if values.is_empty() {
        Ok(Decimal::ZERO)
    } else {
        checked_div(values.iter().copied().sum(), Decimal::from(values.len()))
    }
}

fn floor_to(value: Decimal, increment: Decimal) -> Result<Decimal, SimulationError> {
    Ok(checked_sub(value, value % increment)?)
}

fn ceil_to(value: Decimal, increment: Decimal) -> Result<Decimal, SimulationError> {
    let floor = floor_to(value, increment)?;
    Ok(if floor == value {
        value
    } else {
        checked_add(floor, increment)?
    })
}

fn checked_add(left: Decimal, right: Decimal) -> Result<Decimal, SimulationError> {
    left.checked_add(right).ok_or_else(overflow)
}

fn checked_sub(left: Decimal, right: Decimal) -> Result<Decimal, SimulationError> {
    left.checked_sub(right).ok_or_else(overflow)
}

fn checked_mul(left: Decimal, right: Decimal) -> Result<Decimal, SimulationError> {
    left.checked_mul(right).ok_or_else(overflow)
}

fn checked_div(left: Decimal, right: Decimal) -> Result<Decimal, SimulationError> {
    left.checked_div(right).ok_or_else(overflow)
}

fn overflow() -> SimulationError {
    SimulationError("Backtest Decimal overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(time: i64, open: i64, high: i64, low: i64, close: i64) -> OhlcvBar {
        OhlcvBar {
            open_time_ms: time,
            open: open.into(),
            high: high.into(),
            low: low.into(),
            close: close.into(),
            base_volume: Decimal::ONE,
            quote_volume: Decimal::ONE,
        }
    }

    fn profile(policy: FillPolicy) -> ExecutionProfile {
        ExecutionProfile {
            maker_fee_rate: Decimal::new(1, 3),
            taker_fee_rate: Decimal::new(2, 3),
            adverse_slippage_rate: Decimal::new(1, 2),
            rebalance_threshold: Decimal::ZERO,
            price_increment: Decimal::ONE,
            quantity_increment: Decimal::new(1, 1),
            minimum_quantity: Decimal::new(1, 1),
            risk_free_rate: Decimal::ZERO,
            fill_policy: policy,
        }
    }

    #[test]
    fn taker_fills_on_next_open_and_leaves_final_position_open() {
        let bars = vec![bar(1, 100, 101, 99, 100), bar(2, 110, 112, 108, 111)];
        let result = SpotSimulator::execute(
            &bars,
            &[],
            &[TargetDecision {
                open_time_ms: 1,
                target_exposure: Decimal::ONE,
            }],
            Decimal::from(1000),
            &profile(FillPolicy::Taker),
        )
        .unwrap();
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].open_time_ms, 2);
        assert!(result.final_base_quantity > Decimal::ZERO);
        assert_eq!(result.metrics.fill_count, 1);
        assert_eq!(result.benchmark_equity.len(), bars.len());
    }

    #[test]
    fn maker_cancels_across_gap_instead_of_assuming_a_fill() {
        let bars = vec![bar(1, 100, 101, 99, 100), bar(3, 90, 105, 80, 95)];
        let gaps = vec![BarGap {
            start_time_ms: 2,
            end_time_ms: 3,
        }];
        let result = SpotSimulator::execute(
            &bars,
            &gaps,
            &[TargetDecision {
                open_time_ms: 1,
                target_exposure: Decimal::ONE,
            }],
            Decimal::from(1000),
            &profile(FillPolicy::Maker),
        )
        .unwrap();
        assert!(result.fills.is_empty());
        assert_eq!(
            result.orders[0].status,
            OrderStatus::Cancelled("Bar Gap".into())
        );
    }

    #[test]
    fn sells_use_weighted_average_cost_basis() {
        let bars = vec![
            bar(1, 100, 100, 100, 100),
            bar(2, 100, 100, 100, 100),
            bar(3, 200, 200, 200, 200),
            bar(4, 150, 150, 150, 150),
        ];
        let mut execution = profile(FillPolicy::Taker);
        execution.taker_fee_rate = Decimal::ZERO;
        execution.adverse_slippage_rate = Decimal::ZERO;
        execution.quantity_increment = Decimal::ONE;
        let result = SpotSimulator::execute(
            &bars,
            &[],
            &[
                TargetDecision {
                    open_time_ms: 1,
                    target_exposure: Decimal::new(5, 1),
                },
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 3,
                    target_exposure: Decimal::new(5, 1),
                },
            ],
            Decimal::from(1000),
            &execution,
        )
        .unwrap();
        assert_eq!(result.final_base_quantity, Decimal::from(4));
        assert_eq!(result.fills[1].quantity, Decimal::from(2));
        assert_eq!(result.fills[1].requested_quantity, Decimal::from(5));
        assert_eq!(
            result.fills[2].realized_pnl.round_dp(2),
            Decimal::new(6429, 2)
        );
        assert_eq!(
            result.metrics.unrealized_pnl.round_dp(2),
            Decimal::new(8571, 2)
        );
    }
}
