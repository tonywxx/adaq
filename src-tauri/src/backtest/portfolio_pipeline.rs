use std::collections::BTreeMap;

use adaq_backtest_core::{
    BacktestEvidence, PortfolioBacktestRequest as CoreRequest, PortfolioMarketDecision, RiskPolicy,
    StrategyTarget, TopNForecastStrategy, execute_portfolio_backtest,
};
use rusqlite::{OptionalExtension, params};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use sha2::{Digest, Sha256};

use super::{Backtests, PortfolioBacktestRequest, PortfolioBacktestView, string};

pub(super) fn execute(
    backtests: &Backtests,
    request: PortfolioBacktestRequest,
) -> Result<PortfolioBacktestView, String> {
    if request.strategy_id.trim().is_empty()
        || request.universe_snapshot_id.trim().is_empty()
        || request.signal_dataset_ids.is_empty()
    {
        return Err("Portfolio Backtest request is incomplete".into());
    }
    let project = backtests.strategy_project(&request.user_id, &request.strategy_id)?;
    if project.scope != adaq_backtest_core::StrategyScope::Portfolio
        || project.context_hash != request.universe_snapshot_id
    {
        return Err("Portfolio Backtest Strategy Context is mixed or stale".into());
    }
    let request_json = serde_json::to_vec(&request).map_err(string)?;
    let request_hash = Sha256::digest(request_json)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if let Some(evidence) = load_existing(backtests, &request.user_id, &request_hash)? {
        return Ok(PortfolioBacktestView {
            run_id: format!("portfolio-{request_hash}"),
            reused_existing_run: true,
            evidence,
        });
    }

    let initial_capital = decimal(&request.initial_capital, "initial-capital")?;
    let execution_cost_rate = decimal(&request.execution_cost_rate, "execution-cost-rate")?;
    let max_instrument_weight = decimal(&request.max_instrument_weight, "max-instrument-weight")?;
    let max_turnover = request
        .max_turnover
        .as_deref()
        .map(|value| decimal(value, "max-turnover"))
        .transpose()?;
    let universe = backtests
        .source()
        .portfolio_universe_snapshot_for_user(&request.user_id, &request.universe_snapshot_id)?;
    if universe.universe.evidence_state == "unknown"
        || universe.universe.evidence_reasons.is_empty()
    {
        return Err("Portfolio Backtest requires known Universe evidence".into());
    }
    if request.top_n == 0 || request.top_n > universe.components.len() {
        return Err("Portfolio Backtest Top-N exceeds the frozen Universe".into());
    }

    let mut bars_by_code = BTreeMap::new();
    let mut snapshot_by_code = BTreeMap::new();
    for component in &universe.components {
        let (snapshot, bars) = backtests
            .source()
            .snapshot_for_user(&request.user_id, &component.snapshot_id)?;
        if snapshot.snapshot_id != component.snapshot_id
            || snapshot.interval != universe.interval
            || snapshot.start_time_ms < universe.start_time_ms
            || snapshot.end_time_ms > universe.end_time_ms
        {
            return Err("Portfolio Backtest Snapshot Context is mixed or incomplete".into());
        }
        let code = component.dataset.instrument.code.clone();
        if bars_by_code.insert(code.clone(), bars).is_some() {
            return Err("Portfolio Backtest Universe contains duplicate instruments".into());
        }
        snapshot_by_code.insert(code, component.snapshot_id.clone());
    }

    let datasets = backtests.source().signal_datasets(
        &request.user_id,
        true,
        Some(&request.signal_dataset_ids),
    )?;
    let mut datasets_by_code = BTreeMap::new();
    for dataset in datasets {
        if dataset.snapshot_id.trim().is_empty() || dataset.evidence_state == "unknown" {
            return Err("Portfolio Backtest Signal Context is not admissible".into());
        }
        let code = dataset.code.clone();
        let snapshot_id = snapshot_by_code
            .get(&code)
            .ok_or_else(|| "Portfolio Backtest Signal is outside the frozen Universe".to_owned())?;
        if snapshot_id != &dataset.snapshot_id || datasets_by_code.insert(code, dataset).is_some() {
            return Err("Portfolio Backtest Signal Context is mixed or duplicated".into());
        }
    }
    if datasets_by_code.len() != bars_by_code.len() {
        return Err("Portfolio Backtest requires one causal Signal Dataset per instrument".into());
    }
    let window = match request.window {
        adaq_backtest_core::EvaluationWindow::Selection => &project.selection_window,
        adaq_backtest_core::EvaluationWindow::Final => &project.final_window,
    };

    let times = bars_by_code
        .values()
        .next()
        .ok_or_else(|| "Portfolio Backtest Universe has no Bars".to_owned())?
        .iter()
        .map(|bar| bar.open_time_ms)
        .filter(|time| {
            *time >= window.start_time_ms
                && *time <= window.end_time_ms
                && bars_by_code
                    .values()
                    .all(|bars| bars.iter().any(|bar| bar.open_time_ms == *time))
        })
        .collect::<Vec<_>>();
    if times.is_empty() {
        return Err("Portfolio Backtest Universe has no aligned Closed Bars".into());
    }

    let mut decisions = Vec::with_capacity(times.len());
    for time in times {
        let mut prices = BTreeMap::new();
        let mut forecasts = BTreeMap::new();
        for (code, bars) in &bars_by_code {
            let bar = bars
                .iter()
                .find(|bar| bar.open_time_ms == time)
                .ok_or_else(|| "Portfolio Backtest price alignment failed".to_owned())?;
            prices.insert(code.clone(), bar.open);
            let dataset = datasets_by_code
                .get(code)
                .ok_or_else(|| format!("Portfolio Backtest signal is missing for {code}"))?;
            let row = dataset
                .rows
                .iter()
                .find(|row| row.prediction_time_ms == time)
                .ok_or_else(|| format!("Portfolio Backtest signal row is missing for {code}"))?;
            if row.available_at_ms > time {
                return Err("Portfolio Backtest signal is not causally available".into());
            }
            let value = row
                .values
                .as_ref()
                .and_then(|values| values.first())
                .and_then(|value| Decimal::from_f64(*value))
                .ok_or_else(|| "Portfolio Backtest signal value is unavailable".to_owned())?;
            forecasts.insert(code.clone(), value);
        }
        let target = TopNForecastStrategy::target(
            time,
            &universe.universe.universe_id,
            &forecasts,
            request.top_n,
        )
        .map_err(string)?;
        decisions.push(PortfolioMarketDecision {
            time_ms: time,
            prices,
            strategy_target: StrategyTarget {
                target,
                strategy_id: request.strategy_id.clone(),
                input_provenance: BTreeMap::from([(
                    "universeSnapshotId".into(),
                    universe.snapshot_id.clone(),
                )]),
            },
        });
    }

    let evidence = execute_portfolio_backtest(CoreRequest {
        initial_capital,
        risk_policy: RiskPolicy {
            policy_id: format!("strategy:{}", request.strategy_id),
            max_instrument_weight,
            max_turnover,
        },
        execution_cost_rate,
        decisions,
    })
    .map_err(string)?;
    let run_id = format!("portfolio-{request_hash}");
    let evidence_json = serde_json::to_string(&evidence).map_err(string)?;
    backtests
        .source()
        .database()?
        .execute(
            "INSERT INTO portfolio_backtest_runs(run_id, user_id, request_hash, evidence_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, request.user_id, request_hash, evidence_json],
        )
        .map_err(string)?;
    Ok(PortfolioBacktestView {
        run_id,
        reused_existing_run: false,
        evidence,
    })
}

fn load_existing(
    backtests: &Backtests,
    user_id: &str,
    request_hash: &str,
) -> Result<Option<BacktestEvidence>, String> {
    let database = backtests.source().database()?;
    let json = database
        .query_row(
            "SELECT evidence_json FROM portfolio_backtest_runs
             WHERE user_id = ?1 AND request_hash = ?2",
            params![user_id, request_hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(string)?;
    json.map(|value| serde_json::from_str(&value).map_err(string))
        .transpose()
}

fn decimal(value: &str, field: &str) -> Result<Decimal, String> {
    value
        .parse::<Decimal>()
        .map_err(|_| format!("Portfolio Backtest {field} is invalid"))
}
