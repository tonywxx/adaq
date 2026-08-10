use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
};

use adaq_data_core::{OhlcvBar, market::TradingCalendarSnapshot};
use adaq_indicator_engine::{
    CompiledIndicator, IndicatorColumn, IndicatorEngine, IndicatorRequest,
    MarketField as IndicatorMarketField, OhlcvSegment, ParameterValue,
};
use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    EvaluationStage, FeatureDefinition, FeatureEngineIdentity, FeatureEvaluationError,
    FeatureEvaluationErrorCode, FeatureInput, FeatureNode, FeatureObservation, FeatureOperator,
    FeatureOutput, FeaturePlan, FeatureScope, FeatureSource, FeatureUnavailabilityReason,
    FrozenBuiltInParameter, MarketField,
};

/// An exact decimal retained alongside its finite analytical representation.
/// The string is the authoritative market value; `f64` is only an evaluation input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalDecimal(String);

impl CanonicalDecimal {
    pub fn new(value: impl Into<String>) -> Result<Self, FeatureInputError> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || Decimal::from_str_exact(&value).is_err() {
            return Err(FeatureInputError::new("invalid-canonical-decimal"));
        }
        let analytical = value
            .parse::<f64>()
            .map_err(|_| FeatureInputError::new("non-finite-analytical-value"))?;
        if !analytical.is_finite() {
            return Err(FeatureInputError::new("non-finite-analytical-value"));
        }
        Ok(Self(value))
    }

    pub fn from_decimal(value: Decimal) -> Self {
        Self(value.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn analytical(&self) -> Result<f64, FeatureInputError> {
        self.0
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| FeatureInputError::new("non-finite-analytical-value"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureInputError {
    pub code: &'static str,
}

impl FeatureInputError {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

impl fmt::Display for FeatureInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for FeatureInputError {}

/// A lossless OHLCV projection. Missing fields remain missing instead of being imputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureMarketBar {
    pub open_time_ms: i64,
    pub open: Option<CanonicalDecimal>,
    pub high: Option<CanonicalDecimal>,
    pub low: Option<CanonicalDecimal>,
    pub close: Option<CanonicalDecimal>,
    pub base_volume: Option<CanonicalDecimal>,
    pub quote_volume: Option<CanonicalDecimal>,
}

impl FeatureMarketBar {
    pub fn complete(
        open_time_ms: i64,
        open: impl Into<String>,
        high: impl Into<String>,
        low: impl Into<String>,
        close: impl Into<String>,
        base_volume: impl Into<String>,
        quote_volume: impl Into<String>,
    ) -> Result<Self, FeatureInputError> {
        Ok(Self {
            open_time_ms,
            open: Some(CanonicalDecimal::new(open)?),
            high: Some(CanonicalDecimal::new(high)?),
            low: Some(CanonicalDecimal::new(low)?),
            close: Some(CanonicalDecimal::new(close)?),
            base_volume: Some(CanonicalDecimal::new(base_volume)?),
            quote_volume: Some(CanonicalDecimal::new(quote_volume)?),
        })
    }

    pub fn from_ohlcv(bar: OhlcvBar) -> Self {
        Self {
            open_time_ms: bar.open_time_ms,
            open: Some(CanonicalDecimal::from_decimal(bar.open)),
            high: Some(CanonicalDecimal::from_decimal(bar.high)),
            low: Some(CanonicalDecimal::from_decimal(bar.low)),
            close: Some(CanonicalDecimal::from_decimal(bar.close)),
            base_volume: Some(CanonicalDecimal::from_decimal(bar.base_volume)),
            quote_volume: Some(CanonicalDecimal::from_decimal(bar.quote_volume)),
        }
    }

    pub fn from_ashare_action(
        action: &adaq_data_core::a_share::AshareCorporateAction,
    ) -> Result<Vec<CorporateAction>, FeatureInputError> {
        let effective_at_ms = action
            .effective_at_ms
            .ok_or_else(|| FeatureInputError::new("corporate-action-effective-time-missing"))?;
        let instrument_id = format!("{}:{}", action.instrument.venue.id, action.instrument.code);
        let evidence_id = crate::sha256_hex(
            &serde_json::to_vec(action)
                .map_err(|_| FeatureInputError::new("corporate-action-provenance-missing"))?,
        );
        let mut actions = Vec::new();
        if let Some(shares_per_share) = &action.shares_per_share {
            actions.push(CorporateAction::split_with_evidence(
                &instrument_id,
                &evidence_id,
                effective_at_ms,
                action.available_at_ms,
                shares_per_share.clone(),
            )?);
        }
        if let Some(cash) = &action.cash_per_share {
            actions.push(CorporateAction::dividend_with_evidence(
                &instrument_id,
                &evidence_id,
                effective_at_ms,
                action.available_at_ms,
                cash.clone(),
                None,
            )?);
        }
        if actions.is_empty() {
            return Err(FeatureInputError::new("unsupported-corporate-action"));
        }
        Ok(actions)
    }

    fn field(&self, field: MarketField) -> Option<&CanonicalDecimal> {
        match field {
            MarketField::Open => self.open.as_ref(),
            MarketField::High => self.high.as_ref(),
            MarketField::Low => self.low.as_ref(),
            MarketField::Close => self.close.as_ref(),
            MarketField::BaseVolume => self.base_volume.as_ref(),
            MarketField::QuoteVolume => self.quote_volume.as_ref(),
        }
    }

    fn indicator_segment(
        bars: &[Self],
        required_fields: &[MarketField],
    ) -> Result<OhlcvSegment, FeatureInputError> {
        let collect = |field: MarketField| {
            bars.iter()
                .map(|bar| {
                    match bar.field(field) {
                        Some(value) => value.analytical(),
                        None if required_fields.contains(&field) => {
                            Err(FeatureInputError::new("missing-market-input"))
                        }
                        // The TA-Lib price holder always receives all columns, but only
                        // declared inputs are semantically required by the catalog entry.
                        None => Ok(0.0),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        };
        OhlcvSegment::new(
            collect(MarketField::Open)?,
            collect(MarketField::High)?,
            collect(MarketField::Low)?,
            collect(MarketField::Close)?,
            collect(MarketField::BaseVolume)?,
            collect(MarketField::QuoteVolume)?,
        )
        .map_err(|_| FeatureInputError::new("invalid-continuous-bar-segment"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum CorporateAction {
    Split {
        instrument_id: String,
        evidence_id: String,
        effective_at_ms: i64,
        available_at_ms: i64,
        share_multiplier: CanonicalDecimal,
        price_factor: CanonicalDecimal,
    },
    Dividend {
        instrument_id: String,
        evidence_id: String,
        effective_at_ms: i64,
        available_at_ms: i64,
        cash_per_share: CanonicalDecimal,
        reference_price: Option<CanonicalDecimal>,
    },
}

impl CorporateAction {
    pub fn split(
        instrument_id: impl Into<String>,
        effective_at_ms: i64,
        available_at_ms: i64,
        shares_per_share: impl Into<String>,
    ) -> Result<Self, FeatureInputError> {
        let instrument_id = instrument_id.into();
        let shares_per_share = shares_per_share.into();
        let evidence_id = format!(
            "manual:{}:{}:{}:{}",
            instrument_id, effective_at_ms, available_at_ms, shares_per_share
        );
        Self::split_with_evidence(
            instrument_id,
            evidence_id,
            effective_at_ms,
            available_at_ms,
            shares_per_share,
        )
    }

    pub fn split_with_evidence(
        instrument_id: impl Into<String>,
        evidence_id: impl Into<String>,
        effective_at_ms: i64,
        available_at_ms: i64,
        shares_per_share: impl Into<String>,
    ) -> Result<Self, FeatureInputError> {
        let instrument_id =
            validated_identity(instrument_id.into(), "corporate-action-instrument-missing")?;
        let evidence_id =
            validated_identity(evidence_id.into(), "corporate-action-evidence-missing")?;
        let shares_per_share = CanonicalDecimal::new(shares_per_share)?;
        let shares_per_share_decimal = Decimal::from_str_exact(shares_per_share.as_str())
            .map_err(|_| FeatureInputError::new("invalid-corporate-action-ratio"))?;
        let share_multiplier = shares_per_share_decimal
            .checked_add(Decimal::ONE)
            .ok_or_else(|| FeatureInputError::new("invalid-corporate-action-ratio"))?;
        if share_multiplier <= Decimal::ZERO {
            return Err(FeatureInputError::new("invalid-corporate-action-ratio"));
        }
        let price_factor = Decimal::ONE
            .checked_div(share_multiplier)
            .ok_or_else(|| FeatureInputError::new("invalid-corporate-action-ratio"))?;
        Ok(Self::Split {
            instrument_id,
            evidence_id,
            effective_at_ms,
            available_at_ms,
            share_multiplier: CanonicalDecimal::from_decimal(share_multiplier),
            price_factor: CanonicalDecimal::from_decimal(price_factor),
        })
    }

    pub fn dividend(
        instrument_id: impl Into<String>,
        effective_at_ms: i64,
        available_at_ms: i64,
        cash_per_share: impl Into<String>,
        reference_price: Option<CanonicalDecimal>,
    ) -> Result<Self, FeatureInputError> {
        let instrument_id = instrument_id.into();
        let cash_per_share = cash_per_share.into();
        let evidence_id = format!(
            "manual:{}:{}:{}:{}",
            instrument_id, effective_at_ms, available_at_ms, cash_per_share
        );
        Self::dividend_with_evidence(
            instrument_id,
            evidence_id,
            effective_at_ms,
            available_at_ms,
            cash_per_share,
            reference_price,
        )
    }

    pub fn dividend_with_evidence(
        instrument_id: impl Into<String>,
        evidence_id: impl Into<String>,
        effective_at_ms: i64,
        available_at_ms: i64,
        cash_per_share: impl Into<String>,
        reference_price: Option<CanonicalDecimal>,
    ) -> Result<Self, FeatureInputError> {
        Ok(Self::Dividend {
            instrument_id: validated_identity(
                instrument_id.into(),
                "corporate-action-instrument-missing",
            )?,
            evidence_id: validated_identity(
                evidence_id.into(),
                "corporate-action-evidence-missing",
            )?,
            effective_at_ms,
            available_at_ms,
            cash_per_share: CanonicalDecimal::new(cash_per_share)?,
            reference_price,
        })
    }

    fn instrument_id(&self) -> &str {
        match self {
            Self::Split { instrument_id, .. } | Self::Dividend { instrument_id, .. } => {
                instrument_id
            }
        }
    }
}

fn validated_identity(value: String, code: &'static str) -> Result<String, FeatureInputError> {
    (!value.trim().is_empty())
        .then_some(value)
        .ok_or_else(|| FeatureInputError::new(code))
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureEvaluationInput {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub available_at_ms: i64,
    pub bar: Option<FeatureMarketBar>,
    pub calendar: Option<TradingCalendarSnapshot>,
    pub corporate_actions: Vec<CorporateAction>,
}

impl FeatureEvaluationInput {
    pub fn new(
        instrument_id: impl Into<String>,
        observation_time_ms: i64,
        available_at_ms: i64,
        bar: FeatureMarketBar,
    ) -> Self {
        Self {
            instrument_id: instrument_id.into(),
            observation_time_ms,
            available_at_ms,
            bar: Some(bar),
            calendar: None,
            corporate_actions: Vec::new(),
        }
    }

    pub fn missing(
        instrument_id: impl Into<String>,
        observation_time_ms: i64,
        available_at_ms: i64,
    ) -> Self {
        Self {
            instrument_id: instrument_id.into(),
            observation_time_ms,
            available_at_ms,
            bar: None,
            calendar: None,
            corporate_actions: Vec::new(),
        }
    }

    pub fn with_calendar(mut self, calendar: TradingCalendarSnapshot) -> Self {
        self.calendar = Some(calendar);
        self
    }

    pub fn with_corporate_actions(mut self, corporate_actions: Vec<CorporateAction>) -> Self {
        self.corporate_actions = corporate_actions;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureInputEvent {
    Observation(FeatureEvaluationInput),
    BarGap {
        instrument_id: String,
        observation_time_ms: i64,
        available_at_ms: i64,
    },
    ScheduledClosure {
        instrument_id: String,
        observation_time_ms: i64,
    },
}

impl FeatureInputEvent {
    pub fn observation(input: FeatureEvaluationInput) -> Self {
        Self::Observation(input)
    }

    pub fn bar_gap(
        instrument_id: impl Into<String>,
        observation_time_ms: i64,
        available_at_ms: i64,
    ) -> Self {
        Self::BarGap {
            instrument_id: instrument_id.into(),
            observation_time_ms,
            available_at_ms,
        }
    }

    pub fn scheduled_closure(instrument_id: impl Into<String>, observation_time_ms: i64) -> Self {
        Self::ScheduledClosure {
            instrument_id: instrument_id.into(),
            observation_time_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeatureEngine {
    identity: FeatureEngineIdentity,
}

impl FeatureEngine {
    pub fn new(identity: FeatureEngineIdentity) -> Self {
        Self { identity }
    }

    pub fn native() -> Result<Self, adaq_indicator_engine::EngineError> {
        Ok(Self::new(FeatureEngineIdentity::native()?))
    }

    pub fn identity(&self) -> &FeatureEngineIdentity {
        &self.identity
    }

    pub fn evaluator(&self, plan: FeaturePlan) -> Result<FeatureEvaluator, FeatureEvaluationError> {
        if plan.engine_identity() != self.identity {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidIdentity,
                EvaluationStage::Validation,
                None,
                None,
                None,
                "feature-engine-identity-mismatch",
            ));
        }
        FeatureEvaluator::new(plan)
    }

    pub fn evaluate_batch(
        &self,
        plan: FeaturePlan,
        events: &[FeatureInputEvent],
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        self.evaluator(plan)?.evaluate_batch(events)
    }
}

pub struct FeatureEvaluator {
    plan: FeaturePlan,
    definitions: Vec<RuntimeDefinition>,
    instruments: HashMap<String, InstrumentRuntime>,
    indicator_engine: Option<IndicatorEngine>,
}

impl fmt::Debug for FeatureEvaluator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeatureEvaluator")
            .field("plan_hash", &self.plan.plan_hash())
            .field("instrument_count", &self.instruments.len())
            .finish()
    }
}

impl FeatureEvaluator {
    pub fn new(plan: FeaturePlan) -> Result<Self, FeatureEvaluationError> {
        let definitions = plan
            .definitions()
            .iter()
            .map(RuntimeDefinition::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            plan,
            definitions,
            instruments: HashMap::new(),
            indicator_engine: None,
        })
    }

    pub fn plan(&self) -> &FeaturePlan {
        &self.plan
    }

    pub fn evaluate_batch(
        &mut self,
        events: &[FeatureInputEvent],
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        let mut observations = Vec::new();
        for event in events {
            observations.extend(self.observe(event.clone())?);
        }
        Ok(observations)
    }

    pub fn observe(
        &mut self,
        event: FeatureInputEvent,
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        match event {
            FeatureInputEvent::Observation(input) => self.observe_bar(input),
            FeatureInputEvent::BarGap {
                instrument_id,
                observation_time_ms,
                available_at_ms,
            } => self.observe_gap(&instrument_id, observation_time_ms, available_at_ms),
            FeatureInputEvent::ScheduledClosure {
                instrument_id,
                observation_time_ms,
            } => self.observe_scheduled_closure(&instrument_id, observation_time_ms),
        }
    }

    fn observe_bar(
        &mut self,
        mut input: FeatureEvaluationInput,
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        if input.instrument_id.is_empty() {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(input.observation_time_ms),
                "empty-instrument-id",
            ));
        }
        let instrument_id = input.instrument_id.clone();
        let plan = &self.plan;
        let definitions = &self.definitions;
        let indicator_engine = &mut self.indicator_engine;
        let runtime = self
            .instruments
            .entry(instrument_id.clone())
            .or_insert_with(|| InstrumentRuntime::new(definitions));
        runtime.note_event(&instrument_id, input.observation_time_ms)?;
        if input
            .corporate_actions
            .iter()
            .any(|action| action.instrument_id() != instrument_id)
        {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Input,
                None,
                Some(instrument_id.clone()),
                Some(input.observation_time_ms),
                "corporate-action-instrument-mismatch",
            ));
        }
        runtime.observe_count = runtime.observe_count.saturating_add(1);
        runtime.remember_actions(&input.corporate_actions);
        input.corporate_actions = runtime.corporate_actions.clone();

        let mut observations = Vec::new();
        for (index, template) in definitions.iter().enumerate() {
            let definition = &mut runtime.definitions[index];
            definition.current.clear();
            for node_id in template.order.iter() {
                let node = definition.nodes.get(node_id).cloned().ok_or_else(|| {
                    fatal_error(
                        FeatureEvaluationErrorCode::BrokenShape,
                        EvaluationStage::Invariant,
                        Some(node_id.clone()),
                        Some(instrument_id.clone()),
                        Some(input.observation_time_ms),
                        "runtime-node-missing",
                    )
                })?;
                let values = node
                    .inputs
                    .iter()
                    .map(|feature_input| {
                        resolve_input(
                            feature_input,
                            &input,
                            &definition.current,
                            &instrument_id,
                            node_id,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let state = definition.states.get_mut(node_id).ok_or_else(|| {
                    fatal_error(
                        FeatureEvaluationErrorCode::BrokenShape,
                        EvaluationStage::Invariant,
                        Some(node_id.clone()),
                        Some(instrument_id.clone()),
                        Some(input.observation_time_ms),
                        "runtime-state-missing",
                    )
                })?;
                let value = evaluate_node(
                    &node,
                    &values,
                    &input,
                    state,
                    &plan.engine_identity(),
                    indicator_engine,
                    &instrument_id,
                )?;
                definition.current.insert(node.id.clone(), value);
            }
            for output in &template.outputs {
                let value = definition.current.get(&output.node_id).ok_or_else(|| {
                    fatal_error(
                        FeatureEvaluationErrorCode::BrokenShape,
                        EvaluationStage::Invariant,
                        Some(output.node_id.clone()),
                        Some(instrument_id.clone()),
                        Some(input.observation_time_ms),
                        "runtime-output-missing",
                    )
                })?;
                observations.push(observation_from_value(&output.name, &input, value)?);
            }
        }

        for slot in plan.slots() {
            let value = match &slot.source {
                FeatureSource::Market { field } => input
                    .bar
                    .as_ref()
                    .and_then(|bar| bar.field(*field))
                    .map(|value| {
                        value.analytical().map_or_else(
                            |_| {
                                EvalValue::Unavailable(
                                    FeatureUnavailabilityReason::MissingMarketInput,
                                )
                            },
                            |value| EvalValue::Available {
                                value,
                                available_at_ms: input.available_at_ms,
                            },
                        )
                    })
                    .unwrap_or(EvalValue::Unavailable(
                        FeatureUnavailabilityReason::MissingMarketInput,
                    )),
                FeatureSource::BuiltIn {
                    indicator,
                    output,
                    real_inputs,
                    parameters,
                } => {
                    let node = builtin_node(
                        slot.name.as_str(),
                        indicator.as_str(),
                        output.as_str(),
                        real_inputs,
                        parameters,
                    );
                    let values = node
                        .inputs
                        .iter()
                        .map(|feature_input| {
                            resolve_input(
                                feature_input,
                                &input,
                                &HashMap::new(),
                                &instrument_id,
                                &slot.name,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let state = runtime.slot_states.entry(slot.name.clone()).or_default();
                    evaluate_node(
                        &node,
                        &values,
                        &input,
                        state,
                        &plan.engine_identity(),
                        indicator_engine,
                        &instrument_id,
                    )?
                }
                FeatureSource::External { .. } | FeatureSource::Signal { .. } => {
                    EvalValue::Unavailable(FeatureUnavailabilityReason::MissingDependency)
                }
            };
            observations.push(observation_from_value(&slot.name, &input, &value)?);
        }
        Ok(observations)
    }

    fn observe_gap(
        &mut self,
        instrument_id: &str,
        observation_time_ms: i64,
        available_at_ms: i64,
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        let definitions = &self.definitions;
        let runtime = self
            .instruments
            .entry(instrument_id.to_owned())
            .or_insert_with(|| InstrumentRuntime::new(definitions));
        runtime.note_event(instrument_id, observation_time_ms)?;
        runtime.reset_gap();
        let input =
            FeatureEvaluationInput::missing(instrument_id, observation_time_ms, available_at_ms);
        self.output_names()
            .into_iter()
            .map(|name| {
                FeatureObservation::unavailable(
                    name,
                    instrument_id,
                    observation_time_ms,
                    FeatureUnavailabilityReason::BarGap,
                )
                .map_err(|error| {
                    fatal_error(
                        error.code,
                        EvaluationStage::Invariant,
                        None,
                        Some(input.instrument_id.clone()),
                        Some(input.observation_time_ms),
                        "invalid-gap-observation",
                    )
                })
            })
            .collect()
    }

    fn observe_scheduled_closure(
        &mut self,
        instrument_id: &str,
        observation_time_ms: i64,
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        if instrument_id.is_empty() {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(observation_time_ms),
                "empty-instrument-id",
            ));
        }
        let definitions = &self.definitions;
        let runtime = self
            .instruments
            .entry(instrument_id.to_owned())
            .or_insert_with(|| InstrumentRuntime::new(definitions));
        runtime.note_event(instrument_id, observation_time_ms)?;
        Ok(Vec::new())
    }

    fn output_names(&self) -> Vec<String> {
        self.plan
            .definitions()
            .iter()
            .flat_map(|definition| {
                definition
                    .outputs()
                    .iter()
                    .map(|output| output.name.clone())
            })
            .chain(self.plan.slots().iter().map(|slot| slot.name.clone()))
            .collect()
    }
}

struct RuntimeDefinition {
    nodes: HashMap<String, FeatureNode>,
    order: Vec<String>,
    outputs: Vec<FeatureOutput>,
    states: HashMap<String, RuntimeNodeState>,
    current: HashMap<String, EvalValue>,
}

impl RuntimeDefinition {
    fn new(definition: &FeatureDefinition) -> Result<Self, FeatureEvaluationError> {
        let nodes = definition
            .nodes()
            .iter()
            .map(|node| (node.id.clone(), node.clone()))
            .collect::<HashMap<_, _>>();
        let mut order = Vec::new();
        let mut visiting = HashMap::new();
        for node in definition.nodes() {
            visit_runtime_node(node.id.as_str(), &nodes, &mut visiting, &mut order)?;
        }
        let states = definition
            .nodes()
            .iter()
            .map(|node| {
                (
                    node.id.clone(),
                    RuntimeNodeState {
                        gap_affected: depends_on_market(
                            node.id.as_str(),
                            &nodes,
                            &mut HashMap::new(),
                        ),
                        ..RuntimeNodeState::default()
                    },
                )
            })
            .collect();
        Ok(Self {
            nodes,
            order,
            outputs: definition.outputs().to_vec(),
            states,
            current: HashMap::new(),
        })
    }

    fn reset_gap(&mut self) {
        for state in self.states.values_mut() {
            if state.gap_affected {
                state.reset();
            }
        }
    }
}

struct InstrumentRuntime {
    definitions: Vec<RuntimeDefinition>,
    slot_states: HashMap<String, RuntimeNodeState>,
    corporate_actions: Vec<CorporateAction>,
    observe_count: usize,
    last_event_time_ms: Option<i64>,
}

impl InstrumentRuntime {
    fn new(definition_templates: &[RuntimeDefinition]) -> Self {
        Self {
            definitions: definition_templates
                .iter()
                .map(|template| RuntimeDefinition {
                    nodes: template.nodes.clone(),
                    order: template.order.clone(),
                    outputs: template.outputs.clone(),
                    states: template
                        .states
                        .iter()
                        .map(|(id, template_state)| {
                            (
                                id.clone(),
                                RuntimeNodeState {
                                    gap_affected: template_state.gap_affected,
                                    ..RuntimeNodeState::default()
                                },
                            )
                        })
                        .collect(),
                    current: HashMap::new(),
                })
                .collect(),
            slot_states: HashMap::new(),
            corporate_actions: Vec::new(),
            observe_count: 0,
            last_event_time_ms: None,
        }
    }

    fn note_event(
        &mut self,
        instrument_id: &str,
        observation_time_ms: i64,
    ) -> Result<(), FeatureEvaluationError> {
        if self
            .last_event_time_ms
            .is_some_and(|last| observation_time_ms <= last)
        {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                Some(instrument_id.to_owned()),
                Some(observation_time_ms),
                "non-monotonic-observation-time",
            ));
        }
        self.last_event_time_ms = Some(observation_time_ms);
        Ok(())
    }

    fn remember_actions(&mut self, actions: &[CorporateAction]) {
        for action in actions {
            if !self.corporate_actions.contains(action) {
                self.corporate_actions.push(action.clone());
            }
        }
    }

    fn reset_gap(&mut self) {
        for definition in &mut self.definitions {
            definition.reset_gap();
        }
        for state in self.slot_states.values_mut() {
            state.reset();
        }
        self.observe_count = 0;
    }
}

#[derive(Default)]
struct RuntimeNodeState {
    history: VecDeque<EvalValue>,
    market_history: Vec<FeatureMarketBar>,
    market_available_at: Vec<i64>,
    compiled_indicator: Option<CompiledIndicator>,
    bars_since_reset: usize,
    gap_affected: bool,
}

impl RuntimeNodeState {
    fn reset(&mut self) {
        self.history.clear();
        self.market_history.clear();
        self.market_available_at.clear();
        self.bars_since_reset = 0;
    }
}

#[derive(Debug, Clone, PartialEq)]
enum EvalValue {
    Available { value: f64, available_at_ms: i64 },
    Unavailable(FeatureUnavailabilityReason),
}

impl EvalValue {
    fn available(value: f64, available_at_ms: i64) -> Self {
        Self::Available {
            value,
            available_at_ms,
        }
    }

    fn value(&self) -> Option<f64> {
        match self {
            Self::Available { value, .. } => Some(*value),
            Self::Unavailable(_) => None,
        }
    }

    fn available_at_ms(&self) -> Option<i64> {
        match self {
            Self::Available {
                available_at_ms, ..
            } => Some(*available_at_ms),
            Self::Unavailable(_) => None,
        }
    }
}

fn resolve_input(
    input: &FeatureInput,
    observation: &FeatureEvaluationInput,
    current: &HashMap<String, EvalValue>,
    instrument_id: &str,
    node_id: &str,
) -> Result<EvalValue, FeatureEvaluationError> {
    match input {
        FeatureInput::Market { field } => {
            match observation.bar.as_ref().and_then(|bar| bar.field(*field)) {
                Some(value) => value
                    .analytical()
                    .map(|value| EvalValue::available(value, observation.available_at_ms))
                    .map_err(|error| {
                        fatal_error(
                            FeatureEvaluationErrorCode::InvalidInvariant,
                            EvaluationStage::Input,
                            Some(node_id.to_owned()),
                            Some(instrument_id.to_owned()),
                            Some(observation.observation_time_ms),
                            error.code,
                        )
                    }),
                None => Ok(EvalValue::Unavailable(
                    FeatureUnavailabilityReason::MissingMarketInput,
                )),
            }
        }
        FeatureInput::Node {
            node_id: dependency,
        } => Ok(current
            .get(dependency)
            .cloned()
            .unwrap_or(EvalValue::Unavailable(
                FeatureUnavailabilityReason::MissingDependency,
            ))),
        FeatureInput::Artifact { .. } => Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::ArtifactMissingInstrument,
        )),
    }
}

fn evaluate_node(
    node: &FeatureNode,
    inputs: &[EvalValue],
    observation: &FeatureEvaluationInput,
    state: &mut RuntimeNodeState,
    expected_identity: &FeatureEngineIdentity,
    indicator_engine: &mut Option<IndicatorEngine>,
    instrument_id: &str,
) -> Result<EvalValue, FeatureEvaluationError> {
    if let Some(reason) = input_unavailability(node, inputs) {
        state.reset();
        return Ok(EvalValue::Unavailable(reason));
    }
    state.bars_since_reset = state.bars_since_reset.saturating_add(1);
    let available_at_ms = inputs
        .iter()
        .filter_map(EvalValue::available_at_ms)
        .chain(std::iter::once(observation.available_at_ms))
        .max()
        .unwrap_or(observation.available_at_ms);
    let value = match &node.operator {
        FeatureOperator::CheckedArithmetic => checked_arithmetic(inputs, node, available_at_ms),
        FeatureOperator::Indicator { id } => evaluate_indicator(
            node,
            id,
            observation,
            state,
            expected_identity,
            indicator_engine,
            instrument_id,
        ),
        FeatureOperator::BackwardSimpleReturn => {
            backward_return(inputs, node, state, false, available_at_ms)
        }
        FeatureOperator::BackwardLogReturn => {
            backward_return(inputs, node, state, true, available_at_ms)
        }
        FeatureOperator::RollingMean
        | FeatureOperator::RollingPopulationStandardDeviation
        | FeatureOperator::RollingMinimum
        | FeatureOperator::RollingMaximum
        | FeatureOperator::RollingQuoteVolume => rolling(inputs, node, state, available_at_ms),
        FeatureOperator::RealizedVolatility => {
            realized_volatility(inputs, node, state, available_at_ms)
        }
        FeatureOperator::QuoteVolume => unary(inputs, available_at_ms),
        FeatureOperator::ZeroVolume => zero_volume(inputs, available_at_ms),
        FeatureOperator::AmihudIlliquidity => amihud(inputs, available_at_ms),
        FeatureOperator::TradingDayOfWeek
        | FeatureOperator::TradingMonth
        | FeatureOperator::MinutesFromSessionOpen
        | FeatureOperator::MinutesToSessionClose
        | FeatureOperator::SessionProgress => calendar_operator(&node.operator, observation),
        FeatureOperator::OneHot => one_hot(inputs, node, available_at_ms),
        FeatureOperator::Sine | FeatureOperator::Cosine => cycle(inputs, node, available_at_ms),
        FeatureOperator::CausalSplitAdjustment => {
            corporate_action_value(inputs, node, observation, available_at_ms, false)
        }
        FeatureOperator::DividendTotalReturn => {
            corporate_action_value(inputs, node, observation, available_at_ms, true)
        }
        FeatureOperator::Standardization | FeatureOperator::Winsorization => Ok(
            EvalValue::Unavailable(FeatureUnavailabilityReason::ArtifactMissingInstrument),
        ),
        FeatureOperator::CrossSectionalRank
        | FeatureOperator::CrossSectionalPercentile
        | FeatureOperator::CrossSectionalZScore => Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        )),
    }?;
    let reset = matches!(value, EvalValue::Unavailable(reason) if reason != FeatureUnavailabilityReason::Warmup);
    if reset {
        state.reset();
        return Ok(value);
    }
    Ok(if state.bars_since_reset <= node.warmup_bars as usize {
        EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup)
    } else {
        value
    })
}

fn input_unavailability(
    node: &FeatureNode,
    inputs: &[EvalValue],
) -> Option<FeatureUnavailabilityReason> {
    inputs
        .iter()
        .enumerate()
        .find_map(|(index, value)| match value {
            EvalValue::Available { .. } => None,
            EvalValue::Unavailable(reason) => Some(
                if matches!(node.inputs.get(index), Some(FeatureInput::Market { .. })) {
                    *reason
                } else {
                    FeatureUnavailabilityReason::MissingDependency
                },
            ),
        })
}

fn checked_arithmetic(
    inputs: &[EvalValue],
    node: &FeatureNode,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let values = inputs
        .iter()
        .filter_map(EvalValue::value)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    }
    let operation = node
        .parameters
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("add");
    let result = match operation {
        "add" => values.iter().copied().try_fold(0.0, finite_add),
        "subtract" => values[1..]
            .iter()
            .copied()
            .try_fold(values[0], finite_subtract),
        "multiply" => values.iter().copied().try_fold(1.0, finite_multiply),
        "divide" => values[1..]
            .iter()
            .copied()
            .try_fold(values[0], finite_divide),
        "min" => Ok(values.iter().copied().fold(f64::INFINITY, f64::min)),
        "max" => Ok(values.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        _ => {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Operator,
                Some(node.id.clone()),
                None,
                None,
                "unknown-arithmetic-operation",
            ));
        }
    };
    Ok(match result {
        Ok(value) if value.is_finite() => EvalValue::available(value, available_at_ms),
        _ => EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic),
    })
}

fn finite_add(left: f64, right: f64) -> Result<f64, ()> {
    let result = left + right;
    result.is_finite().then_some(result).ok_or(())
}

fn finite_subtract(left: f64, right: f64) -> Result<f64, ()> {
    let result = left - right;
    result.is_finite().then_some(result).ok_or(())
}

fn finite_multiply(left: f64, right: f64) -> Result<f64, ()> {
    let result = left * right;
    result.is_finite().then_some(result).ok_or(())
}

fn finite_divide(left: f64, right: f64) -> Result<f64, ()> {
    if right == 0.0 {
        return Err(());
    }
    let result = left / right;
    result.is_finite().then_some(result).ok_or(())
}

fn unary(inputs: &[EvalValue], available_at_ms: i64) -> Result<EvalValue, FeatureEvaluationError> {
    Ok(inputs.first().and_then(EvalValue::value).map_or(
        EvalValue::Unavailable(FeatureUnavailabilityReason::MissingDependency),
        |value| EvalValue::available(value, available_at_ms),
    ))
}

fn backward_return(
    inputs: &[EvalValue],
    node: &FeatureNode,
    state: &mut RuntimeNodeState,
    logarithmic: bool,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let current = inputs.first().and_then(EvalValue::value).ok_or_else(|| {
        fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Operator,
            Some(node.id.clone()),
            None,
            None,
            "return-input-missing",
        )
    })?;
    let period = positive_parameter(node, "period", 1)?;
    let previous = state
        .history
        .iter()
        .rev()
        .nth(period.saturating_sub(1))
        .cloned();
    state
        .history
        .push_back(EvalValue::available(current, available_at_ms));
    while state.history.len() > period.saturating_add(1) {
        state.history.pop_front();
    }
    let Some(previous) = previous else {
        return Ok(EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup));
    };
    let previous_available_at = previous.available_at_ms().unwrap_or(available_at_ms);
    let Some(previous) = previous.value() else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    let available_at_ms = available_at_ms.max(previous_available_at);
    if logarithmic {
        if current <= 0.0 || previous <= 0.0 {
            return Ok(EvalValue::Unavailable(
                FeatureUnavailabilityReason::UndefinedArithmetic,
            ));
        }
        let value = (current / previous).ln();
        Ok(if value.is_finite() {
            EvalValue::available(value, available_at_ms)
        } else {
            EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
        })
    } else if previous == 0.0 {
        Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::UndefinedArithmetic,
        ))
    } else {
        let value = current / previous - 1.0;
        Ok(if value.is_finite() {
            EvalValue::available(value, available_at_ms)
        } else {
            EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
        })
    }
}

