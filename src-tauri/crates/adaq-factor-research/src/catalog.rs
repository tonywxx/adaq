use serde::{Deserialize, Serialize};

use crate::{ContractError, FACTOR_RESEARCH_SCHEMA_VERSION, canonical_json, content_hash};

pub const FACTOR_METRIC_CATALOG_VERSION: &str = "adaq-factor-metric-catalog@1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricId {
    Coverage,
    Missingness,
    Ic,
    RankIc,
    Turnover,
    Decay,
    Stability,
    Economic,
    Regime,
    Neutralized,
    SampleCount,
    RawStatistic,
    PValue,
    HolmAdjusted,
}

impl MetricId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coverage => "coverage",
            Self::Missingness => "missingness",
            Self::Ic => "ic",
            Self::RankIc => "rank-ic",
            Self::Turnover => "turnover",
            Self::Decay => "decay",
            Self::Stability => "stability",
            Self::Economic => "economic",
            Self::Regime => "regime",
            Self::Neutralized => "neutralized",
            Self::SampleCount => "sample-count",
            Self::RawStatistic => "raw-statistic",
            Self::PValue => "p-value",
            Self::HolmAdjusted => "holm-adjusted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricDirection {
    HigherBetter,
    LowerBetter,
    Descriptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricRange {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MetricFormula {
    Coverage {
        numerator: String,
        denominator: String,
    },
    Missingness {
        numerator: String,
        denominator: String,
    },
    Correlation {
        method: CorrelationMethod,
        factor: String,
        target: String,
    },
    Turnover {
        estimator: String,
        weighting: String,
    },
    Decay {
        estimator: String,
        horizons: Vec<u32>,
    },
    Stability {
        estimator: String,
        aggregation: String,
    },
    Economic {
        quantiles: u8,
        weighting: String,
        portfolios: Vec<String>,
        next_eligible_bar: bool,
    },
    Regime {
        assignment: String,
        threshold_fit_window: String,
    },
    Neutralized {
        estimator: String,
        intercept: bool,
    },
    SampleCount {
        source: String,
    },
    RawStatistic {
        statistic: String,
    },
    PValue {
        method: String,
    },
    HolmAdjusted {
        correction: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CorrelationMethod {
    Pearson,
    SpearmanAverageRank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricUndefinedReason {
    InsufficientSamples,
    ConstantValues,
    SingularMatrix,
    UnavailableTarget,
    MissingInput,
    NoEligibleObservations,
    InvalidRequirement,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricDefinition {
    pub id: MetricId,
    pub formula: MetricFormula,
    pub direction: MetricDirection,
    pub range: Option<MetricRange>,
    pub minimum_samples: u64,
    pub undefined_reasons: Vec<MetricUndefinedReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum MetricObservation {
    Available {
        value: f64,
        sample_count: u64,
    },
    Unavailable {
        reason: MetricUndefinedReason,
        sample_count: u64,
    },
}

impl MetricObservation {
    pub fn available(value: f64, sample_count: u64) -> Result<Self, ContractError> {
        if !value.is_finite() {
            return Err(ContractError::Invalid(
                "metric values must be finite".into(),
            ));
        }
        Ok(Self::Available {
            value,
            sample_count,
        })
    }

    pub const fn unavailable(reason: MetricUndefinedReason, sample_count: u64) -> Self {
        Self::Unavailable {
            reason,
            sample_count,
        }
    }

    pub const fn value(&self) -> Option<f64> {
        match self {
            Self::Available { value, .. } => Some(*value),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if matches!(self, Self::Available { value, .. } if !value.is_finite()) {
            return Err(ContractError::Invalid(
                "metric values must be finite".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMetricCatalog {
    pub schema_version: String,
    pub catalog_version: String,
    pub definitions: Vec<MetricDefinition>,
    pub catalog_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogContent<'a> {
    schema_version: &'a str,
    catalog_version: &'a str,
    definitions: &'a [MetricDefinition],
}

impl FactorMetricCatalog {
    pub fn initial() -> Self {
        let mut catalog = Self {
            schema_version: FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            catalog_version: FACTOR_METRIC_CATALOG_VERSION.into(),
            definitions: definitions(),
            catalog_hash: String::new(),
        };
        catalog.catalog_hash =
            content_hash(&catalog.content()).expect("metric catalog is canonical");
        catalog
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.catalog_version != FACTOR_METRIC_CATALOG_VERSION
            || self.definitions != definitions()
            || !crate::is_sha256(&self.catalog_hash)
            || self.catalog_hash != content_hash(&self.content())?
        {
            return Err(ContractError::Invalid(
                "Factor Metric Catalog identity or definition set is invalid".into(),
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        if self
            .definitions
            .iter()
            .any(|definition| !ids.insert(definition.id))
        {
            return Err(ContractError::Invalid(
                "Factor Metric Catalog metric identities must be unique".into(),
            ));
        }
        Ok(())
    }

    pub fn metrics(&self) -> &[MetricDefinition] {
        &self.definitions
    }

    pub fn metric(&self, id: MetricId) -> Option<&MetricDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.id == id)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ContractError> {
        self.validate()?;
        canonical_json(self)
    }

    fn content(&self) -> CatalogContent<'_> {
        CatalogContent {
            schema_version: &self.schema_version,
            catalog_version: &self.catalog_version,
            definitions: &self.definitions,
        }
    }
}

fn definitions() -> Vec<MetricDefinition> {
    use MetricDirection::{Descriptive, HigherBetter, LowerBetter};
    use MetricUndefinedReason::{
        ConstantValues, InsufficientSamples, MissingInput, NoEligibleObservations, NotApplicable,
        SingularMatrix, UnavailableTarget,
    };
    vec![
        MetricDefinition {
            id: MetricId::Coverage,
            formula: MetricFormula::Coverage {
                numerator: "available-observations".into(),
                denominator: "eligible-observations".into(),
            },
            direction: HigherBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 1,
            undefined_reasons: vec![NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Missingness,
            formula: MetricFormula::Missingness {
                numerator: "unavailable-observations".into(),
                denominator: "eligible-observations".into(),
            },
            direction: LowerBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 1,
            undefined_reasons: vec![NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Ic,
            formula: MetricFormula::Correlation {
                method: CorrelationMethod::Pearson,
                factor: "factor-output".into(),
                target: "future-close-return".into(),
            },
            direction: HigherBetter,
            range: Some(MetricRange {
                minimum: Some(-1.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 3,
            undefined_reasons: vec![InsufficientSamples, ConstantValues, UnavailableTarget],
        },
        MetricDefinition {
            id: MetricId::RankIc,
            formula: MetricFormula::Correlation {
                method: CorrelationMethod::SpearmanAverageRank,
                factor: "average-rank-factor-output".into(),
                target: "average-rank-future-close-return".into(),
            },
            direction: HigherBetter,
            range: Some(MetricRange {
                minimum: Some(-1.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 3,
            undefined_reasons: vec![InsufficientSamples, ConstantValues, UnavailableTarget],
        },
        MetricDefinition {
            id: MetricId::Turnover,
            formula: MetricFormula::Turnover {
                estimator: "mean-absolute-weight-change".into(),
                weighting: "equal-weight".into(),
            },
            direction: LowerBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: None,
            }),
            minimum_samples: 2,
            undefined_reasons: vec![InsufficientSamples, NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Decay,
            formula: MetricFormula::Decay {
                estimator: "lagged-metric-path".into(),
                horizons: vec![1, 2, 3, 5, 10],
            },
            direction: Descriptive,
            range: None,
            minimum_samples: 3,
            undefined_reasons: vec![InsufficientSamples, NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Stability,
            formula: MetricFormula::Stability {
                estimator: "subperiod-sign-consistency".into(),
                aggregation: "signed-count-over-subperiods".into(),
            },
            direction: HigherBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 2,
            undefined_reasons: vec![InsufficientSamples, NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Economic,
            formula: MetricFormula::Economic {
                quantiles: 5,
                weighting: "equal-weight".into(),
                portfolios: vec!["top".into(), "top-minus-bottom".into()],
                next_eligible_bar: true,
            },
            direction: HigherBetter,
            range: None,
            minimum_samples: 5,
            undefined_reasons: vec![InsufficientSamples, MissingInput, NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Regime,
            formula: MetricFormula::Regime {
                assignment: "causal-feature-buckets".into(),
                threshold_fit_window: "selection-window-only".into(),
            },
            direction: Descriptive,
            range: None,
            minimum_samples: 2,
            undefined_reasons: vec![InsufficientSamples, MissingInput, NoEligibleObservations],
        },
        MetricDefinition {
            id: MetricId::Neutralized,
            formula: MetricFormula::Neutralized {
                estimator: "cross-sectional-ols".into(),
                intercept: true,
            },
            direction: HigherBetter,
            range: Some(MetricRange {
                minimum: Some(-1.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 3,
            undefined_reasons: vec![InsufficientSamples, SingularMatrix, MissingInput],
        },
        MetricDefinition {
            id: MetricId::SampleCount,
            formula: MetricFormula::SampleCount {
                source: "complete-metric-observations".into(),
            },
            direction: Descriptive,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: None,
            }),
            minimum_samples: 0,
            undefined_reasons: vec![],
        },
        MetricDefinition {
            id: MetricId::RawStatistic,
            formula: MetricFormula::RawStatistic {
                statistic: "registered-trial-statistic".into(),
            },
            direction: Descriptive,
            range: None,
            minimum_samples: 1,
            undefined_reasons: vec![InsufficientSamples, NotApplicable],
        },
        MetricDefinition {
            id: MetricId::PValue,
            formula: MetricFormula::PValue {
                method: "declared-statistical-test".into(),
            },
            direction: LowerBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 1,
            undefined_reasons: vec![InsufficientSamples, NotApplicable],
        },
        MetricDefinition {
            id: MetricId::HolmAdjusted,
            formula: MetricFormula::HolmAdjusted {
                correction: "holm-bonferroni-family-wise-error".into(),
            },
            direction: LowerBetter,
            range: Some(MetricRange {
                minimum: Some(0.0),
                maximum: Some(1.0),
            }),
            minimum_samples: 1,
            undefined_reasons: vec![InsufficientSamples, NotApplicable],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_catalog_is_machine_readable_and_complete() {
        let catalog = FactorMetricCatalog::initial();
        catalog.validate().unwrap();
        assert_eq!(catalog.metrics().len(), 14);
        assert!(catalog.metric(MetricId::HolmAdjusted).is_some());
        assert!(catalog.to_json().unwrap().len() < crate::MAX_CANONICAL_JSON_BYTES);
    }

    #[test]
    fn undefined_metric_is_not_numeric_zero() {
        let value = MetricObservation::unavailable(MetricUndefinedReason::SingularMatrix, 2);
        assert_eq!(value.value(), None);
        assert!(MetricObservation::available(f64::NAN, 2).is_err());
    }
}
