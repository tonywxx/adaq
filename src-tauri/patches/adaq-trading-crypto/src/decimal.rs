//! `decimalToPrecision` — ccxt 数字精度格式化(ADR-0004)。
//!
//! 三种精度模式 × 四种舍入 × 两种填充,语义对齐 ccxt
//! (`ts/src/base/functions/number.ts` / `python/ccxt/base/decimal_to_precision.py`)。
//! 基于 `rust_decimal`(28 位有效数字),以 Python ccxt(差分测试参考实现)
//! 的舍入语义为准:ROUND = 四舍五入(半值远离零)。

use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::error::{Error, ErrorKind, Result};

/// 精度模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionMode {
    /// 保留 N 位小数。
    DecimalPlaces,
    /// 保留 N 位有效数字。
    SignificantDigits,
    /// 按最小刻度(如 `0.01`)取整。
    TickSize,
}

/// 舍入模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    /// 四舍五入(半值远离零,同 Python `ROUND_HALF_UP`)。
    Round,
    /// 直接截断(向零)。
    Truncate,
    /// 远离零取整(1.01 -> 2)。
    RoundUp,
    /// 向零取整。
    RoundDown,
}

/// 输出填充模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingMode {
    /// 不补零,去掉尾随零。
    NoPadding,
    /// 补零到目标小数位。
    PadWithZero,
}

fn rounding_strategy(mode: RoundingMode) -> RoundingStrategy {
    match mode {
        RoundingMode::Round => RoundingStrategy::MidpointAwayFromZero,
        RoundingMode::Truncate => RoundingStrategy::ToZero,
        RoundingMode::RoundUp => RoundingStrategy::AwayFromZero,
        RoundingMode::RoundDown => RoundingStrategy::ToZero,
    }
}

/// 10^n(n 较小,避免引入 maths feature)。
fn pow10(n: u32) -> Decimal {
    let mut r = Decimal::ONE;
    for _ in 0..n {
        r *= Decimal::from(10u32);
    }
    r
}

/// 在指定小数位(dp,可为负表示按 10^|dp| 取整)舍入。
fn round_dp(value: Decimal, dp: i64, rounding: RoundingMode) -> Decimal {
    let strategy = rounding_strategy(rounding);
    if dp >= 0 {
        value.round_dp_with_strategy(dp as u32, strategy)
    } else {
        let scale = pow10((-dp) as u32);
        (value / scale).round_dp_with_strategy(0, strategy) * scale
    }
}

/// `floor(log10(v))`,v > 0,基于十进制字符串计数(避免浮点)。
fn floor_log10(v: Decimal) -> i64 {
    let s = v.to_string().trim_start_matches('-').to_string();
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.clone(), String::new()),
    };
    let trimmed = int_part.trim_start_matches('0');
    if !trimmed.is_empty() {
        return trimmed.len() as i64 - 1;
    }
    // v < 1:数小数部分的连续前导零
    let mut zeros = 0i64;
    for c in frac_part.chars() {
        if c == '0' {
            zeros += 1;
        } else {
            break;
        }
    }
    -1 - zeros
}

/// 按固定小数位输出(补零,如 `12.3` dp=4 -> `12.3000`)。
fn format_fixed(value: Decimal, dp: u32) -> String {
    let s = value.normalize().to_string();
    let (neg, abs) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", s.as_str()),
    };
    let (int_part, frac_part) = match abs.split_once('.') {
        Some((i, f)) => (i, f),
        None => (abs, ""),
    };
    if dp == 0 {
        return format!("{neg}{int_part}");
    }
    let mut frac = frac_part.to_string();
    while frac.len() < dp as usize {
        frac.push('0');
    }
    format!("{neg}{int_part}.{frac}")
}

/// 输出格式化:按 padding 决定是否保留尾随零。
fn format_result(value: Decimal, dp: i64, padding: PaddingMode) -> String {
    match padding {
        PaddingMode::PadWithZero if dp > 0 => format_fixed(value, dp as u32),
        _ => value.normalize().to_string(),
    }
}