fn rolling(
    inputs: &[EvalValue],
    node: &FeatureNode,
    state: &mut RuntimeNodeState,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let current = inputs.first().and_then(EvalValue::value).ok_or_else(|| {
        fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Operator,
            Some(node.id.clone()),
            None,
            None,
            "rolling-input-missing",
        )
    })?;
    let window = positive_parameter(node, "window", node.warmup_bars.saturating_add(1))?;
    state
        .history
        .push_back(EvalValue::available(current, available_at_ms));
    while state.history.len() > window {
        state.history.pop_front();
    }
    if state.history.len() < window {
        return Ok(EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup));
    }
    let values = state
        .history
        .iter()
        .filter_map(EvalValue::value)
        .collect::<Vec<_>>();
    if values.len() != window {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    }
    let available_at_ms = state
        .history
        .iter()
        .filter_map(EvalValue::available_at_ms)
        .max()
        .unwrap_or(available_at_ms);
    let value = match &node.operator {
        FeatureOperator::RollingMean | FeatureOperator::RollingQuoteVolume => {
            values.iter().sum::<f64>() / window as f64
        }
        FeatureOperator::RollingPopulationStandardDeviation => {
            let mean = values.iter().sum::<f64>() / window as f64;
            (values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / window as f64)
                .sqrt()
        }
        FeatureOperator::RollingMinimum => values.iter().copied().fold(f64::INFINITY, f64::min),
        FeatureOperator::RollingMaximum => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        _ => unreachable!(),
    };
    Ok(if value.is_finite() {
        EvalValue::available(value, available_at_ms)
    } else {
        EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
    })
}

