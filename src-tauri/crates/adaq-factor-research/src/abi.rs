use std::collections::HashSet;

use crate::{
    AvailableFactorValue, ContractError, CrossSectionalInputRow, FactorResult, FactorSlotCell,
    MAX_FACTOR_OUTPUTS, NamedFactorOutput, TimeSeriesInputRow, is_lower_kebab,
};

pub fn validate_time_series_batch(
    rows: &[TimeSeriesInputRow],
    expected_instrument_id: &str,
    expected_slot_count: usize,
) -> Result<(), ContractError> {
    if expected_slot_count > crate::MAX_FACTOR_SLOTS {
        return Err(ContractError::LimitExceeded {
            name: "Factor Feature Slots",
            limit: crate::MAX_FACTOR_SLOTS,
        });
    }
    let mut previous_time = None;
    for row in rows {
        if row.instrument_id != expected_instrument_id
            || row.instrument_id.is_empty()
            || row.slots.len() != expected_slot_count
            || previous_time.is_some_and(|time| row.observation_time_ms <= time)
        {
            return Err(ContractError::Invalid(
                "Time-Series Factor rows must be one instrument, dense, and causally ordered"
                    .into(),
            ));
        }
        for value in &row.slots {
            validate_available_value(value, row.observation_time_ms)?;
        }
        previous_time = Some(row.observation_time_ms);
    }
    Ok(())
}

pub fn validate_cross_sectional_batch(
    rows: &[CrossSectionalInputRow],
    expected_instrument_ids: &[String],
    expected_slot_count: usize,
) -> Result<(), ContractError> {
    if expected_instrument_ids.is_empty() || expected_slot_count > crate::MAX_FACTOR_SLOTS {
        return Err(ContractError::Invalid(
            "Cross-Sectional Factor batches require a complete bounded Universe".into(),
        ));
    }
    if rows.len() != expected_instrument_ids.len() {
        return Err(ContractError::Invalid(
            "Cross-Sectional Factor batch membership count does not match the Point-in-Time Universe".into(),
        ));
    }
    let mut members = HashSet::new();
    if expected_instrument_ids
        .iter()
        .any(|instrument_id| instrument_id.is_empty() || !members.insert(instrument_id))
    {
        return Err(ContractError::Invalid(
            "Cross-Sectional Factor Universe membership must be unique and non-empty".into(),
        ));
    }
    let observation_time = rows.first().map(|row| row.observation_time_ms);
    for ((row, expected_id), expected_time) in rows
        .iter()
        .zip(expected_instrument_ids)
        .zip(observation_time)
    {
        if row.instrument_id != *expected_id
            || row.instrument_id.is_empty()
            || row.observation_time_ms != expected_time
            || row.slots.len() != expected_slot_count
        {
            return Err(ContractError::Invalid(
                "Cross-Sectional Factor rows must preserve deterministic Universe membership and order".into(),
            ));
        }
        for cell in &row.slots {
            if let FactorSlotCell::Available(value) = cell {
                validate_available_value(value, row.observation_time_ms)?;
            }
        }
    }
    Ok(())
}

pub fn validate_factor_results(
    results: &[FactorResult],
    instrument_ids: &[String],
    observation_times: &[i64],
    expected_output_names: &[String],
) -> Result<(), ContractError> {
    if expected_output_names.is_empty() || expected_output_names.len() > MAX_FACTOR_OUTPUTS {
        return Err(ContractError::Invalid(
            "Factor output declaration must contain 1..=64 outputs".into(),
        ));
    }
    if results.len() != instrument_ids.len() || results.len() != observation_times.len() {
        return Err(ContractError::Invalid(
            "Factor output row count does not match input row count".into(),
        ));
    }
    for (((result, instrument_id), observation_time),) in results
        .iter()
        .zip(instrument_ids)
        .zip(observation_times)
        .map(|value| (value,))
    {
        if result.instrument_id != *instrument_id || result.observation_time_ms != *observation_time
        {
            return Err(ContractError::Invalid(
                "Factor output identity or order does not match input identity".into(),
            ));
        }
        if let Some(values) = &result.values {
            validate_named_outputs(values, expected_output_names)?;
        }
    }
    Ok(())
}

pub fn factor_results_bit_identical(left: &[FactorResult], right: &[FactorResult]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.instrument_id == right.instrument_id
                && left.observation_time_ms == right.observation_time_ms
                && match (&left.values, &right.values) {
                    (None, None) => true,
                    (Some(left), Some(right)) => {
                        left.len() == right.len()
                            && left.iter().zip(right).all(|(left, right)| {
                                left.name == right.name
                                    && left.value.to_bits() == right.value.to_bits()
                            })
                    }
                    _ => false,
                }
        })
}

fn validate_available_value(
    value: &AvailableFactorValue,
    observation_time_ms: i64,
) -> Result<(), ContractError> {
    if !value.value.is_finite() || value.available_at_ms > observation_time_ms {
        return Err(ContractError::Invalid(
            "Factor input availability must be causal and finite".into(),
        ));
    }
    Ok(())
}

fn validate_named_outputs(
    values: &[NamedFactorOutput],
    expected_output_names: &[String],
) -> Result<(), ContractError> {
    if values.len() != expected_output_names.len()
        || values
            .iter()
            .zip(expected_output_names)
            .any(|(value, expected)| {
                value.name != *expected || !is_lower_kebab(&value.name) || !value.value.is_finite()
            })
    {
        return Err(ContractError::Invalid(
            "Factor output names, order, count, or finite values do not match the ABI".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(number: f64) -> AvailableFactorValue {
        AvailableFactorValue {
            value: number,
            available_at_ms: 0,
        }
    }

    #[test]
    fn time_series_validation_rejects_non_causal_and_non_finite_inputs() {
        let mut rows = vec![TimeSeriesInputRow {
            instrument_id: "instrument".into(),
            observation_time_ms: 1,
            slots: vec![value(1.0)],
        }];
        rows[0].slots[0].available_at_ms = 2;
        assert!(validate_time_series_batch(&rows, "instrument", 1).is_err());
        rows[0].slots[0] = value(f64::NAN);
        assert!(validate_time_series_batch(&rows, "instrument", 1).is_err());
    }

    #[test]
    fn cross_sectional_validation_preserves_unavailable_members_and_order() {
        let rows = vec![
            CrossSectionalInputRow {
                instrument_id: "a".into(),
                observation_time_ms: 10,
                slots: vec![FactorSlotCell::Available(value(1.0))],
            },
            CrossSectionalInputRow {
                instrument_id: "b".into(),
                observation_time_ms: 10,
                slots: vec![FactorSlotCell::Unavailable(
                    crate::FactorUnavailabilityReason::MissingInput,
                )],
            },
        ];
        assert!(validate_cross_sectional_batch(&rows, &["a".into(), "b".into()], 1).is_ok());
        assert!(validate_cross_sectional_batch(&rows, &["b".into(), "a".into()], 1).is_err());
        assert!(validate_cross_sectional_batch(&rows, &["a".into(), "a".into()], 1).is_err());
    }

    #[test]
    fn result_validation_rejects_identity_and_output_mismatches() {
        let result = FactorResult {
            instrument_id: "a".into(),
            observation_time_ms: 1,
            values: Some(vec![NamedFactorOutput {
                name: "wrong".into(),
                value: 1.0,
            }]),
        };
        assert!(
            validate_factor_results(&[result], &["a".into()], &[1], &["value".into()]).is_err()
        );
    }
}
