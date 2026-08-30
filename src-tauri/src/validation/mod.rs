//! Validation Studies module.
//!
//! One deep, Tauri-independent module owning Validation Protocol identity,
//! protocol validation rules, walk-forward and cross-market Validation
//! Report runs, report identity, aggregation, and export. The external
//! interface is limited to creating and listing Protocols, running, listing,
//! and exporting Reports, and the summary, reset, and run-reference hooks
//! the composition root calls; all Validation Protocol and Validation Report
//! schema handling and SQL stay private to this module.

mod runner;

#[cfg(test)]
mod tests;

use std::{
    collections::HashSet,
    sync::{Arc, MutexGuard},
};

use adaq_backtest_core::{BacktestMetrics, MarketDataSnapshot};
use adaq_component_tooling::ComponentPackage;
use adaq_data_core::{BarGap, OhlcvBar};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    backtest::{BacktestRunRequest, RunPauseRecord, StrategyQualificationBinding},
    user::validate_user,
};

/// The concrete local dependencies composed into Validation Studies. The
/// complete Local Research state is never passed in; only database access,
/// Component Package access, Market Data Snapshot access and persistence,
/// and Backtest Run execution are shared.
pub(crate) trait ValidationSource: Send + Sync {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String>;
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String>;
    fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String>;
    fn run_backtest(&self, request: BacktestRunRequest) -> Result<ValidationRunOutcome, String>;
    fn run_portfolio_backtest(
        &self,
        request: BacktestRunRequest,
    ) -> Result<ValidationRunOutcome, String>;
}

/// The subset of one Backtest Run outcome consumed by Validation Reports.
pub(crate) struct ValidationRunOutcome {
    pub run_id: String,
    pub metrics: BacktestMetrics,
    pub pauses: Vec<RunPauseRecord>,
}

/// The Validation evidence counts the Local Data summary reports for one
/// User.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValidationSummary {
    pub protocol_count: u64,
    pub report_count: u64,
}

/// The Validation Studies interface: Protocol creation and listing, Report
/// running, listing, and export, plus the summary-for-user, reset-for-user,
/// and run-reference hooks the composition root calls.
#[derive(Clone)]
pub(crate) struct ValidationStudies(pub(super) Arc<dyn ValidationSource>);

impl ValidationStudies {
    /// Creates the module and initializes the Validation Protocol and
    /// Validation Report schema, which live inside this module.
    pub(crate) fn open(source: Arc<dyn ValidationSource>) -> Result<Self, String> {
        source
            .database()?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS validation_protocols (
                    protocol_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    protocol_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS validation_reports (
                    report_id TEXT PRIMARY KEY,
                    protocol_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    report_json TEXT NOT NULL,
                    FOREIGN KEY(protocol_id) REFERENCES validation_protocols(protocol_id)
                 );",
            )
            .map_err(string)?;
        Ok(Self(source))
    }

    pub(super) fn source(&self) -> &Arc<dyn ValidationSource> {
        &self.0
    }

