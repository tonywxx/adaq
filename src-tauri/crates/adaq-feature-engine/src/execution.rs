use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fmt,
};

use adaq_data_core::{
    BarInterval, OhlcvBar,
    market::{PriceBasis, TradingCalendarSnapshot, Venue, VenueKind},
};
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
    FeatureOutput, FeaturePlan, FeatureReference, FeatureScope, FeatureSlot, FeatureSource,
    FeatureUnavailabilityReason, FittedTransformationArtifact, FittedTransformationValue,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureMarketContext {
    pub venue: Venue,
    pub asset_class: VenueKind,
    pub bar_interval: BarInterval,
    pub price_basis: PriceBasis,
    pub valuation_currency: String,
}

impl FeatureMarketContext {
    pub fn new(
        venue: Venue,
        asset_class: VenueKind,
        bar_interval: BarInterval,
        price_basis: PriceBasis,
        valuation_currency: impl Into<String>,
    ) -> Result<Self, FeatureInputError> {
        let valuation_currency = valuation_currency.into();
        if venue.kind != asset_class {
            return Err(FeatureInputError::new(
                "market-context-asset-class-mismatch",
            ));
        }
        if valuation_currency.trim().is_empty() {
            return Err(FeatureInputError::new("market-context-currency-missing"));
        }
        Ok(Self {
            venue,
            asset_class,
            bar_interval,
            price_basis,
            valuation_currency,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UniverseEvidenceState {
    Observed,
    Reconstructed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PointInTimeInstrumentUniverse {
    pub universe_id: String,
    pub as_of_ms: i64,
    pub members: Vec<String>,
    pub market_context: FeatureMarketContext,
    pub evidence_state: UniverseEvidenceState,
}

impl PointInTimeInstrumentUniverse {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        universe_id: impl Into<String>,
        as_of_ms: i64,
        members: Vec<String>,
        market_context: FeatureMarketContext,
        evidence_state: UniverseEvidenceState,
    ) -> Result<Self, FeatureInputError> {
        let universe = Self {
            universe_id: universe_id.into(),
            as_of_ms,
            members,
            market_context,
            evidence_state,
        };
        universe.validate()?;
        Ok(universe)
    }

    fn context(&self) -> Result<FeatureMarketContext, FeatureInputError> {
        FeatureMarketContext::new(
            self.market_context.venue.clone(),
            self.market_context.asset_class,
            self.market_context.bar_interval,
            self.market_context.price_basis,
            self.market_context.valuation_currency.clone(),
        )
    }

    fn validate(&self) -> Result<(), FeatureInputError> {
        if self.universe_id.trim().is_empty() {
            return Err(FeatureInputError::new("universe-id-missing"));
        }
        if self.members.iter().any(|member| member.trim().is_empty()) {
            return Err(FeatureInputError::new("universe-member-id-missing"));
        }
        let members = self.members.iter().collect::<BTreeSet<_>>();
        if members.len() != self.members.len() {
            return Err(FeatureInputError::new("duplicate-universe-member"));
        }
        self.context().map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureCrossSectionalBatch {
    pub observation_time_ms: i64,
    pub universe: PointInTimeInstrumentUniverse,
    pub inputs: Vec<FeatureEvaluationInput>,
}

impl FeatureCrossSectionalBatch {
    pub fn new(
        observation_time_ms: i64,
        universe: PointInTimeInstrumentUniverse,
        inputs: Vec<FeatureEvaluationInput>,
    ) -> Self {
        Self {
            observation_time_ms,
            universe,
            inputs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossSectionalCoverage {
    pub universe_count: usize,
    pub available_count: usize,
    pub actual_coverage: f64,
    pub evidence_state: UniverseEvidenceState,
}

impl CrossSectionalCoverage {
    fn new(
        universe_count: usize,
        available_count: usize,
        evidence_state: UniverseEvidenceState,
    ) -> Self {
        Self {
            universe_count,
            available_count,
            actual_coverage: if universe_count == 0 {
                0.0
            } else {
                available_count as f64 / universe_count as f64
            },
            evidence_state,
        }
    }
}

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

    #[cfg(feature = "deferred-equity")]
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
pub enum FeatureDependencySource {
    External {
        dependency_alias: String,
        output: String,
    },
    Signal {
        dataset_id: String,
        signal_name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureDependencyInput {
    pub source: FeatureDependencySource,
    pub value: Option<f64>,
    pub available_at_ms: i64,
}

impl FeatureDependencyInput {
    pub fn external(
        dependency_alias: impl Into<String>,
        output: impl Into<String>,
        value: Option<f64>,
        available_at_ms: i64,
    ) -> Self {
        Self {
            source: FeatureDependencySource::External {
                dependency_alias: dependency_alias.into(),
                output: output.into(),
            },
            value,
            available_at_ms,
        }
    }

    pub fn signal(
        dataset_id: impl Into<String>,
        signal_name: impl Into<String>,
        value: Option<f64>,
        available_at_ms: i64,
    ) -> Self {
        Self {
            source: FeatureDependencySource::Signal {
                dataset_id: dataset_id.into(),
                signal_name: signal_name.into(),
            },
            value,
            available_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureEvaluationInput {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub available_at_ms: i64,
    pub bar: Option<FeatureMarketBar>,
    pub calendar: Option<TradingCalendarSnapshot>,
    pub corporate_actions: Vec<CorporateAction>,
    pub market_context: Option<FeatureMarketContext>,
    pub dependencies: Vec<FeatureDependencyInput>,
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
            market_context: None,
            dependencies: Vec::new(),
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
            market_context: None,
            dependencies: Vec::new(),
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

    pub fn with_market_context(mut self, market_context: FeatureMarketContext) -> Self {
        self.market_context = Some(market_context);
        self
    }

    pub fn with_dependency(mut self, dependency: FeatureDependencyInput) -> Self {
        self.dependencies.push(dependency);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureInputEvent {
    Observation(FeatureEvaluationInput),
    CrossSectionalBatch(FeatureCrossSectionalBatch),
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

    pub fn cross_sectional_batch(
        observation_time_ms: i64,
        universe: PointInTimeInstrumentUniverse,
        inputs: Vec<FeatureEvaluationInput>,
    ) -> Self {
        Self::CrossSectionalBatch(FeatureCrossSectionalBatch::new(
            observation_time_ms,
            universe,
            inputs,
        ))
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
        self.evaluator_with_artifacts(plan, &[])
    }

    pub fn evaluator_with_artifacts(
        &self,
        plan: FeaturePlan,
        artifacts: &[FittedTransformationArtifact],
    ) -> Result<FeatureEvaluator, FeatureEvaluationError> {
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
        FeatureEvaluator::new_with_artifacts(plan, artifacts)
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
    factor_warmup_bars: HashMap<String, u32>,
    instruments: HashMap<String, InstrumentRuntime>,
    indicator_engine: Option<IndicatorEngine>,
    artifacts: HashMap<String, FittedTransformationArtifact>,
    fitted_artifacts: HashMap<(String, String), FittedTransformationArtifact>,
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
        Self::new_with_artifacts(plan, &[])
    }

    pub fn new_with_artifacts(
        plan: FeaturePlan,
        artifacts: &[FittedTransformationArtifact],
    ) -> Result<Self, FeatureEvaluationError> {
        let definitions = plan
            .definitions()
            .iter()
            .map(RuntimeDefinition::new)
            .collect::<Result<Vec<_>, _>>()?;
        let factor_warmup_bars = plan
            .factors()
            .iter()
            .map(|factor| (factor.alias.clone(), factor.warmup_bars))
            .collect();
        let artifacts = artifacts
            .iter()
            .map(|artifact| (artifact.artifact_id().to_owned(), artifact.clone()))
            .collect::<HashMap<_, _>>();
        if artifacts
            .values()
            .any(|artifact| !artifact.integrity_valid())
        {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidIdentity,
                EvaluationStage::Validation,
                None,
                None,
                None,
                "invalid-fitted-artifact",
            ));
        }
        let expected_identity = plan.engine_identity();
        let mut fitted_artifacts = HashMap::new();
        for binding in plan.artifacts() {
            if let Some(artifact) = artifacts.get(&binding.artifact_id) {
                let fitted_node_owners = plan
                    .definitions()
                    .iter()
                    .filter(|definition| {
                        definition.definition_hash() == binding.fitted_output.definition_hash
                            && definition.outputs().iter().any(|output| {
                                output.node_id == binding.fitted_output.node_id
                                    && output.name == binding.fitted_output.output_name
                            })
                    })
                    .flat_map(|definition| {
                        definition
                            .nodes()
                            .iter()
                            .filter(|node| node.id == binding.fitted_output.node_id)
                            .map(move |node| (definition, node))
                    })
                    .collect::<Vec<_>>();
                let fitted_node_is_bound = fitted_node_owners.len() == 1 && {
                    let (definition, node) = fitted_node_owners[0];
                    definition.definition_hash() == binding.fitted_output.definition_hash
                        && node.id == binding.fitted_output.node_id
                        && node.id == artifact.fitted_node_id
                        && definition.outputs().iter().any(|output| {
                            output.node_id == binding.fitted_output.node_id
                                && output.name == binding.fitted_output.output_name
                        })
                };
                let fitted_input_is_bound = fitted_node_owners.first().is_some_and(|(_, node)| {
                    node.inputs.iter().any(|input| {
                        matches!(
                            input,
                            FeatureInput::Node {
                                node_id,
                                definition_hash: Some(definition_hash),
                            } if node_id == &artifact.input_feature.node_id
                                && definition_hash == &artifact.input_feature.definition_hash
                        )
                    })
                });
                let feature_definition_is_bound = plan.definitions().iter().any(|definition| {
                    definition.definition_hash() == artifact.input_feature.definition_hash
                        && definition.outputs().iter().any(|output| {
                            output.node_id == artifact.input_feature.node_id
                                && output.name == artifact.input_feature.output_name
                        })
                });
                if artifact.eligible_at_ms() != binding.eligible_at_ms
                    || artifact.engine_identity != expected_identity
                    || artifact.fitted_output != binding.fitted_output
                    || !feature_definition_is_bound
                    || !fitted_node_is_bound
                    || !fitted_input_is_bound
                {
                    return Err(fatal_error(
                        FeatureEvaluationErrorCode::InvalidIdentity,
                        EvaluationStage::Validation,
                        None,
                        None,
                        None,
                        "fitted-artifact-eligibility-mismatch",
                    ));
                }
                if fitted_artifacts
                    .insert(
                        (
                            binding.fitted_output.definition_hash.clone(),
                            binding.fitted_output.node_id.clone(),
                        ),
                        artifact.clone(),
                    )
                    .is_some()
                {
                    return Err(fatal_error(
                        FeatureEvaluationErrorCode::InvalidIdentity,
                        EvaluationStage::Validation,
                        None,
                        None,
                        None,
                        "duplicate-fitted-artifact-output",
                    ));
                }
            }
        }
        Ok(Self {
            plan,
            definitions,
            factor_warmup_bars,
            instruments: HashMap::new(),
            indicator_engine: None,
            artifacts,
            fitted_artifacts,
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
            FeatureInputEvent::CrossSectionalBatch(batch) => self.observe_cross_sectional(batch),
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
        let artifacts = &self.artifacts;
        let fitted_artifacts = &self.fitted_artifacts;
        let factor_warmup_bars = &self.factor_warmup_bars;
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

        let current = evaluate_definitions_for_input(
            definitions,
            runtime,
            &input,
            &plan.engine_identity(),
            indicator_engine,
            artifacts,
            fitted_artifacts,
            &instrument_id,
            false,
        )?;
        let slots = evaluate_slots_for_input(
            plan.slots(),
            runtime,
            &input,
            &plan.engine_identity(),
            indicator_engine,
            artifacts,
            fitted_artifacts,
            factor_warmup_bars,
            &instrument_id,
        )?;
        let mut observations = Vec::new();
        for (definition_index, template) in definitions.iter().enumerate() {
            for output in &template.outputs {
                let value = current[definition_index]
                    .get(&output.node_id)
                    .ok_or_else(|| {
                        fatal_error(
                            FeatureEvaluationErrorCode::BrokenShape,
                            EvaluationStage::Invariant,
                            Some(output.node_id.clone()),
                            Some(instrument_id.clone()),
                            Some(input.observation_time_ms),
                            "runtime-output-missing",
                        )
                    })?;
                let feature_reference = FeatureReference {
                    definition_hash: template.definition_hash.clone(),
                    node_id: output.node_id.clone(),
                    output_name: output.name.clone(),
                };
                observations.push(observation_from_value(
                    &output.name,
                    &input,
                    Some(&feature_reference),
                    value,
                )?);
            }
        }
        for slot in plan.slots() {
            let value = slots.get(&slot.name).ok_or_else(|| {
                fatal_error(
                    FeatureEvaluationErrorCode::BrokenShape,
                    EvaluationStage::Invariant,
                    None,
                    Some(instrument_id.clone()),
                    Some(input.observation_time_ms),
                    "runtime-slot-missing",
                )
            })?;
            observations.push(observation_from_value(&slot.name, &input, None, value)?);
        }
        Ok(observations)
    }

    fn observe_cross_sectional(
        &mut self,
        batch: FeatureCrossSectionalBatch,
    ) -> Result<Vec<FeatureObservation>, FeatureEvaluationError> {
        let prepared = self.prepare_cross_sectional_batch(batch)?;
        let PreparedCrossSectionalBatch {
            observation_time_ms,
            universe,
            members,
            mut inputs,
        } = prepared;
        let universe_count = members.len();
        let definitions = &self.definitions;
        let plan = &self.plan;
        let artifacts = &self.artifacts;
        let fitted_artifacts = &self.fitted_artifacts;
        let factor_warmup_bars = &self.factor_warmup_bars;
        let expected_identity = plan.engine_identity();
        let indicator_engine = &mut self.indicator_engine;

        if universe.evidence_state == UniverseEvidenceState::Unknown {
            for member in &members {
                let runtime = self
                    .instruments
                    .entry(member.clone())
                    .or_insert_with(|| InstrumentRuntime::new(definitions));
                runtime.note_event(member, observation_time_ms)?;
            }
            let coverage = CrossSectionalCoverage::new(universe_count, 0, universe.evidence_state);
            let mut observations = Vec::new();
            for member in &members {
                for (definition_index, template) in definitions.iter().enumerate() {
                    for output in &template.outputs {
                        let mut observation = FeatureObservation::unavailable(
                            &output.name,
                            member,
                            observation_time_ms,
                            FeatureUnavailabilityReason::UnknownUniverse,
                        )
                        .map_err(|error| {
                            fatal_error(
                                error.code,
                                EvaluationStage::Invariant,
                                Some(output.node_id.clone()),
                                Some(member.clone()),
                                Some(observation_time_ms),
                                "invalid-cross-sectional-observation",
                            )
                        })?;
                        observation.feature_reference = Some(FeatureReference {
                            definition_hash: definitions[definition_index].definition_hash.clone(),
                            node_id: output.node_id.clone(),
                            output_name: output.name.clone(),
                        });
                        if definitions[definition_index]
                            .nodes
                            .get(&output.node_id)
                            .is_some_and(|node| node.scope == FeatureScope::CrossSectional)
                        {
                            observation.cross_sectional_coverage = Some(coverage.clone());
                        }
                        observations.push(observation);
                    }
                }
                for slot in plan.slots() {
                    observations.push(
                        FeatureObservation::unavailable(
                            &slot.name,
                            member,
                            observation_time_ms,
                            FeatureUnavailabilityReason::UnknownUniverse,
                        )
                        .map_err(|error| {
                            fatal_error(
                                error.code,
                                EvaluationStage::Invariant,
                                None,
                                Some(member.clone()),
                                Some(observation_time_ms),
                                "invalid-cross-sectional-observation",
                            )
                        })?,
                    );
                }
            }
            return Ok(observations);
        }

        let mut row_states = vec![vec![HashMap::new(); definitions.len()]; inputs.len()];
        let mut row_coverage = vec![vec![HashMap::new(); definitions.len()]; inputs.len()];
        let mut slot_values = vec![HashMap::new(); inputs.len()];

        for (row_index, input) in inputs.iter_mut().enumerate() {
            let instrument_id = input.instrument_id.clone();
            let runtime = self
                .instruments
                .entry(instrument_id.clone())
                .or_insert_with(|| InstrumentRuntime::new(definitions));
            runtime.note_event(&instrument_id, observation_time_ms)?;
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
                    Some(observation_time_ms),
                    "corporate-action-instrument-mismatch",
                ));
            }
            runtime.observe_count = runtime.observe_count.saturating_add(1);
            runtime.remember_actions(&input.corporate_actions);
            input.corporate_actions = runtime.corporate_actions.clone();

            row_states[row_index] = evaluate_definitions_for_input(
                definitions,
                runtime,
                input,
                &expected_identity,
                indicator_engine,
                artifacts,
                fitted_artifacts,
                &instrument_id,
                true,
            )?;
            slot_values[row_index] = evaluate_slots_for_input(
                plan.slots(),
                runtime,
                input,
                &expected_identity,
                indicator_engine,
                artifacts,
                fitted_artifacts,
                factor_warmup_bars,
                &instrument_id,
            )?;
        }

        for (definition_index, template) in definitions.iter().enumerate() {
            for node_id in &template.order {
                let node = template.nodes.get(node_id).ok_or_else(|| {
                    fatal_error(
                        FeatureEvaluationErrorCode::BrokenShape,
                        EvaluationStage::Invariant,
                        Some(node_id.clone()),
                        None,
                        Some(observation_time_ms),
                        "runtime-node-missing",
                    )
                })?;
                if node.scope != FeatureScope::CrossSectional {
                    continue;
                }
                let values = inputs
                    .iter()
                    .enumerate()
                    .map(|(row_index, input)| {
                        node.inputs
                            .iter()
                            .map(|feature_input| {
                                resolve_input(
                                    feature_input,
                                    input,
                                    &row_states[row_index][definition_index],
                                    &row_states[row_index],
                                    definitions,
                                    definition_index,
                                    artifacts,
                                    &input.instrument_id,
                                    node_id,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let evaluated = evaluate_cross_sectional_node(
                    node,
                    &values,
                    &members,
                    universe.evidence_state,
                    observation_time_ms,
                )?;
                for (row_index, (value, coverage)) in evaluated.into_iter().enumerate() {
                    row_states[row_index][definition_index].insert(node.id.clone(), value);
                    row_coverage[row_index][definition_index].insert(node.id.clone(), coverage);
                }
            }
        }

        let mut observations = Vec::new();
        for row_index in 0..inputs.len() {
            let input = &inputs[row_index];
            for (definition_index, template) in definitions.iter().enumerate() {
                for output in &template.outputs {
                    let value = row_states[row_index][definition_index]
                        .get(&output.node_id)
                        .ok_or_else(|| {
                            fatal_error(
                                FeatureEvaluationErrorCode::BrokenShape,
                                EvaluationStage::Invariant,
                                Some(output.node_id.clone()),
                                Some(input.instrument_id.clone()),
                                Some(observation_time_ms),
                                "runtime-output-missing",
                            )
                        })?;
                    let feature_reference = FeatureReference {
                        definition_hash: template.definition_hash.clone(),
                        node_id: output.node_id.clone(),
                        output_name: output.name.clone(),
                    };
                    let mut observation = observation_from_value(
                        &output.name,
                        input,
                        Some(&feature_reference),
                        value,
                    )?;
                    observation.cross_sectional_coverage = row_coverage[row_index]
                        [definition_index]
                        .get(&output.node_id)
                        .cloned();
                    observations.push(observation);
                }
            }
            for slot in plan.slots() {
                let value = slot_values[row_index].get(&slot.name).ok_or_else(|| {
                    fatal_error(
                        FeatureEvaluationErrorCode::BrokenShape,
                        EvaluationStage::Invariant,
                        None,
                        Some(input.instrument_id.clone()),
                        Some(observation_time_ms),
                        "runtime-slot-missing",
                    )
                })?;
                observations.push(observation_from_value(&slot.name, input, None, value)?);
            }
        }
        Ok(observations)
    }

    fn prepare_cross_sectional_batch(
        &self,
        batch: FeatureCrossSectionalBatch,
    ) -> Result<PreparedCrossSectionalBatch, FeatureEvaluationError> {
        if !self
            .plan
            .definitions()
            .iter()
            .any(|definition| definition.scope() == FeatureScope::CrossSectional)
        {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(batch.observation_time_ms),
                "cross-sectional-batch-requires-cross-sectional-plan",
            ));
        }
        batch.universe.validate().map_err(|error| {
            fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(batch.observation_time_ms),
                error.code,
            )
        })?;
        if batch.universe.as_of_ms != batch.observation_time_ms {
            return Err(fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(batch.observation_time_ms),
                "cross-sectional-observation-time-mismatch",
            ));
        }
        let context = batch.universe.context().map_err(|error| {
            fatal_error(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                None,
                None,
                Some(batch.observation_time_ms),
                error.code,
            )
        })?;
        let mut members = batch.universe.members.clone();
        members.sort();
        let member_set = members.iter().collect::<BTreeSet<_>>();
        let mut inputs_by_id = BTreeMap::new();
        for mut input in batch.inputs {
            if input.instrument_id.is_empty() {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    None,
                    Some(batch.observation_time_ms),
                    "empty-instrument-id",
                ));
            }
            if input.observation_time_ms != batch.observation_time_ms {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    Some(input.instrument_id),
                    Some(input.observation_time_ms),
                    "cross-sectional-observation-time-mismatch",
                ));
            }
            if input.available_at_ms > batch.observation_time_ms {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Availability,
                    None,
                    Some(input.instrument_id),
                    Some(batch.observation_time_ms),
                    "cross-sectional-input-not-yet-available",
                ));
            }
            if input
                .bar
                .as_ref()
                .is_some_and(|bar| bar.open_time_ms != batch.observation_time_ms)
            {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    Some(input.instrument_id),
                    Some(batch.observation_time_ms),
                    "cross-sectional-bar-time-mismatch",
                ));
            }
            if !member_set.contains(&input.instrument_id) {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    Some(input.instrument_id),
                    Some(batch.observation_time_ms),
                    "cross-sectional-instrument-outside-universe",
                ));
            }
            if input.market_context.is_none() {
                input.market_context = Some(context.clone());
            }
            if input
                .market_context
                .as_ref()
                .is_some_and(|value| value != &context)
            {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    Some(input.instrument_id),
                    Some(batch.observation_time_ms),
                    "cross-sectional-market-context-mismatch",
                ));
            }
            if inputs_by_id
                .insert(input.instrument_id.clone(), input)
                .is_some()
            {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidObservation,
                    EvaluationStage::Validation,
                    None,
                    None,
                    Some(batch.observation_time_ms),
                    "duplicate-cross-sectional-input",
                ));
            }
        }
        let inputs = members
            .iter()
            .map(|member| {
                inputs_by_id.remove(member).unwrap_or_else(|| {
                    FeatureEvaluationInput::missing(
                        member,
                        batch.observation_time_ms,
                        batch.observation_time_ms,
                    )
                })
            })
            .collect();
        Ok(PreparedCrossSectionalBatch {
            observation_time_ms: batch.observation_time_ms,
            universe: batch.universe,
            members,
            inputs,
        })
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
        let mut observations = Vec::new();
        for definition in self.plan.definitions() {
            for output in definition.outputs() {
                let mut observation = FeatureObservation::unavailable(
                    &output.name,
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
                })?;
                observation.feature_reference = Some(FeatureReference {
                    definition_hash: definition.definition_hash().into(),
                    node_id: output.node_id.clone(),
                    output_name: output.name.clone(),
                });
                observations.push(observation);
            }
        }
        for slot in self.plan.slots() {
            observations.push(
                FeatureObservation::unavailable(
                    &slot.name,
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
                })?,
            );
        }
        Ok(observations)
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
}