fn realized_volatility(
    inputs: &[EvalValue],
    node: &FeatureNode,
    state: &mut RuntimeNodeState,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let current = inputs.first().and_then(EvalValue::value).ok_or_else(|| {
        fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Operator,
            Some(node.id.clone()),
            None,
            None,
            "realized-volatility-input-missing",
        )
    })?;
    let window = positive_parameter(node, "window", node.warmup_bars.max(1))?;
    state
        .history
        .push_back(EvalValue::available(current, available_at_ms));
    while state.history.len() > window.saturating_add(1) {
        state.history.pop_front();
    }
    if state.history.len() < window.saturating_add(1) {
        return Ok(EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup));
    }
    let prices = state
        .history
        .iter()
        .filter_map(EvalValue::value)
        .collect::<Vec<_>>();
    if prices.len() != window.saturating_add(1) || prices.iter().any(|price| *price <= 0.0) {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::UndefinedArithmetic,
        ));
    }
    let returns = prices
        .windows(2)
        .map(|pair| {
            let value = (pair[1] / pair[0]).ln();
            value.is_finite().then_some(value)
        })
        .collect::<Option<Vec<_>>>();
    let Some(returns) = returns else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::UndefinedArithmetic,
        ));
    };
    let mean = returns.iter().sum::<f64>() / window as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / window as f64;
    let available_at_ms = state
        .history
        .iter()
        .filter_map(EvalValue::available_at_ms)
        .max()
        .unwrap_or(available_at_ms);
    let value = variance.sqrt();
    Ok(if value.is_finite() {
        EvalValue::available(value, available_at_ms)
    } else {
        EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
    })
}

