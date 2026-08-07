use adaq_backtest_core::MarketDataSnapshot;

use super::{
    BacktestRunRequest, CrossMarketEvidence, CrossMarketValidationReport,
    CrossMarketValidationRequest, RecommendedContext, ValidationAggregate, ValidationProtocol,
    ValidationReport, ValidationStudies, ValidationWindowReport, content_id, string,
};

pub(super) fn run_report(
    studies: &ValidationStudies,
    user_id: &str,
    protocol_id: &str,
) -> Result<ValidationReport, String> {
    let protocol = studies.load_protocol(user_id, protocol_id)?;
    if let Some(cross_market) = &protocol.cross_market {
        return run_cross_market(studies, &protocol, cross_market);
    }
    let mut windows = Vec::with_capacity(protocol.windows.len());
    for window in &protocol.windows {
        let (sample_in, sample_out) = studies.split_snapshot(&protocol.user_id, window)?;
        let (sample_in_request, sample_out_request) =
            window_run_requests(&protocol, &sample_in, &sample_out);
        let sample_in_snapshot_id = sample_in_request.snapshot_id.clone();
        let sample_out_snapshot_id = sample_out_request.snapshot_id.clone();
        match (
            studies.source().run_backtest(sample_in_request),
            studies.source().run_backtest(sample_out_request),
        ) {
            (Ok(sample_in_run), Ok(sample_out_run)) => windows.push(ValidationWindowReport {
                sample_out_start_time_ms: window.sample_out_start_time_ms,
                sample_out_end_time_ms: window.sample_out_end_time_ms,
                sample_in_snapshot_id,
                sample_out_snapshot_id,
                sample_in_run_id: Some(sample_in_run.run_id),
                sample_out_run_id: Some(sample_out_run.run_id),
                sample_in_metrics: Some(sample_in_run.metrics),
                sample_out_metrics: Some(sample_out_run.metrics),
                sample_in_pauses: sample_in_run.pauses,
                sample_out_pauses: sample_out_run.pauses,
                failure: None,
            }),
            (sample_in_result, sample_out_result) => windows.push(ValidationWindowReport {
                sample_out_start_time_ms: window.sample_out_start_time_ms,
                sample_out_end_time_ms: window.sample_out_end_time_ms,
                sample_in_snapshot_id,
                sample_out_snapshot_id,
                sample_in_run_id: sample_in_result.as_ref().ok().map(|run| run.run_id.clone()),
                sample_out_run_id: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.run_id.clone()),
                sample_in_metrics: sample_in_result
                    .as_ref()
                    .ok()
                    .map(|run| run.metrics.clone()),
                sample_out_metrics: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.metrics.clone()),
                sample_in_pauses: sample_in_result
                    .as_ref()
                    .ok()
                    .map(|run| run.pauses.clone())
                    .unwrap_or_default(),
                sample_out_pauses: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.pauses.clone())
                    .unwrap_or_default(),
                failure: Some(
                    [sample_in_result.err(), sample_out_result.err()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            }),
        }
    }
    let aggregate = aggregate_validation(&windows);
    let mut report = ValidationReport {
        report_id: String::new(),
        protocol_id: protocol.protocol_id,
        user_id: protocol.user_id,
        method_version: protocol.method_version,
        aggregation_rule_version: protocol.aggregation_rule_version,
        walk_forward: protocol.walk_forward,
        cross_market: vec![],
        recommended_contexts: vec![],
        cross_market_evidence: None,
        windows,
        aggregate,
    };
    report.report_id = content_id(&report)?;
    studies.save_report(&report)?;
    Ok(report)
}

fn window_run_requests(
    protocol: &ValidationProtocol,
    sample_in: &MarketDataSnapshot,
    sample_out: &MarketDataSnapshot,
) -> (BacktestRunRequest, BacktestRunRequest) {
    let mut sample_in_request = protocol.run.clone();
    sample_in_request.user_id = protocol.user_id.clone();
    let mut sample_out_request = sample_in_request.clone();
    if protocol.run.signal_instances.is_empty() {
        sample_in_request.snapshot_id = sample_in.snapshot_id.clone();
        sample_in_request.run_start_time_ms = None;
        sample_in_request.run_end_time_ms = None;
        sample_out_request.snapshot_id = sample_out.snapshot_id.clone();
        sample_out_request.run_start_time_ms = None;
        sample_out_request.run_end_time_ms = None;
    } else {
        sample_in_request.run_start_time_ms = Some(sample_in.start_time_ms);
        sample_in_request.run_end_time_ms = Some(sample_in.end_time_ms);
        sample_out_request.run_start_time_ms = Some(sample_out.start_time_ms);
        sample_out_request.run_end_time_ms = Some(sample_out.end_time_ms);
    }
    (sample_in_request, sample_out_request)
}