fn evaluate_definitions_for_input(
    definitions: &[RuntimeDefinition],
    runtime: &mut InstrumentRuntime,
    input: &FeatureEvaluationInput,
    expected_identity: &FeatureEngineIdentity,
    indicator_engine: &mut Option<IndicatorEngine>,
    artifacts: &HashMap<String, FittedTransformationArtifact>,
    fitted_artifacts: &HashMap<(String, String), FittedTransformationArtifact>,
    instrument_id: &str,
    skip_cross_sectional: bool,
) -> Result<Vec<HashMap<String, EvalValue>>, FeatureEvaluationError> {
    let mut current = vec![HashMap::new(); definitions.len()];
    for (definition_index, template) in definitions.iter().enumerate() {
        let definition = &mut runtime.definitions[definition_index];
        definition.current.clear();
        for node_id in &template.order {
            let node = definition.nodes.get(node_id).cloned().ok_or_else(|| {
                fatal_error(
                    FeatureEvaluationErrorCode::BrokenShape,
                    EvaluationStage::Invariant,
                    Some(node_id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(input.observation_time_ms),
                    "runtime-node-missing",
                )
            })?;
            if skip_cross_sectional && node.scope == FeatureScope::CrossSectional {
                continue;
            }
            let values = node
                .inputs
                .iter()
                .map(|feature_input| {
                    resolve_input(
                        feature_input,
                        input,
                        &definition.current,
                        &current,
                        definitions,
                        definition_index,
                        artifacts,
                        instrument_id,
                        node_id,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let state = definition.states.get_mut(node_id).ok_or_else(|| {
                fatal_error(
                    FeatureEvaluationErrorCode::BrokenShape,
                    EvaluationStage::Invariant,
                    Some(node_id.clone()),
                    Some(instrument_id.to_owned()),
                    Some(input.observation_time_ms),
                    "runtime-state-missing",
                )
            })?;
            let value = evaluate_node(
                &node,
                &values,
                input,
                state,
                expected_identity,
                indicator_engine,
                fitted_artifacts,
                &template.definition_hash,
                instrument_id,
            )?;
            definition.current.insert(node.id.clone(), value);
        }
        current[definition_index] = definition.current.clone();
    }
    Ok(current)
}

fn evaluate_slots_for_input(
    slots: &[FeatureSlot],
    runtime: &mut InstrumentRuntime,
    input: &FeatureEvaluationInput,
    expected_identity: &FeatureEngineIdentity,
    indicator_engine: &mut Option<IndicatorEngine>,
    artifacts: &HashMap<String, FittedTransformationArtifact>,
    fitted_artifacts: &HashMap<(String, String), FittedTransformationArtifact>,
    factor_warmup_bars: &HashMap<String, u32>,
    instrument_id: &str,
) -> Result<HashMap<String, EvalValue>, FeatureEvaluationError> {
    let mut values_by_name = HashMap::new();
    for slot in slots {
        let value = match &slot.source {
            FeatureSource::Market { field } => input
                .bar
                .as_ref()
                .and_then(|bar| bar.field(*field))
                .map(|value| {
                    value.analytical().map_or_else(
                        |_| EvalValue::Unavailable(FeatureUnavailabilityReason::MissingMarketInput),
                        |value| EvalValue::available(value, input.available_at_ms),
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
                            input,
                            &HashMap::new(),
                            &[],
                            &[],
                            0,
                            artifacts,
                            instrument_id,
                            &slot.name,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let state = runtime.slot_states.entry(slot.name.clone()).or_default();
                evaluate_node(
                    &node,
                    &values,
                    input,
                    state,
                    expected_identity,
                    indicator_engine,
                    fitted_artifacts,
                    "",
                    instrument_id,
                )?
            }
            FeatureSource::External {
                dependency_alias,
                output,
            } => {
                let warmup_bars = factor_warmup_bars
                    .get(dependency_alias)
                    .copied()
                    .unwrap_or_default()
                    .max(slot.warmup_bars);
                resolve_dependency(
                    input,
                    &FeatureDependencySource::External {
                        dependency_alias: dependency_alias.clone(),
                        output: output.clone(),
                    },
                    instrument_id,
                    &slot.name,
                    Some(warmup_bars),
                    runtime.observe_count,
                )?
            }
            FeatureSource::Signal {
                dataset_id,
                signal_name,
                ..
            } => resolve_dependency(
                input,
                &FeatureDependencySource::Signal {
                    dataset_id: dataset_id.clone(),
                    signal_name: signal_name.clone(),
                },
                instrument_id,
                &slot.name,
                Some(slot.warmup_bars),
                runtime.observe_count,
            )?,
        };
        values_by_name.insert(slot.name.clone(), value);
    }
    Ok(values_by_name)
}

fn resolve_dependency(
    input: &FeatureEvaluationInput,
    source: &FeatureDependencySource,
    instrument_id: &str,
    node_id: &str,
    warmup_bars: Option<u32>,
    observe_count: usize,
) -> Result<EvalValue, FeatureEvaluationError> {
    if warmup_bars.is_some_and(|warmup| observe_count <= warmup as usize) {
        return Ok(EvalValue::Unavailable(FeatureUnavailabilityReason::Warmup));
    }
    let dependency =
        input
            .dependencies
            .iter()
            .find(|dependency| match (&dependency.source, source) {
                (
                    FeatureDependencySource::External {
                        dependency_alias: left_alias,
                        output: left_output,
                    },
                    FeatureDependencySource::External {
                        dependency_alias: right_alias,
                        output: right_output,
                    },
                ) => left_alias == right_alias && left_output == right_output,
                (
                    FeatureDependencySource::Signal {
                        dataset_id: left_dataset,
                        signal_name: left_signal,
                    },
                    FeatureDependencySource::Signal {
                        dataset_id: right_dataset,
                        signal_name: right_signal,
                    },
                ) => left_dataset == right_dataset && left_signal == right_signal,
                _ => false,
            });
    let Some(dependency) = dependency else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    let Some(value) = dependency.value else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::MissingDependency,
        ));
    };
    if !value.is_finite() {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::NonFiniteOutput,
            EvaluationStage::Input,
            Some(node_id.to_owned()),
            Some(instrument_id.to_owned()),
            Some(input.observation_time_ms),
            "non-finite-feature-dependency",
        ));
    }
    Ok(EvalValue::available(value, dependency.available_at_ms))
}

struct PreparedCrossSectionalBatch {
    observation_time_ms: i64,
    universe: PointInTimeInstrumentUniverse,
    members: Vec<String>,
    inputs: Vec<FeatureEvaluationInput>,
}

struct RuntimeDefinition {
    definition_hash: String,
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
            definition_hash: definition.definition_hash().into(),
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
                    definition_hash: template.definition_hash.clone(),
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
    Available {
        value: f64,
        available_at_ms: i64,
    },
    Artifact {
        artifact_id: String,
        eligible_at_ms: i64,
    },
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
            Self::Artifact { .. } => None,
            Self::Unavailable(_) => None,
        }
    }

    fn available_at_ms(&self) -> Option<i64> {
        match self {
            Self::Available {
                available_at_ms, ..
            } => Some(*available_at_ms),
            Self::Artifact { eligible_at_ms, .. } => Some(*eligible_at_ms),
            Self::Unavailable(_) => None,
        }
    }
}