fn zero_volume(
    inputs: &[EvalValue],
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(value) = inputs.first().and_then(EvalValue::value) else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    if value < 0.0 {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::UndefinedArithmetic,
        ));
    }
    Ok(EvalValue::available(
        (value == 0.0) as u8 as f64,
        available_at_ms,
    ))
}

fn amihud(inputs: &[EvalValue], available_at_ms: i64) -> Result<EvalValue, FeatureEvaluationError> {
    let (Some(price_move), Some(quote_volume)) = (
        inputs.first().and_then(EvalValue::value),
        inputs.get(1).and_then(EvalValue::value),
    ) else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    if quote_volume <= 0.0 {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::UndefinedArithmetic,
        ));
    }
    let value = price_move.abs() / quote_volume;
    Ok(if value.is_finite() {
        EvalValue::available(value, available_at_ms)
    } else {
        EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
    })
}

fn calendar_operator(
    operator: &FeatureOperator,
    observation: &FeatureEvaluationInput,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(calendar) = observation.calendar.as_ref() else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::InsufficientCoverage,
        ));
    };
    let session_operator = matches!(
        operator,
        FeatureOperator::MinutesFromSessionOpen
            | FeatureOperator::MinutesToSessionClose
            | FeatureOperator::SessionProgress
    );
    if session_operator
        && calendar
            .is_scheduled_non_trading(observation.observation_time_ms)
            .map_err(|_| FeatureUnavailabilityReason::InsufficientCoverage)
            .is_ok_and(|closed| closed)
    {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::InsufficientCoverage,
        ));
    }
    let date = calendar
        .trading_date_of(observation.observation_time_ms)
        .map_err(|_| {
            FeatureEvaluationError::observation(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Availability,
                &observation.instrument_id,
                observation.observation_time_ms,
            )
        })?;
    let value = match operator {
        FeatureOperator::TradingDayOfWeek => date
            .to_naive_date()
            .map_err(|_| FeatureUnavailabilityReason::InsufficientCoverage)
            .map(|date| date.weekday().num_days_from_monday() as f64),
        FeatureOperator::TradingMonth => Ok(date.month as f64),
        FeatureOperator::MinutesFromSessionOpen => {
            session_boundary_minutes(calendar, date, observation.observation_time_ms, true)
        }
        FeatureOperator::MinutesToSessionClose => {
            session_boundary_minutes(calendar, date, observation.observation_time_ms, false)
        }
        FeatureOperator::SessionProgress => {
            session_progress(calendar, date, observation.observation_time_ms)
        }
        _ => unreachable!(),
    };
    Ok(match value {
        Ok(value) if value.is_finite() => EvalValue::available(value, observation.available_at_ms),
        Ok(_) => EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic),
        Err(reason) => EvalValue::Unavailable(reason),
    })
}