    /// Validates and freezes a new Protocol for one User, deriving
    /// walk-forward windows from the frozen Snapshot when requested.
    pub(crate) fn create_protocol(
        &self,
        request: ValidationProtocolCreateRequest,
    ) -> Result<ValidationProtocol, String> {
        self.validate_protocol(&request)?;
        let windows = request
            .walk_forward
            .as_ref()
            .map(|walk_forward| self.walk_forward_windows(&request.user_id, walk_forward))
            .transpose()?
            .unwrap_or(request.windows);
        let mut protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: request.user_id.clone(),
            run: request.run,
            windows,
            walk_forward: request.walk_forward,
            cross_market: request.cross_market,
            method_version: request.method_version,
            aggregation_rule_version: request.aggregation_rule_version,
            strategy_binding: request.strategy_binding,
            final_evidence_sealed: request.final_evidence_sealed,
        };
        protocol.protocol_id = content_id(&protocol)?;
        self.save_protocol(&protocol)?;
        self.load_protocol(&protocol.user_id, &protocol.protocol_id)
    }

    pub(crate) fn list_protocols(&self, user_id: &str) -> Result<Vec<ValidationProtocol>, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let mut statement = database
            .prepare("SELECT protocol_json FROM validation_protocols WHERE user_id = ?1 ORDER BY rowid DESC")
            .map_err(string)?;
        json_rows(&mut statement, user_id)
    }

    /// Runs the Report bound to one Protocol, reusing the exact Backtest
    /// Runs already recorded for every window and market context.
    pub(crate) fn run_report(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<ValidationReport, String> {
        runner::run_report(self, user_id, protocol_id)
    }

    pub(crate) fn list_reports(&self, user_id: &str) -> Result<Vec<ValidationReport>, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let mut statement = database
            .prepare(
                "SELECT report_json FROM validation_reports WHERE user_id = ?1 ORDER BY rowid DESC",
            )
            .map_err(string)?;
        json_rows(&mut statement, user_id)
    }

    pub(crate) fn protocol_for_user(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<ValidationProtocol, String> {
        self.load_protocol(user_id, protocol_id)
    }

    pub(crate) fn report_for_user(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<ValidationReport, String> {
        self.list_reports(user_id)?
            .into_iter()
            .find(|report| report.report_id == report_id)
            .ok_or_else(|| "Validation Report was not found".to_owned())
    }

    pub(crate) fn export_report(
        &self,
        user_id: &str,
        report_id: &str,
        format: &str,
    ) -> Result<String, String> {
        let report = self
            .list_reports(user_id)?
            .into_iter()
            .find(|report| report.report_id == report_id)
            .ok_or("Validation Report was not found")?;
        match format {
            "json" => serde_json::to_string_pretty(&report).map_err(string),
            "markdown" => Ok(validation_markdown(&report)),
            _ => Err("Validation export format is invalid".into()),
        }
    }

    /// Whether any immutable Validation Report of one User references a
    /// Backtest Run; the composition root uses this to refuse deleting Runs
    /// frozen into Validation evidence.
    pub(crate) fn references_run(&self, user_id: &str, run_id: &str) -> Result<bool, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let mut statement = database
            .prepare("SELECT report_json FROM validation_reports WHERE user_id = ?1")
            .map_err(string)?;
        let reports: Vec<ValidationReport> = json_rows(&mut statement, user_id)?;
        Ok(reports
            .iter()
            .any(|report| report_references_run(report, run_id)))
    }

    /// Summary hook: the Validation evidence counts for one User.
    pub(crate) fn summary_for_user(&self, user_id: &str) -> Result<ValidationSummary, String> {
        validate_user(user_id)?;
        let database = self.0.database()?;
        let count = |sql: &str| -> Result<u64, String> {
            database
                .query_row(sql, [user_id], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(string)
        };
        Ok(ValidationSummary {
            protocol_count: count("SELECT COUNT(*) FROM validation_protocols WHERE user_id = ?1")?,
            report_count: count("SELECT COUNT(*) FROM validation_reports WHERE user_id = ?1")?,
        })
    }

    /// Reset hook: removes one User's Validation evidence inside the
    /// caller's reset transaction.
    pub(crate) fn reset_for_user(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<(), String> {
        validate_user(user_id)?;
        database
            .execute(
                "DELETE FROM validation_reports WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        database
            .execute(
                "DELETE FROM validation_protocols WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        Ok(())
    }

    fn validate_protocol(&self, request: &ValidationProtocolCreateRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        if request.run.user_id != request.user_id
            || !request
                .aggregation_rule_version
                .starts_with("equal-window@")
        {
            return Err("Validation Protocol is invalid".into());
        }
        if request.strategy_binding != request.run.strategy_binding {
            return Err("Validation Strategy binding must match its Backtest Run".into());
        }
        self.validate_run_configuration(&request.user_id, &request.run)?;
        match request.method_version.as_str() {
            "chronological-holdout@1"
                if request.walk_forward.is_none() && !request.windows.is_empty() =>
            {
                for window in &request.windows {
                    self.split_snapshot(&request.user_id, window)?;
                }
            }
            "walk-forward@1" if request.windows.is_empty() => {
                let walk_forward = request
                    .walk_forward
                    .as_ref()
                    .ok_or("Walk-forward configuration is required")?;
                if request.run.snapshot_id != walk_forward.snapshot_id {
                    return Err("Walk-forward must use the frozen Snapshot".into());
                }
                self.walk_forward_windows(&request.user_id, walk_forward)?;
            }
            "cross-market@1"
                if request.windows.is_empty()
                    && request.walk_forward.is_none()
                    && request.cross_market.is_some() =>
            {
                self.validate_cross_market(request)?;
            }
            _ => return Err("Validation Protocol is invalid".into()),
        }
        Ok(())
    }

    fn validate_run_configuration(
        &self,
        user_id: &str,
        run: &BacktestRunRequest,
    ) -> Result<(), String> {
        if run.user_id != user_id {
            return Err("Validation Run configuration belongs to another User".into());
        }
        self.0
            .package_for_user(user_id, &run.strategy_archive_sha256)?;
        for factor in &run.factor_instances {
            self.0.package_for_user(user_id, &factor.archive_sha256)?;
        }
        Ok(())
    }

    fn validate_cross_market(
        &self,
        request: &ValidationProtocolCreateRequest,
    ) -> Result<(), String> {
        let contexts = &request
            .cross_market
            .as_ref()
            .expect("validated above")
            .contexts;
        if contexts.len() < 2 {
            return Err("Cross-market validation requires at least two markets".into());
        }
        let mut snapshots = HashSet::new();
        let mut markets = HashSet::new();
        let mut interval = None;
        for context in contexts {
            if !snapshots.insert(&context.snapshot_id) {
                return Err("Cross-market validation contains a duplicate Snapshot".into());
            }
            let (snapshot, bars) = self
                .0
                .snapshot_for_user(&request.user_id, &context.snapshot_id)?;
            if bars.is_empty() {
                return Err("Cross-market validation requires market evidence".into());
            }
            if interval
                .replace(snapshot.interval)
                .is_some_and(|current| current != snapshot.interval)
            {
                return Err("Cross-market validation requires compatible Bar Intervals".into());
            }
            if !markets.insert((
                snapshot.src.clone(),
                snapshot.code.clone(),
                snapshot.interval,
            )) {
                return Err(
                    "Cross-market validation contains a duplicate Instrument context".into(),
                );
            }
            if let Some(run) = &context.run_override {
                if run.snapshot_id != context.snapshot_id {
                    return Err("Cross-market override must use its frozen Snapshot".into());
                }
                self.validate_run_configuration(&request.user_id, run)?;
            }
        }
        Ok(())
    }

    /// Splits one frozen Snapshot into chronological sample-in and
    /// sample-out Snapshots for a Validation window.
    fn split_snapshot(
        &self,
        user_id: &str,
        window: &ValidationWindowRequest,
    ) -> Result<(MarketDataSnapshot, MarketDataSnapshot), String> {
        let (snapshot, bars) = self.0.snapshot_for_user(user_id, &window.snapshot_id)?;
        let split = bars.partition_point(|bar| bar.open_time_ms < window.sample_out_start_time_ms);
        let end = window
            .sample_out_end_time_ms
            .map(|end| bars.partition_point(|bar| bar.open_time_ms < end))
            .unwrap_or(bars.len());
        if split == 0 || split >= end {
            return Err("Validation sample-out window must be non-empty and chronological".into());
        }
        let sample_in_start = window
            .sample_in_start_time_ms
            .map(|start| bars.partition_point(|bar| bar.open_time_ms < start))
            .unwrap_or(0);
        let sample_in_end = window
            .sample_in_end_time_ms
            .map(|end| bars.partition_point(|bar| bar.open_time_ms < end))
            .unwrap_or(split);
        if sample_in_start >= sample_in_end || sample_in_end > split {
            return Err(
                "Validation sample-in window must be non-empty and before sample-out".into(),
            );
        }
        let gaps = snapshot
            .gaps
            .iter()
            .map(|gap| BarGap {
                start_time_ms: gap.start_time_ms,
                end_time_ms: gap.end_time_ms,
            })
            .collect::<Vec<_>>();
        let series = |bars: Vec<OhlcvBar>| adaq_data_core::BarSeries {
            src: snapshot.src.clone(),
            code: snapshot.code.clone(),
            interval: snapshot.interval,
            bars,
            gaps: gaps.clone(),
        };
        Ok((
            self.0.persist_snapshot_for_user(
                user_id,
                &series(bars[sample_in_start..sample_in_end].to_vec()),
            )?,
            self.0
                .persist_snapshot_for_user(user_id, &series(bars[split..end].to_vec()))?,
        ))
    }

    fn walk_forward_windows(
        &self,
        user_id: &str,
        request: &WalkForwardValidationRequest,
    ) -> Result<Vec<ValidationWindowRequest>, String> {
        if request.window_size_bars == 0
            || request.step_size_bars == 0
            || request.minimum_history_bars == 0
        {
            return Err("Walk-forward window sizes must be positive".into());
        }
        if request.step_size_bars < request.window_size_bars {
            return Err("Walk-forward step must not overlap sample-out windows".into());
        }
        let (_, bars) = self.0.snapshot_for_user(user_id, &request.snapshot_id)?;
        if request.minimum_history_bars >= bars.len() {
            return Err("Walk-forward requires more history than the minimum".into());
        }
        let windows = (request.minimum_history_bars..bars.len())
            .step_by(request.step_size_bars)
            .take_while(|start| start.saturating_add(request.window_size_bars) <= bars.len())
            .map(|start| ValidationWindowRequest {
                snapshot_id: request.snapshot_id.clone(),
                sample_out_start_time_ms: bars[start].open_time_ms,
                sample_out_end_time_ms: bars
                    .get(start + request.window_size_bars)
                    .map(|bar| bar.open_time_ms),
                sample_in_start_time_ms: None,
                sample_in_end_time_ms: None,
            })
            .collect::<Vec<_>>();
        if windows.is_empty() {
            Err("Walk-forward history cannot produce a complete window".into())
        } else {
            Ok(windows)
        }
    }

    fn save_protocol(&self, protocol: &ValidationProtocol) -> Result<(), String> {
        self.0.database()?.execute(
            "INSERT OR IGNORE INTO validation_protocols(protocol_id, user_id, protocol_json) VALUES (?1, ?2, ?3)",
            params![protocol.protocol_id, protocol.user_id, serde_json::to_string(protocol).map_err(string)?],
        ).map_err(string)?;
        Ok(())
    }

    fn load_protocol(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<ValidationProtocol, String> {
        validate_user(user_id)?;
        self.0
            .database()?
            .query_row(
                "SELECT protocol_json FROM validation_protocols WHERE user_id = ?1 AND protocol_id = ?2",
                params![user_id, protocol_id],
                |row| serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))),
            )
            .map_err(|_| "Validation Protocol was not found".to_owned())
    }

    fn save_report(&self, report: &ValidationReport) -> Result<(), String> {
        self.0.database()?.execute(
            "INSERT OR IGNORE INTO validation_reports(report_id, protocol_id, user_id, report_json) VALUES (?1, ?2, ?3, ?4)",
            params![report.report_id, report.protocol_id, report.user_id, serde_json::to_string(report).map_err(string)?],
        ).map_err(string)?;
        Ok(())
    }
}

fn json_rows<T: serde::de::DeserializeOwned>(
    statement: &mut rusqlite::Statement<'_>,
    user_id: &str,
) -> Result<Vec<T>, String> {
    statement
        .query_map([user_id], |row| {
            serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocolCreateRequest {
    pub user_id: String,
    pub run: BacktestRunRequest,
    pub windows: Vec<ValidationWindowRequest>,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Option<CrossMarketValidationRequest>,
    pub method_version: String,
    pub aggregation_rule_version: String,
    #[serde(default)]
    pub strategy_binding: Option<StrategyQualificationBinding>,
    #[serde(default)]
    pub final_evidence_sealed: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationRequest {
    pub contexts: Vec<CrossMarketValidationContextRequest>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationContextRequest {
    pub snapshot_id: String,
    #[serde(default)]
    pub run_override: Option<BacktestRunRequest>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationWindowRequest {
    pub snapshot_id: String,
    pub sample_out_start_time_ms: i64,
    #[serde(default)]
    pub sample_out_end_time_ms: Option<i64>,
    #[serde(default)]
    pub sample_in_start_time_ms: Option<i64>,
    #[serde(default)]
    pub sample_in_end_time_ms: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardValidationRequest {
    pub snapshot_id: String,
    pub window_size_bars: usize,
    pub step_size_bars: usize,
    pub minimum_history_bars: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocol {
    pub protocol_id: String,
    pub user_id: String,
    pub run: BacktestRunRequest,
    pub windows: Vec<ValidationWindowRequest>,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Option<CrossMarketValidationRequest>,
    pub method_version: String,
    pub aggregation_rule_version: String,
    #[serde(default)]
    pub strategy_binding: Option<StrategyQualificationBinding>,
    #[serde(default)]
    pub final_evidence_sealed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocolIdRequest {
    pub user_id: String,
    pub protocol_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationWindowReport {
    pub sample_out_start_time_ms: i64,
    #[serde(default)]
    pub sample_out_end_time_ms: Option<i64>,
    #[serde(default)]
    pub sample_in_start_time_ms: Option<i64>,
    #[serde(default)]
    pub sample_in_end_time_ms: Option<i64>,
    pub sample_in_snapshot_id: String,
    pub sample_out_snapshot_id: String,
    pub sample_in_run_id: Option<String>,
    pub sample_out_run_id: Option<String>,
    pub sample_in_metrics: Option<BacktestMetrics>,
    pub sample_out_metrics: Option<BacktestMetrics>,
    pub sample_in_pauses: Vec<RunPauseRecord>,
    pub sample_out_pauses: Vec<RunPauseRecord>,
    pub failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationAggregate {
    pub completed_windows: usize,
    pub failed_windows: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_in_return: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_out_return: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub worst_sample_out_drawdown: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_out_sharpe: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_fees: rust_decimal::Decimal,
    pub total_trades: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub report_id: String,
    pub protocol_id: String,
    pub user_id: String,
    pub method_version: String,
    pub aggregation_rule_version: String,
    #[serde(default)]
    pub strategy_binding: Option<StrategyQualificationBinding>,
    #[serde(default)]
    pub final_evidence_sealed: bool,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Vec<CrossMarketValidationReport>,
    #[serde(default)]
    pub recommended_contexts: Vec<RecommendedContext>,
    #[serde(default)]
    pub cross_market_evidence: Option<CrossMarketEvidence>,
    pub windows: Vec<ValidationWindowReport>,
    pub aggregate: ValidationAggregate,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationReport {
    pub snapshot: MarketDataSnapshot,
    pub run: BacktestRunRequest,
    pub run_id: Option<String>,
    pub metrics: Option<BacktestMetrics>,
    pub pauses: Vec<RunPauseRecord>,
    pub failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketEvidence {
    pub completed_markets: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_return_spread: rust_decimal::Decimal,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedContext {
    pub supporting_report_id: String,
    pub snapshot: MarketDataSnapshot,
    pub run: BacktestRunRequest,
}

fn content_id(value: &impl Serialize) -> Result<String, String> {
    let value = canonical_json(serde_json::to_value(value).map_err(string)?);
    Ok(Sha256::digest(serde_json::to_vec(&value).map_err(string)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect(),
        ),
        value => value,
    }
}

fn report_references_run(report: &ValidationReport, run_id: &str) -> bool {
    report.windows.iter().any(|window| {
        window.sample_in_run_id.as_deref() == Some(run_id)
            || window.sample_out_run_id.as_deref() == Some(run_id)
    }) || report
        .cross_market
        .iter()
        .any(|context| context.run_id.as_deref() == Some(run_id))
}

fn validation_markdown(report: &ValidationReport) -> String {
    format!(
        "# Validation Report {}\n\n[Metric definitions](https://github.com/tonywxx/adaq/blob/main/docs/reference/research-metrics.md)\n\n```json\n{}\n```\n",
        report.report_id,
        serde_json::to_string_pretty(report).expect("Validation Report serializes")
    )
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