fn resolve_input(
    input: &FeatureInput,
    observation: &FeatureEvaluationInput,
    local_current: &HashMap<String, EvalValue>,
    all_current: &[HashMap<String, EvalValue>],
    definitions: &[RuntimeDefinition],
    definition_index: usize,
    artifacts: &HashMap<String, FittedTransformationArtifact>,
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
            definition_hash,
        } => {
            let source_current = match definition_hash.as_deref() {
                None => Some(local_current),
                Some(definition_hash) => definitions
                    .iter()
                    .position(|definition| definition.definition_hash == definition_hash)
                    .and_then(|source_index| {
                        if source_index == definition_index {
                            Some(local_current)
                        } else {
                            all_current.get(source_index)
                        }
                    }),
            };
            Ok(source_current
                .and_then(|current| current.get(dependency))
                .cloned()
                .unwrap_or(EvalValue::Unavailable(
                    FeatureUnavailabilityReason::MissingDependency,
                )))
        }
        FeatureInput::Artifact { artifact_id } => Ok(artifacts
            .get(artifact_id)
            .map(|artifact| EvalValue::Artifact {
                artifact_id: artifact_id.clone(),
                eligible_at_ms: artifact.eligible_at_ms(),
            })
            .unwrap_or(EvalValue::Unavailable(
                FeatureUnavailabilityReason::ArtifactMissingInstrument,
            ))),
    }
}