fn session_boundary_minutes(
    calendar: &TradingCalendarSnapshot,
    date: adaq_data_core::market::TradingDate,
    observation_time_ms: i64,
    from_open: bool,
) -> Result<f64, FeatureUnavailabilityReason> {
    let closures = calendar
        .day(date)
        .map(|day| day.closures.as_slice())
        .unwrap_or(&[]);
    for window in calendar
        .session_windows_utc(date)
        .map_err(|_| FeatureUnavailabilityReason::InsufficientCoverage)?
        .into_iter()
        .filter(|window| {
            matches!(
                window.phase,
                adaq_data_core::market::SessionPhase::Continuous
                    | adaq_data_core::market::SessionPhase::Auction
            )
        })
    {
        if observation_time_ms < window.start_ms || observation_time_ms >= window.end_ms {
            continue;
        }
        let fragments = without_closures(window, closures);
        if !fragments.iter().any(|fragment| {
            observation_time_ms >= fragment.start_ms && observation_time_ms < fragment.end_ms
        }) {
            return Err(FeatureUnavailabilityReason::InsufficientCoverage);
        }
        let milliseconds: i64 = if from_open {
            fragments
                .iter()
                .map(|fragment| {
                    if observation_time_ms >= fragment.end_ms {
                        fragment.end_ms - fragment.start_ms
                    } else if observation_time_ms > fragment.start_ms {
                        observation_time_ms - fragment.start_ms
                    } else {
                        0
                    }
                })
                .sum()
        } else {
            fragments
                .iter()
                .map(|fragment| {
                    if observation_time_ms <= fragment.start_ms {
                        fragment.end_ms - fragment.start_ms
                    } else if observation_time_ms < fragment.end_ms {
                        fragment.end_ms - observation_time_ms
                    } else {
                        0
                    }
                })
                .sum()
        };
        return Ok(milliseconds as f64 / 60_000.0);
    }
    Err(FeatureUnavailabilityReason::InsufficientCoverage)
}

