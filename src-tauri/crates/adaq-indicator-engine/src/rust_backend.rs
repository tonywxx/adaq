//! Pure-Rust backend backed by `adaq-talib` (zero-FFI).
//!
//! Every indicator is dispatched by its catalog `raw_name` to the matching `adaq-talib`
//! function. All `adaq-talib` outputs are equal-length `Vec<f64>` buffers with a leading
//! `NaN` warmup, which maps 1:1 onto the engine's `Option<f64>`/`Option<i32>` columns.
//!
//! A few indicators (`MACD` family, `APO`, `PPO`, `STOCHRSI`) are composed from `adaq-talib`
//! moving-average primitives so they honour TA-Lib's configurable `MA Type` exactly
//! (the primitive `macd_*`/`apo`/`ppo` helpers hard-code EMA and a fixed signal MA type).

use super::{
    CompiledIndicator, CompiledRsi, ContinuousBarSegment, EngineError, IndicatorColumn,
    IndicatorDefinition, MarketField, OhlcvSegment, ParameterValue, RsiRequest,
};
use adaq_talib::{
    cycle::{
        HtPhasor, HtSine, ht_dcperiod_with_output, ht_dcphase_with_output, ht_phasor_with_output,
        ht_sine_with_output, ht_trendline_with_output, ht_trendmode_with_output, mama,
    },
    error::TaError,
    math_ops::{
        MinMax, MinMaxIndex, add_with_output, div_with_output, max_index_with_output,
        max_with_output, min_index_with_output, min_with_output, minmax_index_with_output,
        minmax_with_output, mult_with_output, sub_with_output, sum_with_output,
    },
    math_trans::{
        acos_with_output, asin_with_output, atan_with_output, ceil_with_output, cos_with_output,
        cosh_with_output, exp_with_output, floor_with_output, ln_with_output, log10_with_output,
        sin_with_output, sinh_with_output, sqrt_with_output, tan_with_output, tanh_with_output,
    },
    momentum::{
        Aroon, Macd, Stoch, StochF, adx_with_output, adxr_with_output, aroon_with_output,
        bop_with_output, cci_with_output, cmo_with_output, dx_with_output, imi_with_output,
        macd_fix_with_output, macd_with_output, mfi_with_output, minus_di_with_output,
        minus_dm_with_output, mom_with_output, plus_di_with_output, plus_dm_with_output,
        roc_with_output, rocp_with_output, rocr_with_output, rocr100_with_output, rsi_with_output,
        stoch_f_with_output, stoch_rsi_with_output, stoch_with_output, trix, ultosc_with_output,
        willr_with_output,
    },
    overlap::{
        AccBands, Bbands, MaType, accbands_with_output, bbands_with_output, dema_with_output,
        ema_with_output, kama_with_output, ma_with_output, midpoint_with_output,
        midprice_with_output, sar_with_output, sarext_with_output, sma_with_output, t3_with_output,
        tema_with_output, trima_with_output, wma_with_output,
    },
    price_transform::{
        avgdev_with_output, avgprice_with_output, medprice_with_output, typprice_with_output,
        wclprice_with_output,
    },
    stat::{
        beta_with_output, correl_with_output, linear_reg_angle_with_output,
        linear_reg_intercept_with_output, linear_reg_slope_with_output, linear_reg_with_output,
        stddev_with_output, tsf_with_output, var_with_output,
    },
    volatility::{atr_with_output, natr_with_output, trange_with_output},
    volume::{ad_with_output, adosc_with_output, obv_with_output},
};

// Candlestick patterns live in the `pattern` module (one `cdl_*_with_output` per pattern).
use adaq_talib::pattern::*;