fn evaluate_cross_sectional_node(
    node: &FeatureNode,
    row_inputs: &[Vec<EvalValue>],
    instrument_ids: &[String],
    evidence_state: UniverseEvidenceState,
    observation_time_ms: i64,
) -> Result<Vec<(EvalValue, CrossSectionalCoverage)>, FeatureEvaluationError> {
    if node.inputs.len() != 1 || row_inputs.iter().any(|inputs| inputs.len() != 1) {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            None,
            Some(observation_time_ms),
            "cross-sectional-input-count-mismatch",
        ));
    }
    let minimum_count = node
        .parameters
        .get("minimumCount")
        .or_else(|| node.parameters.get("minimum-count"))
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let minimum_count = usize::try_from(minimum_count).map_err(|_| {
        fatal_error(
            FeatureEvaluationErrorCode::OperatorFailure,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            None,
            Some(observation_time_ms),
            "invalid-cross-sectional-minimum-count",
        )
    })?;
    let minimum_coverage = node
        .parameters
        .get("minimumCoverage")
        .or_else(|| node.parameters.get("minimum-coverage"))
        .and_then(value_as_f64)
        .unwrap_or(1.0);
    if minimum_count == 0
        || !minimum_coverage.is_finite()
        || minimum_coverage <= 0.0
        || minimum_coverage > 1.0
    {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::OperatorFailure,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            None,
            Some(observation_time_ms),
            "invalid-cross-sectional-coverage-policy",
        ));
    }
    let reverse = node
        .parameters
        .get("reverse")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let coverage = CrossSectionalCoverage::new(
        row_inputs.len(),
        row_inputs
            .iter()
            .filter(|inputs| inputs[0].value().is_some())
            .count(),
        evidence_state,
    );
    let available_count = coverage.available_count;
    let enough_coverage =
        coverage.available_count >= minimum_count && coverage.actual_coverage >= minimum_coverage;
    let available = row_inputs
        .iter()
        .enumerate()
        .filter_map(|(index, inputs)| inputs[0].value().map(|value| (index, value)))
        .collect::<Vec<_>>();
    let available_at_ms = row_inputs
        .iter()
        .filter_map(|inputs| inputs[0].available_at_ms())
        .max()
        .unwrap_or(observation_time_ms);
    let mut results = row_inputs
        .iter()
        .map(|inputs| (inputs[0].clone(), coverage.clone()))
        .collect::<Vec<_>>();
    if !enough_coverage {
        for (index, value) in &available {
            results[*index].0 = if value.is_finite() {
                EvalValue::Unavailable(FeatureUnavailabilityReason::InsufficientCoverage)
            } else {
                EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
            };
        }
        return Ok(results);
    }

    let mut ranked = available
        .iter()
        .map(|(index, value)| (*index, *value))
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        let order = if reverse {
            right.total_cmp(left)
        } else {
            left.total_cmp(right)
        };
        order.then_with(|| instrument_ids[*left_index].cmp(&instrument_ids[*right_index]))
    });
    let mut ranks = vec![0.0; row_inputs.len()];
    let mut start = 0;
    while start < ranked.len() {
        let mut end = start + 1;
        while end < ranked.len() && ranked[start].1 == ranked[end].1 {
            end += 1;
        }
        let rank = (start + 1 + end) as f64 / 2.0;
        for (index, _) in &ranked[start..end] {
            ranks[*index] = rank;
        }
        start = end;
    }
    let values = available
        .iter()
        .map(|(_, value)| *value)
        .collect::<Vec<_>>();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    for (index, value) in available {
        let output = match &node.operator {
            FeatureOperator::CrossSectionalRank => {
                EvalValue::available(ranks[index], available_at_ms)
            }
            FeatureOperator::CrossSectionalPercentile => {
                if available_count <= 1 {
                    EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
                } else {
                    EvalValue::available(
                        (ranks[index] - 1.0) / (available_count - 1) as f64,
                        available_at_ms,
                    )
                }
            }
            FeatureOperator::CrossSectionalZScore => {
                if variance == 0.0 || !variance.is_finite() {
                    EvalValue::Unavailable(FeatureUnavailabilityReason::UndefinedArithmetic)
                } else {
                    EvalValue::available((value - mean) / variance.sqrt(), available_at_ms)
                }
            }
            _ => {
                return Err(fatal_error(
                    FeatureEvaluationErrorCode::InvalidInvariant,
                    EvaluationStage::Validation,
                    Some(node.id.clone()),
                    None,
                    Some(observation_time_ms),
                    "invalid-cross-sectional-operator",
                ));
            }
        };
        results[index].0 = output;
    }
    Ok(results)
}