fn session_progress(
    calendar: &TradingCalendarSnapshot,
    date: adaq_data_core::market::TradingDate,
    observation_time_ms: i64,
) -> Result<f64, FeatureUnavailabilityReason> {
    let continuous = eligible_continuous_windows(calendar, date)?;
    let total = continuous
        .iter()
        .map(|window| (window.end_ms - window.start_ms).max(0) as f64)
        .sum::<f64>();
    if total <= 0.0 {
        return Err(FeatureUnavailabilityReason::InsufficientCoverage);
    }
    let elapsed = continuous.iter().fold(0.0, |elapsed, window| {
        if observation_time_ms >= window.end_ms {
            elapsed + (window.end_ms - window.start_ms).max(0) as f64
        } else if observation_time_ms > window.start_ms {
            elapsed + (observation_time_ms - window.start_ms) as f64
        } else {
            elapsed
        }
    });
    Ok((elapsed / total).clamp(0.0, 1.0))
}

fn eligible_continuous_windows(
    calendar: &TradingCalendarSnapshot,
    date: adaq_data_core::market::TradingDate,
) -> Result<Vec<adaq_data_core::market::SessionWindowUtc>, FeatureUnavailabilityReason> {
    let windows = calendar
        .session_windows_utc(date)
        .map_err(|_| FeatureUnavailabilityReason::InsufficientCoverage)?;
    let closures = calendar
        .day(date)
        .map(|day| day.closures.as_slice())
        .unwrap_or(&[]);
    let mut eligible = Vec::new();
    for window in windows
        .into_iter()
        .filter(|window| window.phase == adaq_data_core::market::SessionPhase::Continuous)
    {
        eligible.extend(without_closures(window, closures));
    }
    Ok(eligible)
}

fn without_closures(
    window: adaq_data_core::market::SessionWindowUtc,
    closures: &[adaq_data_core::market::ScheduledClosure],
) -> Vec<adaq_data_core::market::SessionWindowUtc> {
    let mut fragments = vec![(window.start_ms, window.end_ms)];
    for closure in closures {
        let mut next = Vec::new();
        for (start_ms, end_ms) in fragments {
            if closure.end_ms <= start_ms || closure.start_ms >= end_ms {
                next.push((start_ms, end_ms));
                continue;
            }
            if start_ms < closure.start_ms {
                next.push((start_ms, closure.start_ms.min(end_ms)));
            }
            if closure.end_ms < end_ms {
                next.push((closure.end_ms.max(start_ms), end_ms));
            }
        }
        fragments = next;
    }
    fragments
        .into_iter()
        .filter_map(|(start_ms, end_ms)| {
            (start_ms < end_ms).then_some(adaq_data_core::market::SessionWindowUtc {
                phase: window.phase,
                start_ms,
                end_ms,
            })
        })
        .collect()
}

fn one_hot(
    inputs: &[EvalValue],
    node: &FeatureNode,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(value) = inputs.first().and_then(EvalValue::value) else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    let category = node
        .parameters
        .get("category")
        .or_else(|| node.parameters.get("value"))
        .and_then(value_as_f64)
        .ok_or_else(|| {
            fatal_error(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Operator,
                Some(node.id.clone()),
                None,
                None,
                "invalid-one-hot-category",
            )
        })?;
    Ok(EvalValue::available(
        (value == category) as u8 as f64,
        available_at_ms,
    ))
}

fn cycle(
    inputs: &[EvalValue],
    node: &FeatureNode,
    available_at_ms: i64,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(value) = inputs.first().and_then(EvalValue::value) else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    let period = node
        .parameters
        .get("period")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if !period.is_finite() || period <= 0.0 {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::OperatorFailure,
            EvaluationStage::Operator,
            Some(node.id.clone()),
            None,
            None,
            "invalid-cycle-period",
        ));
    }
    let angle = std::f64::consts::TAU * value / period;
    let result = match node.operator {
        FeatureOperator::Sine => angle.sin(),
        FeatureOperator::Cosine => angle.cos(),
        _ => unreachable!(),
    };
    Ok(EvalValue::available(result, available_at_ms))
}