pub fn evaluate(
    request: &CompiledIndicator,
    segment: &OhlcvSegment,
) -> Result<Vec<(String, IndicatorColumn)>, EngineError> {
    let n = segment.close.len();
    if n <= request.lookback {
        return Ok(request
            .outputs
            .iter()
            .map(|index| {
                let output = &request.definition.outputs[*index];
                let column = if output.kind == "Integer Array" {
                    IndicatorColumn::Integer(vec![None; n])
                } else {
                    IndicatorColumn::Real(vec![None; n])
                };
                (output.id.clone(), column)
            })
            .collect());
    }
    let raw = compute(
        &request.definition,
        &request.parameters,
        &request.real_inputs,
        segment,
    )?;
    let mut result = Vec::with_capacity(request.outputs.len());
    for index in &request.outputs {
        let output = &request.definition.outputs[*index];
        let values = &raw[*index];
        let column = if output.kind == "Integer Array" {
            IndicatorColumn::Integer(
                values
                    .iter()
                    .map(|value| {
                        if value.is_finite() {
                            Some(*value as i32)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
        } else {
            IndicatorColumn::Real(
                values
                    .iter()
                    .map(|value| {
                        if value.is_finite() {
                            Some(*value)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
        };
        result.push((output.id.clone(), column));
    }
    Ok(result)
}

pub fn lookback(
    definition: &IndicatorDefinition,
    parameters: &[ParameterValue],
) -> Result<usize, EngineError> {
    let segment = synthetic_segment();
    let real_inputs = default_real_inputs(definition);
    match compute(definition, parameters, &real_inputs, &segment) {
        Ok(raw) => {
            let mut lookback = 0;
            for column in &raw {
                let leading = column.iter().take_while(|value| !value.is_finite()).count();
                lookback = lookback.max(leading);
            }
            Ok(lookback)
        }
        Err(_) => Ok(0),
    }
}

pub fn compile_rsi(request: &RsiRequest) -> Result<CompiledRsi, EngineError> {
    let time_period = i32::try_from(request.time_period)
        .ok()
        .filter(|period| (2..=100_000).contains(period))
        .ok_or(EngineError::InvalidRequest {
            code: "invalid-rsi-time-period",
        })?;
    let lookback = rsi_lookback(time_period)?;
    Ok(CompiledRsi {
        time_period,
        lookback,
    })
}

pub fn evaluate_rsi(
    request: &CompiledRsi,
    segment: &ContinuousBarSegment,
) -> Result<Vec<Option<f64>>, EngineError> {
    let n = segment.close.len();
    let mut output = vec![None; n];
    if n <= request.lookback {
        return Ok(output);
    }
    let mut raw = vec![f64::NAN; n];
    rsi_with_output(&segment.close, request.time_period as usize, &mut raw).map_err(talib_err)?;
    for (index, value) in raw.iter().enumerate() {
        if value.is_finite() {
            output[index] = Some(*value);
        }
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Allocate an equal-length `NaN`-prefilled buffer and run `f` to fill it.
fn single(
    n: usize,
    f: impl FnOnce(&mut [f64]) -> Result<(), EngineError>,
) -> Result<Vec<Vec<f64>>, EngineError> {
    let mut out = vec![f64::NAN; n];
    f(&mut out)?;
    Ok(vec![out])
}

fn compute(
    definition: &IndicatorDefinition,
    parameters: &[ParameterValue],
    real_inputs: &[MarketField],
    segment: &OhlcvSegment,
) -> Result<Vec<Vec<f64>>, EngineError> {
    let n = segment.close.len();
    let o = segment.open.as_slice();
    let h = segment.high.as_slice();
    let l = segment.low.as_slice();
    let c = segment.close.as_slice();
    let reals: Vec<&[f64]> = definition
        .inputs
        .iter()
        .filter(|input| matches!(input.kind.as_str(), "Double Array" | "Volume"))
        .enumerate()
        .map(|(index, _)| segment.field(real_inputs[index]))
        .collect();
    let r0 = reals.first().copied().unwrap_or(&[]);
    let _r1 = reals.get(1).copied().unwrap_or(&[]);
    let vol = reals.last().copied().unwrap_or(&[]);

    let p_int = |index: usize| -> i32 {
        if let ParameterValue::Integer(value) = &parameters[index] {
            *value
        } else {
            0
        }
    };
    let p_real = |index: usize| -> f64 {
        if let ParameterValue::Real(value) = &parameters[index] {
            *value
        } else {
            0.0
        }
    };
    let ma_t = |index: usize| -> MaType { ma_type_from_int(p_int(index)) };
    let u = |index: usize| -> usize { p_int(index).max(0) as usize };

    match definition.raw_name.as_str() {
        // ---- Math Transform (Double Array, no params) ----
        "ACOS" => single(n, |out| acos_with_output(r0, out).map_err(talib_err)),
        "ASIN" => single(n, |out| asin_with_output(r0, out).map_err(talib_err)),
        "ATAN" => single(n, |out| atan_with_output(r0, out).map_err(talib_err)),
        "CEIL" => single(n, |out| ceil_with_output(r0, out).map_err(talib_err)),
        "COS" => single(n, |out| cos_with_output(r0, out).map_err(talib_err)),
        "COSH" => single(n, |out| cosh_with_output(r0, out).map_err(talib_err)),
        "EXP" => single(n, |out| exp_with_output(r0, out).map_err(talib_err)),
        "FLOOR" => single(n, |out| floor_with_output(r0, out).map_err(talib_err)),
        "LN" => single(n, |out| ln_with_output(r0, out).map_err(talib_err)),
        "LOG10" => single(n, |out| log10_with_output(r0, out).map_err(talib_err)),
        "SIN" => single(n, |out| sin_with_output(r0, out).map_err(talib_err)),
        "SINH" => single(n, |out| sinh_with_output(r0, out).map_err(talib_err)),
        "SQRT" => single(n, |out| sqrt_with_output(r0, out).map_err(talib_err)),
        "TAN" => single(n, |out| tan_with_output(r0, out).map_err(talib_err)),
        "TANH" => single(n, |out| tanh_with_output(r0, out).map_err(talib_err)),

        // ---- Single Double-Array input + time-period ----
        "SMA" => single(n, |out| sma_with_output(r0, u(0), out).map_err(talib_err)),
        "EMA" => single(n, |out| ema_with_output(r0, u(0), out).map_err(talib_err)),
        "WMA" => single(n, |out| wma_with_output(r0, u(0), out).map_err(talib_err)),
        "DEMA" => single(n, |out| dema_with_output(r0, u(0), out).map_err(talib_err)),
        "TEMA" => single(n, |out| tema_with_output(r0, u(0), out).map_err(talib_err)),
        "TRIMA" => single(n, |out| trima_with_output(r0, u(0), out).map_err(talib_err)),
        "KAMA" => single(n, |out| kama_with_output(r0, u(0), out).map_err(talib_err)),
        "MIDPOINT" => single(n, |out| {
            midpoint_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "CMO" => single(n, |out| cmo_with_output(r0, u(0), out).map_err(talib_err)),
        "MOM" => single(n, |out| mom_with_output(r0, u(0), out).map_err(talib_err)),
        "ROC" => single(n, |out| roc_with_output(r0, u(0), out).map_err(talib_err)),
        "ROCP" => single(n, |out| rocp_with_output(r0, u(0), out).map_err(talib_err)),
        "ROCR" => single(n, |out| rocr_with_output(r0, u(0), out).map_err(talib_err)),
        "ROCR100" => single(n, |out| {
            rocr100_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "RSI" => single(n, |out| rsi_with_output(r0, u(0), out).map_err(talib_err)),
        "AVGDEV" => single(n, |out| {
            avgdev_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "MAX" => single(n, |out| max_with_output(r0, u(0), out).map_err(talib_err)),
        "MIN" => single(n, |out| min_with_output(r0, u(0), out).map_err(talib_err)),
        "SUM" => single(n, |out| sum_with_output(r0, u(0), out).map_err(talib_err)),
        "MAXINDEX" => single(n, |out| {
            max_index_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "MININDEX" => single(n, |out| {
            min_index_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "LINEARREG" => single(n, |out| {
            linear_reg_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "LINEARREG_ANGLE" => single(n, |out| {
            linear_reg_angle_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "LINEARREG_INTERCEPT" => single(n, |out| {
            linear_reg_intercept_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "LINEARREG_SLOPE" => single(n, |out| {
            linear_reg_slope_with_output(r0, u(0), out).map_err(talib_err)
        }),
        "TSF" => single(n, |out| tsf_with_output(r0, u(0), out).map_err(talib_err)),
        "STDDEV" => single(n, |out| {
            stddev_with_output(r0, u(0), p_real(1), out).map_err(talib_err)
        }),
        "VAR" => single(n, |out| {
            var_with_output(r0, u(0), p_real(1), out).map_err(talib_err)
        }),
        "TRIX" => {
            let v = trix(r0, u(0)).map_err(talib_err)?;
            Ok(vec![v])
        }

        // ---- Price / volume inputs ----
        "TRANGE" => single(n, |out| trange_with_output(h, l, c, out).map_err(talib_err)),
        "ATR" => single(n, |out| {
            atr_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "NATR" => single(n, |out| {
            natr_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "CCI" => single(n, |out| {
            cci_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "WILLR" => single(n, |out| {
            willr_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "MFI" => single(n, |out| {
            mfi_with_output(h, l, c, vol, u(0), out).map_err(talib_err)
        }),
        "ADX" => single(n, |out| {
            adx_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "ADXR" => single(n, |out| {
            adxr_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "DX" => single(n, |out| {
            dx_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "AROONOSC" => {
            // `adaq-talib`'s `aroon_osc_with_output` inherits the same up/down swap as
            // `aroon_with_output` and therefore returns `true_down - true_up`. TA-Lib defines
            // `AROONOSC = true_up - true_down`, so we derive it directly from the (un-swapped)
            // `Aroon` struct: `a.down` holds the true up line, `a.up` holds the true down line.
            let mut a = Aroon {
                up: vec![f64::NAN; n],
                down: vec![f64::NAN; n],
            };
            aroon_with_output(h, l, u(0), &mut a).map_err(talib_err)?;
            let out = a
                .down
                .iter()
                .zip(&a.up)
                .map(|(up, down)| up - down)
                .collect::<Vec<_>>();
            Ok(vec![out])
        }
        "PLUS_DM" => single(n, |out| {
            plus_dm_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "MINUS_DM" => single(n, |out| {
            minus_dm_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "PLUS_DI" => single(n, |out| {
            plus_di_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "MINUS_DI" => single(n, |out| {
            minus_di_with_output(h, l, c, u(0), out).map_err(talib_err)
        }),
        "IMI" => single(n, |out| imi_with_output(o, c, u(0), out).map_err(talib_err)),
        "MIDPRICE" => single(n, |out| {
            midprice_with_output(h, l, u(0), out).map_err(talib_err)
        }),
        "AD" => single(n, |out| {
            ad_with_output(h, l, c, vol, out).map_err(talib_err)
        }),
        "ADOSC" => single(n, |out| {
            adosc_with_output(h, l, c, vol, u(0), u(1), out).map_err(talib_err)
        }),
        "OBV" => single(n, |out| obv_with_output(r0, vol, out).map_err(talib_err)),
        "BOP" => single(n, |out| bop_with_output(o, h, l, c, out).map_err(talib_err)),
        "ULTOSC" => single(n, |out| {
            ultosc_with_output(h, l, c, u(0), u(1), u(2), out).map_err(talib_err)
        }),
        "AVGPRICE" => single(n, |out| {
            avgprice_with_output(h, l, c, o, out).map_err(talib_err)
        }),
        "MEDPRICE" => single(n, |out| medprice_with_output(h, l, out).map_err(talib_err)),
        "TYPPRICE" => single(n, |out| {
            typprice_with_output(h, l, c, out).map_err(talib_err)
        }),
        "WCLPRICE" => single(n, |out| {
            wclprice_with_output(h, l, c, out).map_err(talib_err)
        }),

        // ---- Two Double-Array inputs ----
        "ADD" => single(n, |out| add_with_output(r0, _r1, out).map_err(talib_err)),
        "SUB" => single(n, |out| sub_with_output(r0, _r1, out).map_err(talib_err)),
        "MULT" => single(n, |out| mult_with_output(r0, _r1, out).map_err(talib_err)),
        "DIV" => single(n, |out| div_with_output(r0, _r1, out).map_err(talib_err)),
        "CORREL" => single(n, |out| {
            correl_with_output(r0, _r1, u(0), out).map_err(talib_err)
        }),
        "BETA" => single(n, |out| {
            beta_with_output(r0, _r1, u(0), out).map_err(talib_err)
        }),

        // ---- MA-type aware overlaps / oscillators (compose to honour MA type) ----
        "MA" => single(n, |out| {
            ma_with_output(r0, u(0), ma_t(1), out).map_err(talib_err)
        }),
        "APO" => Ok(vec![compose_apo_ppo(r0, u(0), u(1), ma_t(2), n, false)?]),
        "PPO" => Ok(vec![compose_apo_ppo(r0, u(0), u(1), ma_t(2), n, true)?]),
        "BBANDS" => {
            let mut b = Bbands {
                upper: vec![f64::NAN; n],
                middle: vec![f64::NAN; n],
                lower: vec![f64::NAN; n],
            };
            bbands_with_output(r0, u(0), p_real(1), p_real(2), ma_t(3), &mut b)
                .map_err(talib_err)?;
            Ok(vec![b.upper, b.middle, b.lower])
        }
        "ACCBANDS" => {
            let mut b = AccBands {
                upper: vec![f64::NAN; n],
                middle: vec![f64::NAN; n],
                lower: vec![f64::NAN; n],
            };
            accbands_with_output(h, l, c, u(0), &mut b).map_err(talib_err)?;
            Ok(vec![b.upper, b.middle, b.lower])
        }
        "SAR" => single(n, |out| {
            sar_with_output(h, l, p_real(0), p_real(1), out).map_err(talib_err)
        }),
        "SAREXT" => single(n, |out| {
            sarext_with_output(
                h,
                l,
                p_real(0),
                p_real(1),
                p_real(2),
                p_real(3),
                p_real(4),
                p_real(5),
                p_real(6),
                p_real(7),
                out,
            )
            .map_err(talib_err)
        }),
        "T3" => single(n, |out| {
            t3_with_output(r0, u(0), p_real(1), out).map_err(talib_err)
        }),

        // ---- MACD family ----
        "MACD" => {
            let mut m = Macd {
                macd: vec![f64::NAN; n],
                signal: vec![f64::NAN; n],
                hist: vec![f64::NAN; n],
            };
            macd_with_output(r0, u(0), u(1), u(2), &mut m).map_err(talib_err)?;
            Ok(vec![m.macd, m.signal, m.hist])
        }
        "MACDFIX" => {
            // TA-Lib `TA_MACDFIX` hard-codes the fast/slow smoothing factors to 0.15 / 0.075
            // (the historic MACD constants) rather than `2/(period+1)`, which is why its macd
            // line differs from `MACD`. `adaq-talib`'s `macd_fix_with_output` reproduces this
            // exactly; the (single) catalog parameter is the signal period.
            let mut m = Macd {
                macd: vec![f64::NAN; n],
                signal: vec![f64::NAN; n],
                hist: vec![f64::NAN; n],
            };
            macd_fix_with_output(r0, 12, 26, u(0), &mut m).map_err(talib_err)?;
            Ok(vec![m.macd, m.signal, m.hist])
        }
        "MACDEXT" => compose_macd(r0, u(0), u(2), u(4), ma_t(1), ma_t(3), ma_t(5)),

        // ---- Stochastic family ----
        "STOCH" => {
            let mut s = Stoch {
                slow_k: vec![f64::NAN; n],
                slow_d: vec![f64::NAN; n],
            };
            // `adaq-talib`'s `stoch_with_output` hard-codes SMA for the slow-K/slow-D
            // smoothing, which matches TA-Lib's default MA type. The catalog's MA-type
            // parameters are ignored here; non-default MA types are a documented adaq-talib
            // limitation (see improvement suggestion for `stoch_with_output`).
            stoch_with_output(h, l, c, u(0), u(1), u(3), &mut s).map_err(talib_err)?;
            Ok(vec![s.slow_k, s.slow_d])
        }
        "STOCHF" => {
            let mut s = StochF {
                fast_k: vec![f64::NAN; n],
                fast_d: vec![f64::NAN; n],
            };
            stoch_f_with_output(h, l, c, u(0), u(1), &mut s).map_err(talib_err)?;
            Ok(vec![s.fast_k, s.fast_d])
        }
        "STOCHRSI" => {
            // TA-Lib `TA_STOCHRSI` 同时输出 `fastK` 与 `fastD`，二者对齐到同一前导不稳定期
            // （`fastD` 是 `fastK` 的移动平均，但首值位置与 `fastK` 相同，而非后移 `fastD-1`）。
            // `stoch_rsi_with_output` 直接按此对齐返回两行，无需在引擎侧额外合成 `fastD`。
            // TA-Lib `TA_STOCHRSI` emits both `fastK` and `fastD` aligned to the same begin;
            // `stoch_rsi_with_output` returns both lines already aligned.
            let mut s = StochF {
                fast_k: vec![f64::NAN; n],
                fast_d: vec![f64::NAN; n],
            };
            stoch_rsi_with_output(r0, u(0), u(1), u(2), &mut s).map_err(talib_err)?;
            Ok(vec![s.fast_k, s.fast_d])
        }

        // ---- Multi-output structs ----
        "AROON" => {
            let mut a = Aroon {
                up: vec![f64::NAN; n],
                down: vec![f64::NAN; n],
            };
            aroon_with_output(h, l, u(0), &mut a).map_err(talib_err)?;
            // `adaq-talib` stores the *true* down line in `a.up` and the *true* up line in
            // `a.down` (it swapped the fields to match its own golden vectors). TA-Lib's
            // `outAroonDown`/`outAroonUp` are the canonical values, so we un-swap here.
            Ok(vec![a.up, a.down])
        }
        "MAMA" => {
            let m = mama(r0, p_real(0), p_real(1)).map_err(talib_err)?;
            Ok(vec![m.mama, m.fama])
        }
        "HT_PHASOR" => {
            let mut p = HtPhasor {
                in_phase: vec![f64::NAN; n],
                quadrature: vec![f64::NAN; n],
            };
            ht_phasor_with_output(r0, &mut p).map_err(talib_err)?;
            Ok(vec![p.in_phase, p.quadrature])
        }
        "HT_SINE" => {
            let mut s = HtSine {
                sine: vec![f64::NAN; n],
                lead_sine: vec![f64::NAN; n],
            };
            ht_sine_with_output(r0, &mut s).map_err(talib_err)?;
            Ok(vec![s.sine, s.lead_sine])
        }
        "MINMAX" => {
            let mut m = MinMax {
                min: vec![f64::NAN; n],
                max: vec![f64::NAN; n],
            };
            minmax_with_output(r0, u(0), &mut m).map_err(talib_err)?;
            Ok(vec![m.min, m.max])
        }
        "MINMAXINDEX" => {
            let mut m = MinMaxIndex {
                min_idx: vec![f64::NAN; n],
                max_idx: vec![f64::NAN; n],
            };
            minmax_index_with_output(r0, u(0), &mut m).map_err(talib_err)?;
            Ok(vec![m.min_idx, m.max_idx])
        }

        // ---- Hilbert transform single outputs ----
        "HT_DCPERIOD" => single(n, |out| ht_dcperiod_with_output(r0, out).map_err(talib_err)),
        "HT_DCPHASE" => single(n, |out| ht_dcphase_with_output(r0, out).map_err(talib_err)),
        "HT_TRENDLINE" => single(n, |out| {
            ht_trendline_with_output(r0, out).map_err(talib_err)
        }),
        "HT_TRENDMODE" => single(n, |out| {
            ht_trendmode_with_output(r0, out).map_err(talib_err)
        }),

        // ---- Candlestick patterns (single integer output) ----
        "CDL2CROWS" => single(n, |out| {
            cdl_2crows_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3BLACKCROWS" => single(n, |out| {
            cdl_3blackcrows_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3INSIDE" => single(n, |out| {
            cdl_3inside_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3LINESTRIKE" => single(n, |out| {
            cdl_3linestrike_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3OUTSIDE" => single(n, |out| {
            cdl_3outside_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3STARSINSOUTH" => single(n, |out| {
            cdl_3starsinsouth_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDL3WHITESOLDIERS" => single(n, |out| {
            cdl_3whitesoldiers_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLABANDONEDBABY" => single(n, |out| {
            cdl_abandonedbaby_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLADVANCEBLOCK" => single(n, |out| {
            cdl_advanceblock_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLBELTHOLD" => single(n, |out| {
            cdl_belthold_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLBREAKAWAY" => single(n, |out| {
            cdl_breakaway_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLCLOSINGMARUBOZU" => single(n, |out| {
            cdl_closingmarubozu_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLCONCEALBABYSWALL" => single(n, |out| {
            cdl_concealbabyswall_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLCOUNTERATTACK" => single(n, |out| {
            cdl_counterattack_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLDARKCLOUDCOVER" => single(n, |out| {
            cdl_darkcloudcover_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLDOJI" => single(n, |out| {
            cdl_doji_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLDOJISTAR" => single(n, |out| {
            cdl_dojistar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLDRAGONFLYDOJI" => single(n, |out| {
            cdl_dragonflydoji_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLENGULFING" => single(n, |out| {
            cdl_engulfing_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLEVENINGDOJISTAR" => single(n, |out| {
            cdl_eveningdojistar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLEVENINGSTAR" => single(n, |out| {
            cdl_eveningstar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLGAPSIDESIDEWHITE" => single(n, |out| {
            cdl_gapsidesidewhite_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLGRAVESTONEDOJI" => single(n, |out| {
            cdl_gravestonedoji_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHAMMER" => single(n, |out| {
            cdl_hammer_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHANGINGMAN" => single(n, |out| {
            cdl_hangingman_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHARAMI" => single(n, |out| {
            cdl_harami_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHARAMICROSS" => single(n, |out| {
            cdl_haramicross_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHIGHWAVE" => single(n, |out| {
            cdl_highwave_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHIKKAKE" => single(n, |out| {
            cdl_hikkake_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHIKKAKEMOD" => single(n, |out| {
            cdl_hikkakemod_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLHOMINGPIGEON" => single(n, |out| {
            cdl_homingpigeon_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLIDENTICAL3CROWS" => single(n, |out| {
            cdl_identical3crows_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLINNECK" => single(n, |out| {
            cdl_inneck_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLINVERTEDHAMMER" => single(n, |out| {
            cdl_invertedhammer_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLKICKING" => single(n, |out| {
            cdl_kicking_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLKICKINGBYLENGTH" => single(n, |out| {
            cdl_kickingbylength_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLLADDERBOTTOM" => single(n, |out| {
            cdl_ladderbottom_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLLONGLEGGEDDOJI" => single(n, |out| {
            cdl_longleggeddoji_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLLONGLINE" => single(n, |out| {
            cdl_longline_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLMARUBOZU" => single(n, |out| {
            cdl_marubozu_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLMATCHINGLOW" => single(n, |out| {
            cdl_matchinglow_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLMATHOLD" => single(n, |out| {
            cdl_mathold_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLMORNINGDOJISTAR" => single(n, |out| {
            cdl_morningdojistar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLMORNINGSTAR" => single(n, |out| {
            cdl_morningstar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLONNECK" => single(n, |out| {
            cdl_onneck_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLPIERCING" => single(n, |out| {
            cdl_piercing_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLRICKSHAWMAN" => single(n, |out| {
            cdl_rickshawman_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLRISEFALL3METHODS" => single(n, |out| {
            cdl_risefall3methods_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSEPARATINGLINES" => single(n, |out| {
            cdl_separatinglines_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSHOOTINGSTAR" => single(n, |out| {
            cdl_shootingstar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSHORTLINE" => single(n, |out| {
            cdl_shortline_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSPINNINGTOP" => single(n, |out| {
            cdl_spinningtop_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSTALLEDPATTERN" => single(n, |out| {
            cdl_stalledpattern_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLSTICKSANDWICH" => single(n, |out| {
            cdl_sticksandwich_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLTAKURI" => single(n, |out| {
            cdl_takuri_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLTASUKIGAP" => single(n, |out| {
            cdl_tasukigap_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLTHRUSTING" => single(n, |out| {
            cdl_thrusting_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLTRISTAR" => single(n, |out| {
            cdl_tristar_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLUNIQUE3RIVER" => single(n, |out| {
            cdl_unique3river_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLUPSIDEGAP2CROWS" => single(n, |out| {
            cdl_upsidegap2crows_with_output(o, h, l, c, out).map_err(talib_err)
        }),
        "CDLXSIDEGAP3METHODS" => single(n, |out| {
            cdl_xsidegap3methods_with_output(o, h, l, c, out).map_err(talib_err)
        }),

        _ => Err(EngineError::InvalidRequest {
            code: "rust-backend-unknown-indicator",
        }),
    }
}

// ---------------------------------------------------------------------------
// Composition helpers (honour TA-Lib MA types exactly)
// ---------------------------------------------------------------------------

fn compose_macd(
    values: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
    fast_ma: MaType,
    slow_ma: MaType,
    signal_ma: MaType,
) -> Result<Vec<Vec<f64>>, EngineError> {
    let n = values.len();
    let mut fast = vec![f64::NAN; n];
    ma_with_output(values, fast_period, fast_ma, &mut fast).map_err(talib_err)?;
    let mut slow = vec![f64::NAN; n];
    ma_with_output(values, slow_period, slow_ma, &mut slow).map_err(talib_err)?;
    let mut macd = vec![f64::NAN; n];
    for i in 0..n {
        if fast[i].is_finite() && slow[i].is_finite() {
            macd[i] = fast[i] - slow[i];
        }
    }
    // TA-Lib aligns every MACD-family output to the function's overall lookback, which is the
    // index where the signal line first becomes finite. The macd line is NaN-padded up to
    // that index (its own earlier finite values are withheld), matching TA-Lib exactly.
    let macd_first = macd.iter().position(|value| value.is_finite()).unwrap_or(n);
    let compact: Vec<f64> = macd[macd_first..].to_vec();
    let mut signal_compact = vec![f64::NAN; compact.len()];
    ma_with_output(&compact, signal_period, signal_ma, &mut signal_compact).map_err(talib_err)?;
    let signal_warmup = signal_compact
        .iter()
        .position(|value| value.is_finite())
        .unwrap_or(compact.len());
    let overall_begin = macd_first + signal_warmup;
    for value in macd.iter_mut().take(overall_begin) {
        *value = f64::NAN;
    }
    let mut signal = vec![f64::NAN; n];
    for (j, &value) in signal_compact.iter().enumerate() {
        signal[macd_first + j] = value;
    }
    let mut hist = vec![f64::NAN; n];
    for i in 0..n {
        if macd[i].is_finite() && signal[i].is_finite() {
            hist[i] = macd[i] - signal[i];
        }
    }
    Ok(vec![macd, signal, hist])
}

fn compose_apo_ppo(
    values: &[f64],
    fast_period: usize,
    slow_period: usize,
    ma_type: MaType,
    n: usize,
    as_percent: bool,
) -> Result<Vec<f64>, EngineError> {
    let mut fast = vec![f64::NAN; n];
    ma_with_output(values, fast_period, ma_type, &mut fast).map_err(talib_err)?;
    let mut slow = vec![f64::NAN; n];
    ma_with_output(values, slow_period, ma_type, &mut slow).map_err(talib_err)?;
    let mut out = vec![f64::NAN; n];
    for i in 0..n {
        if fast[i].is_finite() && slow[i].is_finite() && slow[i] != 0.0 {
            out[i] = if as_percent {
                (fast[i] - slow[i]) / slow[i] * 100.0
            } else {
                fast[i] - slow[i]
            };
        }
    }
    Ok(out)
}

fn ma_type_from_int(value: i32) -> MaType {
    match value {
        0 => MaType::Sma,
        1 => MaType::Ema,
        2 => MaType::Wma,
        3 => MaType::Dema,
        4 => MaType::Tema,
        5 => MaType::Trima,
        6 => MaType::Kama,
        7 => MaType::Mama,
        _ => MaType::Sma,
    }
}

fn talib_err(error: TaError) -> EngineError {
    let _ = error;
    EngineError::TaLib {
        code: "adaq-talib-error",
        ret_code: 0,
        ret_code_name: "ADAQ_TALIB",
    }
}

// ---------------------------------------------------------------------------
// Synthetic data for lookback computation
// ---------------------------------------------------------------------------

fn synthetic_segment() -> OhlcvSegment {
    let values: Vec<f64> = (0..512).map(|i| 0.1 + (i % 50) as f64 / 100.0).collect();
    OhlcvSegment::new(
        values.iter().map(|v| v - 0.01).collect(),
        values.iter().map(|v| v + 0.01).collect(),
        values.iter().map(|v| v - 0.02).collect(),
        values.clone(),
        (0..512).map(|i| 10.0 + i as f64).collect(),
        (0..512).map(|i| 1_000.0 + i as f64).collect(),
    )
    .unwrap()
}

fn default_real_inputs(definition: &IndicatorDefinition) -> Vec<MarketField> {
    definition
        .inputs
        .iter()
        .filter(|input| matches!(input.kind.as_str(), "Double Array" | "Volume"))
        .map(|input| {
            if input.kind == "Volume" {
                MarketField::BaseVolume
            } else {
                MarketField::Close
            }
        })
        .collect()
}

fn rsi_lookback(time_period: i32) -> Result<usize, EngineError> {
    let length = (time_period as usize).saturating_add(8).max(16);
    let close: Vec<f64> = (0..length).map(|i| 0.1 + (i % 50) as f64 / 100.0).collect();
    let mut raw = vec![f64::NAN; length];
    rsi_with_output(&close, time_period as usize, &mut raw).map_err(talib_err)?;
    Ok(raw.iter().take_while(|value| !value.is_finite()).count())
}
