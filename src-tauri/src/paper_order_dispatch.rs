//! Host-owned Paper order dispatch shared by Bot Target and Flatten.

use adaq_paper_trading_core::RiskPolicy;

use crate::{
    bot_operations::BotStore, local_research::LocalResearchState, paper_trading::PaperOrderRequest,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProviderOrderKind {
    PostOnly,
    Market,
}

impl ProviderOrderKind {
    pub(crate) fn for_fill_policy(policy: adaq_backtest_core::FillPolicy) -> Self {
        match policy {
            adaq_backtest_core::FillPolicy::Maker => Self::PostOnly,
            adaq_backtest_core::FillPolicy::Taker => Self::Market,
        }
    }
}

#[derive(Debug)]
pub(crate) enum DispatchError {
    Begin(String),
    ProviderOrderIdentityMissing,
    ProviderOutcomeUncertain(String),
    ProviderRejected(String),
    OutcomeRetention(String),
}

pub(crate) fn submit(
    local: &LocalResearchState,
    bots: &BotStore,
    bot_id: &str,
    decision_id: Option<&str>,
    request: &PaperOrderRequest,
    policy: &RiskPolicy,
    kind: ProviderOrderKind,
) -> Result<(), DispatchError> {
    local
        .paper_trading
        .begin_order_with_policy(request, Some(policy), adaq_bot_runtime::unix_now_ms())
        .map_err(DispatchError::Begin)?;

    let quantity = request.quantity.to_string();
    let remote = match kind {
        ProviderOrderKind::PostOnly => local.connections.create_okx_demo_order_outcome(
            &request.user_id,
            &request.instrument,
            "post_only",
            &request.side,
            &quantity,
            Some(&request.limit_price.to_string()),
            adaq_bot_runtime::unix_now_ms(),
        ),
        ProviderOrderKind::Market => local.connections.create_okx_demo_order_outcome(
            &request.user_id,
            &request.instrument,
            "market",
            &request.side,
            &quantity,
            None,
            adaq_bot_runtime::unix_now_ms(),
        ),
    };
    let provider_order = match remote {
        Ok(order) => order,
        Err(error) if error.code == "provider_rejected" => {
            let now_ms = adaq_bot_runtime::unix_now_ms();
            local
                .paper_trading
                .record_confirmed_rejection(
                    &request.user_id,
                    &request.operation_id,
                    error.code,
                    now_ms,
                )
                .map_err(DispatchError::OutcomeRetention)?;
            bots.record_order(
                &request.user_id,
                bot_id,
                &request.operation_id,
                decision_id,
                "rejected",
                None,
            )
            .map_err(DispatchError::OutcomeRetention)?;
            bots.record_evidence(
                &request.user_id,
                bot_id,
                "execution",
                "provider-order-rejected",
                &error.redacted_message,
                decision_id,
            )
            .map_err(DispatchError::OutcomeRetention)?;
            return Err(DispatchError::ProviderRejected(error.redacted_message));
        }
        Err(error) => {
            retain_uncertain(local, bots, bot_id, decision_id, request)?;
            return Err(DispatchError::ProviderOutcomeUncertain(
                error.redacted_message,
            ));
        }
    };
    let Some(provider_order_id) = provider_order.id else {
        retain_uncertain(local, bots, bot_id, decision_id, request)?;
        return Err(DispatchError::ProviderOrderIdentityMissing);
    };
    let status = provider_order.status.unwrap_or_else(|| "accepted".into());
    local
        .paper_trading
        .record_order_result(
            &request.user_id,
            &request.operation_id,
            Some(provider_order_id.clone()),
            &status,
            None,
            adaq_bot_runtime::unix_now_ms(),
        )
        .map_err(DispatchError::OutcomeRetention)?;
    bots.record_order(
        &request.user_id,
        bot_id,
        &request.operation_id,
        decision_id,
        &status,
        Some(&provider_order_id),
    )
    .map(|_| ())
    .map_err(DispatchError::OutcomeRetention)
}

fn retain_uncertain(
    local: &LocalResearchState,
    bots: &BotStore,
    bot_id: &str,
    decision_id: Option<&str>,
    request: &PaperOrderRequest,
) -> Result<(), DispatchError> {
    let now_ms = adaq_bot_runtime::unix_now_ms();
    let paper_failed = local
        .paper_trading
        .mark_uncertain(&request.user_id, &request.operation_id, now_ms)
        .is_err();
    let bot_failed = bots
        .record_order(
            &request.user_id,
            bot_id,
            &request.operation_id,
            decision_id,
            "uncertain",
            None,
        )
        .is_err();
    if paper_failed || bot_failed {
        Err(DispatchError::OutcomeRetention(
            "Provider uncertainty evidence could not be durably retained.".into(),
        ))
    } else {
        Ok(())
    }
}