fn corporate_action_value(
    inputs: &[EvalValue],
    node: &FeatureNode,
    observation: &FeatureEvaluationInput,
    available_at_ms: i64,
    dividends: bool,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(mut value) = inputs.first().and_then(EvalValue::value) else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    let mut action_available_at = available_at_ms;
    for action in &observation.corporate_actions {
        match action {
            CorporateAction::Split {
                effective_at_ms,
                available_at_ms,
                price_factor,
                share_multiplier,
                ..
            } if !dividends && *effective_at_ms <= observation.observation_time_ms => {
                if *available_at_ms > observation.observation_time_ms {
                    return Ok(EvalValue::Unavailable(
                        FeatureUnavailabilityReason::CorporateActionUnavailable,
                    ));
                }
                let factor = split_factor(node, price_factor, share_multiplier)?;
                let factor = factor.analytical().map_err(|_| {
                    FeatureEvaluationError::observation(
                        FeatureEvaluationErrorCode::InvalidInvariant,
                        EvaluationStage::Input,
                        &observation.instrument_id,
                        observation.observation_time_ms,
                    )
                })?;
                if factor <= 0.0 || !factor.is_finite() {
                    return Ok(EvalValue::Unavailable(
                        FeatureUnavailabilityReason::UndefinedArithmetic,
                    ));
                }
                value *= factor;
                action_available_at = action_available_at.max(*available_at_ms);
            }
            CorporateAction::Dividend {
                effective_at_ms,
                available_at_ms,
                cash_per_share,
                reference_price,
                ..
            } if dividends && *effective_at_ms <= observation.observation_time_ms => {
                if *available_at_ms > observation.observation_time_ms {
                    return Ok(EvalValue::Unavailable(
                        FeatureUnavailabilityReason::CorporateActionUnavailable,
                    ));
                }
                let cash = cash_per_share.analytical().map_err(|_| {
                    FeatureEvaluationError::observation(
                        FeatureEvaluationErrorCode::InvalidInvariant,
                        EvaluationStage::Input,
                        &observation.instrument_id,
                        observation.observation_time_ms,
                    )
                })?;
                let reference = reference_price
                    .as_ref()
                    .map(CanonicalDecimal::analytical)
                    .transpose()
                    .map_err(|_| {
                        FeatureEvaluationError::observation(
                            FeatureEvaluationErrorCode::InvalidInvariant,
                            EvaluationStage::Input,
                            &observation.instrument_id,
                            observation.observation_time_ms,
                        )
                    })?
                    .unwrap_or(value);
                if cash < 0.0 || reference <= 0.0 || !reference.is_finite() {
                    return Ok(EvalValue::Unavailable(
                        FeatureUnavailabilityReason::UndefinedArithmetic,
                    ));
                }
                value *= 1.0 + cash / reference;
                action_available_at = action_available_at.max(*available_at_ms);
            }
            _ => {}
        }
    }
    Ok(if value.is_finite() {
        EvalValue::available(value, action_available_at)
    } else {
        EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
    })
}

fn split_factor(
    node: &FeatureNode,
    price_factor: &CanonicalDecimal,
    share_multiplier: &CanonicalDecimal,
) -> Result<CanonicalDecimal, FeatureEvaluationError> {
    let unit = node
        .parameters
        .get("unit")
        .and_then(Value::as_str)
        .or_else(|| {
            node.inputs.first().and_then(|input| match input {
                FeatureInput::Market { field } => match field {
                    MarketField::BaseVolume => Some("quantity"),
                    MarketField::QuoteVolume => Some("value"),
                    _ => Some("price"),
                },
                FeatureInput::Node { .. } | FeatureInput::Artifact { .. } => None,
            })
        })
        .unwrap_or("price");
    match unit {
        "price" => Ok(price_factor.clone()),
        "quantity" => Ok(share_multiplier.clone()),
        "value" => Ok(CanonicalDecimal::from_decimal(Decimal::ONE)),
        _ => Err(fatal_error(
            FeatureEvaluationErrorCode::OperatorFailure,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            None,
            None,
            "invalid-split-unit",
        )),
    }
}

