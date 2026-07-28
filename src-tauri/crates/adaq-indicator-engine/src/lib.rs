//! Tauri-independent, pinned TA-Lib RSI engine.

mod bindings;

use std::sync::OnceLock;

const ENGINE_VERSION: &str = "adaq-indicator-engine@1.0.0";
const TA_LIB_VERSION: &str = "0.7.1";

static INITIALIZATION: OnceLock<Result<(), EngineError>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineIdentity {
    pub engine_version: &'static str,
    pub ta_lib_version: &'static str,
    pub build_id: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    Initialization {
        code: &'static str,
        ret_code: i32,
        ret_code_name: &'static str,
    },
    InvalidRequest {
        code: &'static str,
    },
    InvalidSegment {
        code: &'static str,
    },
    TaLib {
        code: &'static str,
        ret_code: i32,
        ret_code_name: &'static str,
    },
    NonFiniteOutput {
        code: &'static str,
        index: usize,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.code())
    }
}

impl std::error::Error for EngineError {}

impl EngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Initialization { code, .. }
            | Self::InvalidRequest { code }
            | Self::InvalidSegment { code }
            | Self::TaLib { code, .. }
            | Self::NonFiniteOutput { code, .. } => code,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsiRequest {
    pub time_period: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinuousBarSegment {
    close: Vec<f64>,
}

impl ContinuousBarSegment {
    pub fn new(close: Vec<f64>) -> Result<Self, EngineError> {
        if close.is_empty() {
            return Err(EngineError::InvalidSegment {
                code: "empty-continuous-bar-segment",
            });
        }
        if close.iter().any(|value| !value.is_finite()) {
            return Err(EngineError::InvalidSegment {
                code: "non-finite-continuous-bar-segment",
            });
        }
        Ok(Self { close })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledRsi {
    time_period: i32,
    lookback: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorEngine {
    identity: EngineIdentity,
}

impl IndicatorEngine {
    pub fn initialize() -> Result<Self, EngineError> {
        initialize_once(&INITIALIZATION, initialize_talib)?;
        Ok(Self {
            identity: EngineIdentity {
                engine_version: ENGINE_VERSION,
                ta_lib_version: TA_LIB_VERSION,
                build_id: env!("ADAQ_INDICATOR_ENGINE_BUILD_ID"),
            },
        })
    }

    pub fn identity(&self) -> &EngineIdentity {
        &self.identity
    }

    pub fn compile_rsi(&self, request: RsiRequest) -> Result<CompiledRsi, EngineError> {
        let time_period = i32::try_from(request.time_period)
            .ok()
            .filter(|period| (2..=100_000).contains(period))
            .ok_or(EngineError::InvalidRequest {
                code: "invalid-rsi-time-period",
            })?;
        let lookback = unsafe { bindings::TA_RSI_Lookback(time_period) };
        if lookback < 0 {
            return Err(EngineError::InvalidRequest {
                code: "invalid-rsi-time-period",
            });
        }
        Ok(CompiledRsi {
            time_period,
            lookback: lookback as usize,
        })
    }

    pub fn evaluate_rsi(
        &self,
        request: CompiledRsi,
        segment: &ContinuousBarSegment,
    ) -> Result<Vec<Option<f64>>, EngineError> {
        let mut output = vec![None; segment.close.len()];
        if segment.close.len() <= request.lookback {
            return Ok(output);
        }
        let mut raw = vec![0.0; segment.close.len() - request.lookback];
        let mut output_begin = 0;
        let mut output_count = 0;
        let ret_code = unsafe {
            bindings::TA_RSI(
                0,
                i32::try_from(segment.close.len() - 1).map_err(|_| {
                    EngineError::InvalidSegment {
                        code: "continuous-bar-segment-too-large",
                    }
                })?,
                segment.close.as_ptr(),
                request.time_period,
                &mut output_begin,
                &mut output_count,
                raw.as_mut_ptr(),
            )
        };
        check_ta(ret_code)?;
        let start = usize::try_from(output_begin).map_err(|_| EngineError::TaLib {
            code: "invalid-ta-lib-output",
            ret_code,
            ret_code_name: ret_code_name(ret_code),
        })?;
        let count = usize::try_from(output_count).map_err(|_| EngineError::TaLib {
            code: "invalid-ta-lib-output",
            ret_code,
            ret_code_name: ret_code_name(ret_code),
        })?;
        if start != request.lookback || count != raw.len() {
            return Err(EngineError::TaLib {
                code: "invalid-ta-lib-output",
                ret_code,
                ret_code_name: ret_code_name(ret_code),
            });
        }
        for (index, value) in raw.into_iter().enumerate() {
            if !value.is_finite() {
                return Err(EngineError::NonFiniteOutput {
                    code: "non-finite-indicator-output",
                    index: start + index,
                });
            }
            output[start + index] = Some(value);
        }
        Ok(output)
    }
}

fn initialize_once(
    initialization: &OnceLock<Result<(), EngineError>>,
    initialize: impl FnOnce() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    initialization.get_or_init(initialize).clone()
}

fn initialize_talib() -> Result<(), EngineError> {
    check_initialization(unsafe { bindings::TA_Initialize() })?;
    check_initialization(unsafe { bindings::TA_SetUnstablePeriod(bindings::TA_FUNC_UNST_ALL, 0) })?;
    check_initialization(unsafe {
        bindings::TA_SetCompatibility(bindings::TA_COMPATIBILITY_DEFAULT)
    })?;
    check_initialization(unsafe {
        bindings::TA_RestoreCandleDefaultSettings(bindings::TA_ALL_CANDLE_SETTINGS)
    })
}

fn check_initialization(ret_code: i32) -> Result<(), EngineError> {
    if ret_code == bindings::TA_SUCCESS {
        Ok(())
    } else {
        Err(EngineError::Initialization {
            code: "ta-lib-initialization-failed",
            ret_code,
            ret_code_name: ret_code_name(ret_code),
        })
    }
}

fn check_ta(ret_code: i32) -> Result<(), EngineError> {
    if ret_code == bindings::TA_SUCCESS {
        Ok(())
    } else {
        Err(EngineError::TaLib {
            code: "ta-lib-error",
            ret_code,
            ret_code_name: ret_code_name(ret_code),
        })
    }
}

fn ret_code_name(ret_code: i32) -> &'static str {
    match ret_code {
        0 => "TA_SUCCESS",
        1 => "TA_LIB_NOT_INITIALIZE",
        2 => "TA_BAD_PARAM",
        3 => "TA_ALLOC_ERR",
        12 => "TA_OUT_OF_RANGE_START_INDEX",
        13 => "TA_OUT_OF_RANGE_END_INDEX",
        5000 => "TA_INTERNAL_ERROR",
        _ => "TA_UNKNOWN_ERR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn initialization_failure_is_sticky() {
        let once = OnceLock::new();
        let first = initialize_once(&once, || check_initialization(2));
        let second = initialize_once(&once, || Ok(()));
        assert_eq!(first, second);
        assert!(matches!(
            first,
            Err(EngineError::Initialization {
                ret_code: 2,
                ret_code_name: "TA_BAD_PARAM",
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_rsi_request() {
        let engine = IndicatorEngine::initialize().unwrap();
        assert_eq!(
            engine
                .compile_rsi(RsiRequest { time_period: 1 })
                .unwrap_err()
                .code(),
            "invalid-rsi-time-period"
        );
    }

    #[test]
    fn preserves_ta_return_codes() {
        assert_eq!(
            check_ta(12),
            Err(EngineError::TaLib {
                code: "ta-lib-error",
                ret_code: 12,
                ret_code_name: "TA_OUT_OF_RANGE_START_INDEX",
            })
        );
    }

    #[test]
    fn rsi_aligns_exact_lookback_warmup() {
        let engine = IndicatorEngine::initialize().unwrap();
        let request = engine.compile_rsi(RsiRequest { time_period: 2 }).unwrap();
        let segment = ContinuousBarSegment::new(vec![1.0, 2.0, 3.0, 2.0, 4.0]).unwrap();
        let output = engine.evaluate_rsi(request, &segment).unwrap();
        assert_eq!(&output[..2], &[None, None]);
        assert!(
            output[2..]
                .iter()
                .all(|value| value.is_some_and(f64::is_finite))
        );
    }

    #[test]
    fn same_build_repeats_and_evaluates_concurrently() {
        let expected = IndicatorEngine::initialize().unwrap().identity().build_id;
        let joins: Vec<_> = (0..4)
            .map(|_| {
                thread::spawn(|| {
                    let engine = IndicatorEngine::initialize().unwrap();
                    let request = engine.compile_rsi(RsiRequest { time_period: 2 }).unwrap();
                    let segment = ContinuousBarSegment::new(vec![1.0, 2.0, 3.0, 2.0, 4.0]).unwrap();
                    (
                        engine.identity().build_id,
                        engine.evaluate_rsi(request, &segment).unwrap(),
                    )
                })
            })
            .collect();
        for join in joins {
            let (build_id, output) = join.join().unwrap();
            assert_eq!(build_id, expected);
            assert_eq!(
                output,
                IndicatorEngine::initialize()
                    .unwrap()
                    .evaluate_rsi(
                        IndicatorEngine::initialize()
                            .unwrap()
                            .compile_rsi(RsiRequest { time_period: 2 })
                            .unwrap(),
                        &ContinuousBarSegment::new(vec![1.0, 2.0, 3.0, 2.0, 4.0]).unwrap(),
                    )
                    .unwrap()
            );
        }
    }
}