fn evaluate_node(
    node: &FeatureNode,
    inputs: &[EvalValue],
    observation: &FeatureEvaluationInput,
    state: &mut RuntimeNodeState,
    expected_identity: &FeatureEngineIdentity,
    indicator_engine: &mut Option<IndicatorEngine>,
    fitted_artifacts: &HashMap<(String, String), FittedTransformationArtifact>,
    definition_hash: &str,
    instrument_id: &str,
) -> Result<EvalValue, FeatureEvaluationError> {
    if matches!(
        node.operator,
        FeatureOperator::Standardization | FeatureOperator::Winsorization
    ) {
        return evaluate_fitted_transformation(
            node,
            inputs,
            observation,
            fitted_artifacts,
            definition_hash,
            instrument_id,
        );
    }
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
        FeatureOperator::Standardization | FeatureOperator::Winsorization => unreachable!(),
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

fn evaluate_fitted_transformation(
    node: &FeatureNode,
    inputs: &[EvalValue],
    observation: &FeatureEvaluationInput,
    fitted_artifacts: &HashMap<(String, String), FittedTransformationArtifact>,
    definition_hash: &str,
    instrument_id: &str,
) -> Result<EvalValue, FeatureEvaluationError> {
    let input = node
        .inputs
        .iter()
        .zip(inputs)
        .find(|(input, _)| !matches!(input, FeatureInput::Artifact { .. }))
        .map(|(_, value)| value);
    let (value, available_at_ms) = match input {
        Some(EvalValue::Available {
            value,
            available_at_ms,
        }) => (*value, *available_at_ms),
        Some(EvalValue::Unavailable(reason)) => return Ok(EvalValue::Unavailable(*reason)),
        Some(EvalValue::Artifact { .. }) | None => {
            return Ok(EvalValue::Unavailable(
                FeatureUnavailabilityReason::MissingDependency,
            ));
        }
    };
    let Some(artifact) = fitted_artifacts.get(&(definition_hash.to_owned(), node.id.clone()))
    else {
        return Ok(EvalValue::Unavailable(
            FeatureUnavailabilityReason::ArtifactMissingInstrument,
        ));
    };
    if node.id != artifact.fitted_node_id {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::InvalidIdentity,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "fitted-artifact-node-mismatch",
        ));
    }
    let algorithm_matches = match node.operator {
        FeatureOperator::Standardization => {
            matches!(artifact.algorithm, crate::FittingAlgorithm::Standardization)
        }
        FeatureOperator::Winsorization => {
            matches!(
                artifact.algorithm,
                crate::FittingAlgorithm::Winsorization { .. }
            )
        }
        _ => false,
    };
    if !algorithm_matches {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::InvalidIdentity,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "fitted-artifact-algorithm-mismatch",
        ));
    }
    let fitted_input = node.inputs.iter().find_map(|input| match input {
        FeatureInput::Node {
            node_id,
            definition_hash,
        } => Some((node_id.as_str(), definition_hash.as_deref())),
        FeatureInput::Market { .. } | FeatureInput::Artifact { .. } => None,
    });
    if !matches!(
        fitted_input,
        Some((node_id, Some(definition_hash)))
            if node_id == artifact.input_feature.node_id
                && definition_hash == artifact.input_feature.definition_hash
    ) {
        return Err(fatal_error(
            FeatureEvaluationErrorCode::InvalidIdentity,
            EvaluationStage::Validation,
            Some(node.id.clone()),
            Some(instrument_id.to_owned()),
            Some(observation.observation_time_ms),
            "fitted-artifact-input-feature-mismatch",
        ));
    }
    match artifact
        .apply_value(
            instrument_id,
            observation.observation_time_ms,
            value,
            available_at_ms,
        )
        .map_err(|error| {
            fatal_error(
                FeatureEvaluationErrorCode::OperatorFailure,
                EvaluationStage::Availability,
                Some(node.id.clone()),
                Some(instrument_id.to_owned()),
                Some(observation.observation_time_ms),
                error.code(),
            )
        })? {
        FittedTransformationValue::Available {
            value,
            available_at_ms,
        } => Ok(EvalValue::available(value, available_at_ms)),
        FittedTransformationValue::Unavailable(reason) => Ok(EvalValue::Unavailable(reason)),
    }
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
            EvalValue::Artifact { .. } => None,
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
    // ponytail: TA-Lib exposes only batch evaluation, so retain each continuous segment to keep
    // EMA-like indicators identical to full-segment evaluation; replace with a native streaming
    // API if profiling makes the O(segment length) per-bar materialization a bottleneck.
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
    feature_reference: Option<&FeatureReference>,
    value: &EvalValue,
) -> Result<FeatureObservation, FeatureEvaluationError> {
    let mut observation = match value {
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
        EvalValue::Artifact { .. } => Err(fatal_error(
            FeatureEvaluationErrorCode::BrokenShape,
            EvaluationStage::Invariant,
            None,
            Some(input.instrument_id.clone()),
            Some(input.observation_time_ms),
            "artifact-value-exposed-as-feature-output",
        )),
        EvalValue::Unavailable(reason) => FeatureObservation::unavailable(
            output_name,
            &input.instrument_id,
            input.observation_time_ms,
            *reason,
        ),
    }?;
    observation.feature_reference = feature_reference.cloned();
    Ok(observation)
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
        if let FeatureInput::Node { node_id, .. } = input {
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
            FeatureInput::Node { node_id, .. } => depends_on_market(node_id, nodes, memo),
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