fn run_cross_market(
    studies: &ValidationStudies,
    protocol: &ValidationProtocol,
    cross_market: &CrossMarketValidationRequest,
) -> Result<ValidationReport, String> {
    let contexts = cross_market
        .contexts
        .iter()
        .map(|context| {
            let (snapshot, _) = studies
                .0
                .snapshot_for_user(&protocol.user_id, &context.snapshot_id)?;
            let mut run = context
                .run_override
                .clone()
                .unwrap_or_else(|| protocol.run.clone());
            run.user_id = protocol.user_id.clone();
            run.snapshot_id = snapshot.snapshot_id.clone();
            match studies.source().run_backtest(run.clone()) {
                Ok(result) => Ok(CrossMarketValidationReport {
                    snapshot,
                    run,
                    run_id: Some(result.run_id),
                    metrics: Some(result.metrics),
                    pauses: result.pauses,
                    failure: None,
                }),
                Err(error) => Ok(CrossMarketValidationReport {
                    snapshot,
                    run,
                    run_id: None,
                    metrics: None,
                    pauses: vec![],
                    failure: Some(error),
                }),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let aggregate = aggregate_cross_market(&contexts);
    let evidence = cross_market_evidence(&contexts);
    let mut report = ValidationReport {
        report_id: String::new(),
        protocol_id: protocol.protocol_id.clone(),
        user_id: protocol.user_id.clone(),
        method_version: protocol.method_version.clone(),
        aggregation_rule_version: protocol.aggregation_rule_version.clone(),
        walk_forward: None,
        cross_market: contexts,
        recommended_contexts: vec![],
        cross_market_evidence: evidence,
        windows: vec![],
        aggregate,
    };
    report.recommended_contexts = report
        .cross_market
        .iter()
        .enumerate()
        .filter(|(_, context)| context.failure.is_none())
        .map(|(_, context)| RecommendedContext {
            supporting_report_id: report.report_id.clone(),
            snapshot: context.snapshot.clone(),
            run: context.run.clone(),
        })
        .collect();
    report.report_id = validation_report_id(&report)?;
    for context in &mut report.recommended_contexts {
        context.supporting_report_id = report.report_id.clone();
    }
    studies.save_report(&report)?;
    Ok(report)
}

pub(super) fn aggregate_validation(windows: &[ValidationWindowReport]) -> ValidationAggregate {
    let complete = windows
        .iter()
        .filter(|window| window.failure.is_none())
        .collect::<Vec<_>>();
    let count = rust_decimal::Decimal::from(complete.len().max(1));
    let average = |metric: fn(&adaq_backtest_core::BacktestMetrics) -> rust_decimal::Decimal,
                   sample_out: bool| {
        complete
            .iter()
            .map(|window| {
                metric(if sample_out {
                    window.sample_out_metrics.as_ref().unwrap()
                } else {
                    window.sample_in_metrics.as_ref().unwrap()
                })
            })
            .sum::<rust_decimal::Decimal>()
            / count
    };
    ValidationAggregate {
        completed_windows: complete.len(),
        failed_windows: windows.len() - complete.len(),
        average_sample_in_return: average(|metrics| metrics.total_return, false),
        average_sample_out_return: average(|metrics| metrics.total_return, true),
        worst_sample_out_drawdown: complete
            .iter()
            .map(|window| window.sample_out_metrics.as_ref().unwrap().max_drawdown)
            .min()
            .unwrap_or_default(),
        average_sample_out_sharpe: average(|metrics| metrics.sharpe, true),
        total_fees: complete
            .iter()
            .map(|window| {
                window.sample_in_metrics.as_ref().unwrap().total_fees
                    + window.sample_out_metrics.as_ref().unwrap().total_fees
            })
            .sum(),
        total_trades: complete
            .iter()
            .map(|window| {
                window
                    .sample_in_metrics
                    .as_ref()
                    .unwrap()
                    .realized_trade_count
                    + window
                        .sample_out_metrics
                        .as_ref()
                        .unwrap()
                        .realized_trade_count
            })
            .sum(),
    }
}

pub(super) fn aggregate_cross_market(
    contexts: &[CrossMarketValidationReport],
) -> ValidationAggregate {
    let complete = contexts
        .iter()
        .filter_map(|context| context.metrics.as_ref())
        .collect::<Vec<_>>();
    let count = rust_decimal::Decimal::from(complete.len().max(1));
    ValidationAggregate {
        completed_windows: complete.len(),
        failed_windows: contexts.len() - complete.len(),
        average_sample_in_return: rust_decimal::Decimal::ZERO,
        average_sample_out_return: complete
            .iter()
            .map(|metrics| metrics.total_return)
            .sum::<rust_decimal::Decimal>()
            / count,
        worst_sample_out_drawdown: complete
            .iter()
            .map(|metrics| metrics.max_drawdown)
            .min()
            .unwrap_or_default(),
        average_sample_out_sharpe: complete
            .iter()
            .map(|metrics| metrics.sharpe)
            .sum::<rust_decimal::Decimal>()
            / count,
        total_fees: complete.iter().map(|metrics| metrics.total_fees).sum(),
        total_trades: complete
            .iter()
            .map(|metrics| metrics.realized_trade_count)
            .sum(),
    }
}

pub(super) fn cross_market_evidence(
    contexts: &[CrossMarketValidationReport],
) -> Option<CrossMarketEvidence> {
    let returns = contexts
        .iter()
        .filter_map(|context| context.metrics.as_ref().map(|metrics| metrics.total_return))
        .collect::<Vec<_>>();
    Some(CrossMarketEvidence {
        completed_markets: returns.len(),
        total_return_spread: returns
            .iter()
            .max()
            .zip(returns.iter().min())
            .map(|(max, min)| *max - *min)
            .unwrap_or_default(),
    })
}

pub(super) fn validation_report_id(report: &ValidationReport) -> Result<String, String> {
    let mut value = serde_json::to_value(report).map_err(string)?;
    let object = value
        .as_object_mut()
        .expect("Validation Report serializes as an object");
    object.remove("reportId");
    if let Some(serde_json::Value::Array(contexts)) = object.get_mut("recommendedContexts") {
        for context in contexts {
            context
                .as_object_mut()
                .expect("Recommended Context serializes as an object")
                .remove("supportingReportId");
        }
    }
    content_id(&value)
}