/// 数字精度格式化(对齐 ccxt `decimalToPrecision`)。
///
/// `precision` 参数:DecimalPlaces/SignificantDigits 时为位数(如 `"2"`),
/// TickSize 时为刻度字符串(如 `"0.01"`)。
pub fn decimal_to_precision(
    value: &str,
    mode: PrecisionMode,
    precision: &str,
    rounding: RoundingMode,
    padding: PaddingMode,
) -> Result<String> {
    let value = Decimal::from_str(value.trim()).map_err(|e| {
        Error::new(
            ErrorKind::BadRequest,
            format!("invalid number {value:?}: {e}"),
        )
    })?;

    match mode {
        PrecisionMode::DecimalPlaces => {
            let dp = parse_digits(precision)?;
            let rounded = round_dp(value, dp as i64, rounding);
            Ok(format_result(rounded, dp as i64, padding))
        }
        PrecisionMode::SignificantDigits => {
            let sig = parse_digits(precision)?;
            let rounded = if value.is_zero() {
                value
            } else {
                let exp = floor_log10(value.abs());
                let dp = sig as i64 - 1 - exp;
                round_dp(value, dp, rounding)
            };
            Ok(format_result(rounded, sig as i64 - 1, padding))
        }
        PrecisionMode::TickSize => {
            let tick = Decimal::from_str(precision.trim()).map_err(|e| {
                Error::new(
                    ErrorKind::BadRequest,
                    format!("invalid tick size {precision:?}: {e}"),
                )
            })?;
            if tick.is_zero() {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    "tick size must not be zero",
                ));
            }
            let scaled = value / tick;
            let tick_dp = tick_scale(tick);
            let rounded = round_dp(scaled, 0, rounding) * tick;
            Ok(format_result(rounded, tick_dp, padding))
        }
    }
}

fn parse_digits(s: &str) -> Result<u32> {
    s.trim().parse::<u32>().map_err(|e| {
        Error::new(
            ErrorKind::BadRequest,
            format!("invalid precision {s:?}: {e}"),
        )
    })
}

/// 刻度的小数位数(如 `0.01` -> 2)。
fn tick_scale(tick: Decimal) -> i64 {
    let s = tick.normalize().to_string();
    match s.split_once('.') {
        Some((_, f)) => f.len() as i64,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PaddingMode::*;
    use PrecisionMode::*;
    use RoundingMode::*;

    fn fmt(value: &str, mode: PrecisionMode, precision: &str, rounding: RoundingMode) -> String {
        decimal_to_precision(value, mode, precision, rounding, NoPadding).unwrap()
    }

    #[test]
    fn decimal_places_rounding() {
        assert_eq!(fmt("12.3456000", DecimalPlaces, "2", Round), "12.35");
        assert_eq!(fmt("12.3456000", DecimalPlaces, "2", Truncate), "12.34");
        assert_eq!(fmt("12.3456000", DecimalPlaces, "2", RoundUp), "12.35");
        assert_eq!(fmt("12.3456000", DecimalPlaces, "2", RoundDown), "12.34");
    }

    #[test]
    fn decimal_places_padding() {
        let r = decimal_to_precision("12.3", DecimalPlaces, "4", Round, PadWithZero).unwrap();
        assert_eq!(r, "12.3000");
    }

    #[test]
    fn significant_digits() {
        assert_eq!(fmt("0.123456", SignificantDigits, "3", Round), "0.123");
        assert_eq!(fmt("0.123456", SignificantDigits, "4", Round), "0.1235");
        assert_eq!(fmt("1.23456", SignificantDigits, "3", Round), "1.23");
        assert_eq!(fmt("12.3456", SignificantDigits, "3", Round), "12.3");
        assert_eq!(fmt("123.456", SignificantDigits, "3", Round), "123");
        assert_eq!(fmt("0.0012345", SignificantDigits, "3", Round), "0.00123");
    }

    #[test]
    fn tick_size() {
        assert_eq!(fmt("123.456", TickSize, "0.01", Round), "123.46");
        assert_eq!(fmt("123.456", TickSize, "0.001", Round), "123.456");
        assert_eq!(fmt("123.456", TickSize, "0.05", Round), "123.45");
        assert_eq!(fmt("123.456", TickSize, "1", Round), "123");
        assert_eq!(fmt("0.005", TickSize, "0.01", Round), "0.01");
        assert_eq!(fmt("123.456", TickSize, "0.01", Truncate), "123.45");
    }

    #[test]
    fn negative_values() {
        assert_eq!(fmt("-1.234", DecimalPlaces, "2", Truncate), "-1.23");
        assert_eq!(fmt("-1.235", DecimalPlaces, "2", Round), "-1.24");
        assert_eq!(fmt("-1.299", DecimalPlaces, "1", RoundUp), "-1.3");
    }

    #[test]
    fn invalid_inputs() {
        assert!(decimal_to_precision("abc", DecimalPlaces, "2", Round, NoPadding).is_err());
        assert!(decimal_to_precision("1.5", TickSize, "0", Round, NoPadding).is_err());
    }
}