fn evaluate_indicator(
    node: &FeatureNode,
    indicator_id: &str,
    observation: &FeatureEvaluationInput,
    state: &mut RuntimeNodeState,
    expected_identity: &FeatureEngineIdentity,
    indicator_engine: &mut Option<IndicatorEngine>,
    instrument_id: &str,
) -> Result<EvalValue, FeatureEvaluationError> {
    let Some(bar) = observation.bar.as_ref() else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingMarketInput,
        ));
    };
    let engine = match indicator_engine {
        Some(engine) => engine,
        None => {
            let engine = IndicatorEngine::initialize().map_err(|error| {
                fatal_error(
                    FeatureEvaluationErrorCode::OperatorFailure,
                    EvaluationStage::Operator,
                    Some(node.id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(observation.observation_time_ms),
                    error.code(),
                )
            })?;
            *indicator_engine = Some(engine);
            indicator_engine
                .as_ref()
                .expect("indicator engine just initialized")
        }
    };
    if FeatureEngineIdentity::from_indicator(engine.identity()) != *expected_identity {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::InvalidIdentity,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "indicator-engine-identity-mismatch",
        ));
    }
    let definition = engine
        .catalog()
        .indicators
        .iter()
        .find(|definition| definition.id == indicator_id)
        .ok_or_else(|| {
            fatal_error(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Validation,
                Some(node.id.clone()),
                Some(instrument_id.to_owned()),
                Some(observation.observation_time_ms),
                "unknown-indicator",
            )
        })?;
    if node.inputs.len() != definition.inputs.len() {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "indicator-input-count-mismatch",
        ));
    }
    let required_fields = node
        .inputs
        .iter()
        .zip(definition.inputs.iter())
        .map(|(input, definition_input)| {
            let FeatureInput::Market { field } = input else {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidInvariant,
                    EvaluationStage::Validation,
                    Some(node.id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(observation.observation_time_ms),
                    "indicator-requires-market-inputs",
                ));
            };
            let valid = match definition_input.kind.as_str() {
                "Double Array" | "Volume" => definition_input
                    .allowed_fields
                    .iter()
                    .any(|allowed| allowed == field.as_str()),
                fixed => fixed.eq_ignore_ascii_case(field.as_str()),
            };
            if !valid {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidInvariant,
                    EvaluationStage::Validation,
                    Some(node.id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(observation.observation_time_ms),
                    "indicator-input-binding-mismatch",
                ));
            }
            Ok(*field)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let real_inputs = required_fields
        .iter()
        .zip(definition.inputs.iter())
        .filter_map(|(field, definition_input)| {
            matches!(definition_input.kind.as_str(), "Double Array" | "Volume")
                .then_some(to_indicator_field(*field))
        })
        .collect::<Vec<_>>();
    if definition.inputs.is_empty() {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::InvalidInvariant,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "indicator-inputs-empty",
        ));
    }
    if state.compiled_indicator.is_none() {
        let output = node
            .parameters
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| definition.outputs.first().map(|output| output.id.clone()))
            .ok_or_else(|| {
                fatal_error(
                    FeatureEvaluationErrorCode::OperatorFailure,
                    EvaluationStage::Validation,
                    Some(node.id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(observation.observation_time_ms),
                    "indicator-output-missing",
                )
            })?;
        let parameters = node
            .parameters
            .iter()
            .filter(|(name, _)| name.as_str() != "output")
            .map(|(name, value)| {
                value_as_indicator_parameter(value)
                    .map(|value| (name.clone(), value))
                    .ok_or_else(|| {
                        fatal_error(
                            FeatureEvaluationErrorCode::OperatorFailure,
                            EvaluationStage::Validation,
                            Some(node.id.clone()),
                            Some(instrument_id.to_owned()),
                            Some(observation.observation_time_ms),
                            "invalid-indicator-parameter",
                        )
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let compiled = engine
            .compile(IndicatorRequest {
                indicator_id: indicator_id.to_owned(),
                real_inputs: real_inputs.clone(),
                parameters,
                outputs: vec![output],
            })
            .map_err(|error| {
                fatal_error(
                    FeatureEvaluationErrorCode::OperatorFailure,
                    EvaluationStage::Operator,
                    Some(node.id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(observation.observation_time_ms),
                    error.code(),
                )
            })?;
        state.compiled_indicator = Some(compiled);
    }
    state.market_history.push(bar.clone());
    state.market_available_at.push(observation.available_at_ms);
    let max_history = state
        .compiled_indicator
        .as_ref()
        .expect("compiled indicator")
        .lookback()
        .saturating_add(1);
    while state.market_history.len() > max_history {
        state.market_history.remove(0);
        state.market_available_at.remove(0);
    }
    // ponytail: TA-Lib exposes only batch evaluation; retain one lookback tail to bound memory, upgrading to a native streaming API if profiling makes O(window) per bar materialization a bottleneck.
    let segment = FeatureMarketBar::indicator_segment(&state.market_history, &required_fields)
        .map_err(|error| {
            fatal_error(
                FeatureEvaluationErrorCode::BrokenShape,
                EvaluationStage::Input,
                Some(node.id.clone()),
                Some(instrument_id.to_owned()),
                Some(observation.observation_time_ms),
                error.code,
            )
        })?;
    let outputs = engine
        .evaluate(
            state
                .compiled_indicator
                .as_ref()
                .expect("compiled indicator"),
            &segment,
        )
        .map_err(|error| {
            let code = if matches!(
                error,
                adaq_indicator_engine::EngineError::NonFiniteOutput { .. }
            ) {
                FeatureEvaluationErrorCode::NonFiniteOutput
            } else {
                FeatureEvaluationErrorCode::OperatorFailure
            };
            fatal_error(
                code,
                if code == FeatureEvaluationErrorCode::NonFiniteOutput {
                    EvaluationStage::Invariant
                } else {
                    EvaluationStage::Operator
                },
                Some(node.id.clone()),
                Some(instrument_id.to_owned()),
                Some(observation.observation_time_ms),
                error.code(),
            )
        })?;
    let Some((_, column)) = outputs.into_iter().next() else {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Invariant,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "indicator-output-empty",
        ));
    };
    let value = match column {
        IndicatorColumn::Real(values) => values.last().copied().flatten(),
        IndicatorColumn::Integer(values) => values.last().copied().flatten().map(f64::from),
    };
    let available_at_ms = state
        .market_available_at
        .iter()
        .copied()
        .max()
        .unwrap_or(observation.available_at_ms);
    Ok(value.map_or(
        EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup),
        |value| EvalValue::available(value, available_at_ms),
    ))
}

fn to_indicator_field(field: MarketField) -> IndicatorMarketField {
    match field {
        MarketField::Open => IndicatorMarketField::Open,
        MarketField::High => IndicatorMarketField::High,
        MarketField::Low => IndicatorMarketField::Low,
        MarketField::Close => IndicatorMarketField::Close,
        MarketField::BaseVolume => IndicatorMarketField::BaseVolume,
        MarketField::QuoteVolume => IndicatorMarketField::QuoteVolume,
    }
}

fn value_as_indicator_parameter(value: &Value) -> Option<ParameterValue> {
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .map(ParameterValue::Integer)
        .or_else(|| value.as_f64().map(ParameterValue::Real))
        .or_else(|| {
            value
                .as_str()
                .map(|value| ParameterValue::Enum(value.to_owned()))
        })
}

fn positive_parameter(
    node: &FeatureNode,
    name: &str,
    default: u32,
) -> Result<usize, FeatureEvaluationError> {
    let value = node
        .parameters
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or(default as u64);
    usize::try_from(value)
        .ok()
        .filter(|value| *value > 0 && *value <= crate::MAX_EFFECTIVE_WARMUP_BARS as usize)
        .ok_or_else(|| {
            fatal_error(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Validation,
                Some(node.id.clone()),
                None,
                None,
                "invalid-positive-parameter",
            )
        })
}

fn value_as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
        .filter(|value| value.is_finite())
}

fn builtin_node(
    name: &str,
    indicator: &str,
    output: &str,
    real_inputs: &[MarketField],
    parameters: &BTreeMap<String, FrozenBuiltInParameter>,
) -> FeatureNode {
    let mut node_parameters = parameters
        .iter()
        .map(|(name, value)| {
            let value = match value {
                FrozenBuiltInParameter::Integer(value) => json!(value),
                FrozenBuiltInParameter::Real(value) | FrozenBuiltInParameter::Enum(value) => {
                    json!(value)
                }
            };
            (name.clone(), value)
        })
        .collect::<BTreeMap<_, _>>();
    node_parameters.insert("output".into(), Value::String(output.to_owned()));
    FeatureNode {
        id: name.to_owned(),
        operator: FeatureOperator::Indicator {
            id: indicator.to_owned(),
        },
        scope: FeatureScope::Pointwise,
        inputs: real_inputs
            .iter()
            .map(|field| FeatureInput::Market { field: *field })
            .collect(),
        parameters: node_parameters,
        warmup_bars: 0,
    }
}

fn observation_from_value(
    output_name: &str,
    input: &FeatureEvaluationInput,
    value: &EvalValue,
) -> Result<FeatureObservation, FeatureEvaluationError> {
    match value {
        EvalValue::Available {
            value,
            available_at_ms,
        } => FeatureObservation::available(
            output_name,
            &input.instrument_id,
            input.observation_time_ms,
            *value,
            *available_at_ms,
        ),
        EvalValue::Unavailable(reason) => FeatureObservation::unavailable(
            output_name,
            &input.instrument_id,
            input.observation_time_ms,
            *reason,
        ),
    }
}

fn visit_runtime_node(
    id: &str,
    nodes: &HashMap<String, FeatureNode>,
    states: &mut HashMap<String, u8>,
    order: &mut Vec<String>,
) -> Result<(), FeatureEvaluationError> {
    match states.get(id).copied() {
        Some(1) => {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidInvariant,
                EvaluationStage::Validation,
                Some(id.to_owned()),
                None,
                None,
                "dependency-cycle",
            ));
        }
        Some(2) => return Ok(()),
        _ => {}
    }
    let node = nodes.get(id).ok_or_else(|| {
        fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Validation,
            Some(id.to_owned()),
            None,
            None,
            "runtime-dependency-missing",
        )
    })?;
    states.insert(id.to_owned(), 1);
    for input in &node.inputs {
        if let FeatureInput::Node { node_id } = input {
            visit_runtime_node(node_id, nodes, states, order)?;
        }
    }
    states.insert(id.to_owned(), 2);
    order.push(id.to_owned());
    Ok(())
}

fn depends_on_market(
    id: &str,
    nodes: &HashMap<String, FeatureNode>,
    memo: &mut HashMap<String, bool>,
) -> bool {
    if let Some(value) = memo.get(id) {
        return *value;
    }
    let value = nodes.get(id).is_some_and(|node| {
        node.inputs.iter().any(|input| match input {
            FeatureInput::Market { .. } => true,
            FeatureInput::Node { node_id } => depends_on_market(node_id, nodes, memo),
            FeatureInput::Artifact { .. } => false,
        })
    });
    memo.insert(id.to_owned(), value);
    value
}

fn fatal_error(
    code: FeatureEvaluationErrorCode,
    stage: EvaluationStage,
    node_id: Option<String>,
    instrument_id: Option<String>,
    observation_time_ms: Option<i64>,
    diagnostic: impl Into<String>,
) -> FeatureEvaluationError {
    FeatureEvaluationError {
        code,
        stage,
        node_id,
        instrument_id,
        observation_time_ms,
        diagnostic: diagnostic.into(),
    }
}
