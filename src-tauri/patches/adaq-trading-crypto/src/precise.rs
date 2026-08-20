//! 精确十进制运算,语义对齐 ccxt `Precise`(ADR-0004)。
//!
//! 存储为「带符号 bigint 尾数 + 非负小数位数」:值 = `integer × 10^-decimals`。
//! 与 ccxt(TS `Precise.ts` / Python `precise.py`)保持一致的解析、
//! 四则运算(除法固定 18 位小数)、取模、比较与 `to_string` 语义,
//! 服务于差分测试与资金逐位对账等极端精度场景。

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use num_bigint::BigInt;
use num_traits::{Signed, Zero};

/// 除法默认精度(位小数),与 ccxt `Precise.div` 默认一致。
pub const DIV_PRECISION: u32 = 18;

/// 精确十进制数:值 = `integer × 10^-decimals`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Precise {
    /// 带符号尾数。
    integer: BigInt,
    /// 小数位数(>= 0)。
    decimals: u32,
}

impl Precise {
    /// 零值。
    pub fn zero() -> Self {
        Self {
            integer: BigInt::zero(),
            decimals: 0,
        }
    }

    /// 解析十进制字符串,支持 `e` 科学计数法(与 ccxt 一致)。
    fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty precision string".into());
        }

        let (sign, rest) = match s.as_bytes()[0] {
            b'-' => (-1, &s[1..]),
            b'+' => (1, &s[1..]),
            _ => (1, s),
        };
        if rest.is_empty() {
            return Err(format!("invalid precision string: {s}"));
        }

        // 拆分指数部分
        let (mantissa, exponent) = match rest.find(['e', 'E']) {
            Some(idx) => {
                let exp: i64 = rest[idx + 1..]
                    .parse()
                    .map_err(|_| format!("invalid exponent in {s}"))?;
                (&rest[..idx], exp)
            }
            None => (rest, 0),
        };

        // 拆分整数/小数部分
        let (int_part, frac_part) = match mantissa.find('.') {
            Some(idx) => (&mantissa[..idx], &mantissa[idx + 1..]),
            None => (mantissa, ""),
        };
        if !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!("invalid precision string: {s}"));
        }

        // digits = int_part + frac_part,去前导零后为尾数
        let digits_raw = format!("{int_part}{frac_part}");
        let digits_trimmed = digits_raw.trim_start_matches('0');
        let digits = if digits_trimmed.is_empty() {
            "0"
        } else {
            digits_trimmed
        };

        let frac_len = frac_part.len() as i64;
        // decimals = 小数位 - 指数
        let mut decimals = frac_len - exponent;
        let mut integer = BigInt::parse_bytes(digits.as_bytes(), 10)
            .ok_or_else(|| format!("invalid precision string: {s}"))?;
        if decimals < 0 {
            // 正指数:把尾数放大,小数位归零
            integer *= BigInt::from(10u32).pow((-decimals) as u32);
            decimals = 0;
        }
        if sign < 0 && !integer.is_zero() {
            integer = -integer;
        }
        Ok(Self {
            integer,
            decimals: decimals as u32,
        })
    }

    /// 缩放到指定小数位(放大尾数)。
    fn scaled_to(&self, decimals: u32) -> BigInt {
        if decimals >= self.decimals {
            &self.integer * BigInt::from(10u32).pow(decimals - self.decimals)
        } else {
            self.integer.clone() // 调用方保证只放大
        }
    }

    /// 加法:对齐到较大小数位。
    pub fn add(&self, other: &Precise) -> Precise {
        let decimals = self.decimals.max(other.decimals);
        Precise {
            integer: self.scaled_to(decimals) + other.scaled_to(decimals),
            decimals,
        }
    }

    /// 减法:对齐到较大小数位。
    pub fn sub(&self, other: &Precise) -> Precise {
        let decimals = self.decimals.max(other.decimals);
        Precise {
            integer: self.scaled_to(decimals) - other.scaled_to(decimals),
            decimals,
        }
    }

    /// 乘法:小数位相加。
    pub fn mul(&self, other: &Precise) -> Precise {
        Precise {
            integer: &self.integer * &other.integer,
            decimals: self.decimals + other.decimals,
        }
    }

    /// 除法:ccxt 算法,`distance = precision - this.decimals + other.decimals`,
    /// 结果固定 `precision` 位小数(默认 18),向零截断。
    pub fn div(&self, other: &Precise, precision: u32) -> Precise {
        let distance = precision as i64 - self.decimals as i64 + other.decimals as i64;
        let mut numerator = self.integer.clone();
        let mut denominator = other.integer.clone();
        if distance < 0 {
            denominator *= BigInt::from(10u32).pow((-distance) as u32);
        } else {
            numerator *= BigInt::from(10u32).pow(distance as u32);
        }
        Precise {
            integer: numerator / denominator,
            decimals: precision,
        }
    }

    /// 取模:对齐到较大小数位后取余。
    pub fn rem(&self, other: &Precise) -> Precise {
        let decimals = self.decimals.max(other.decimals);
        Precise {
            integer: self.scaled_to(decimals) % other.scaled_to(decimals),
            decimals,
        }
    }

    pub fn abs(&self) -> Precise {
        Precise {
            integer: self.integer.abs(),
            decimals: self.decimals,
        }
    }

    pub fn neg(&self) -> Precise {
        Precise {
            integer: -&self.integer,
            decimals: self.decimals,
        }
    }

    /// 比较(对齐小数位后比较尾数)。
    pub fn compare(&self, other: &Precise) -> Ordering {
        let decimals = self.decimals.max(other.decimals);
        self.scaled_to(decimals).cmp(&other.scaled_to(decimals))
    }

    pub fn gt(&self, other: &Precise) -> bool {
        self.compare(other) == Ordering::Greater
    }
    pub fn ge(&self, other: &Precise) -> bool {
        self.compare(other) != Ordering::Less
    }
    pub fn lt(&self, other: &Precise) -> bool {
        self.compare(other) == Ordering::Less
    }
    pub fn le(&self, other: &Precise) -> bool {
        self.compare(other) != Ordering::Greater
    }
    /// 数值相等(如 `0.1 == 0.10`)。
    pub fn equals(&self, other: &Precise) -> bool {
        self.compare(other) == Ordering::Equal
    }

    pub fn min(&self, other: &Precise) -> Precise {
        if self.le(other) {
            self.clone()
        } else {
            other.clone()
        }
    }

    pub fn max(&self, other: &Precise) -> Precise {
        if self.ge(other) {
            self.clone()
        } else {
            other.clone()
        }
    }

    /// 去尾随零(`1.2300` -> `1.23`)。
    pub fn reduce(&self) -> Precise {
        let mut integer = self.integer.clone();
        let mut decimals = self.decimals;
        while decimals > 0 && (&integer % 10u8).is_zero() {
            integer /= 10u8;
            decimals -= 1;
        }
        Precise { integer, decimals }
    }

    /// 十进制字符串输出,保留小数位(与 ccxt `toString` 一致)。
    fn format(&self) -> String {
        if self.integer.is_zero() {
            return "0".to_string();
        }
        if self.decimals == 0 {
            return self.integer.to_string();
        }
        let sign = if self.integer.is_negative() { "-" } else { "" };
        let abs = self.integer.abs().to_string();
        // 补零到至少 decimals+1 位
        let padded = format!("{abs:0>width$}", width = self.decimals as usize + 1);
        let split = padded.len() - self.decimals as usize;
        format!("{sign}{}.{}", &padded[..split], &padded[split..])
    }
}

impl fmt::Display for Precise {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

impl FromStr for Precise {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// 静态字符串助手(ccxt `stringAdd` 等):直接对字符串做运算并输出字符串。
pub fn string_add(a: &str, b: &str) -> String {
    let pa = a.parse::<Precise>().expect("invalid a");
    let pb = b.parse::<Precise>().expect("invalid b");
    pa.add(&pb).to_string()
}

pub fn string_sub(a: &str, b: &str) -> String {
    let pa = a.parse::<Precise>().expect("invalid a");
    let pb = b.parse::<Precise>().expect("invalid b");
    pa.sub(&pb).to_string()
}

pub fn string_mul(a: &str, b: &str) -> String {
    let pa = a.parse::<Precise>().expect("invalid a");
    let pb = b.parse::<Precise>().expect("invalid b");
    pa.mul(&pb).to_string()
}

pub fn string_div(a: &str, b: &str) -> String {
    let pa = a.parse::<Precise>().expect("invalid a");
    let pb = b.parse::<Precise>().expect("invalid b");
    pa.div(&pb, DIV_PRECISION).to_string()
}

pub fn string_mod(a: &str, b: &str) -> String {
    let pa = a.parse::<Precise>().expect("invalid a");
    let pb = b.parse::<Precise>().expect("invalid b");
    pa.rem(&pb).to_string()
}

pub fn string_abs(a: &str) -> String {
    a.parse::<Precise>().expect("invalid a").abs().to_string()
}

/// `1 - d`(预测市场 NO 侧报价反转)。
pub fn one_minus_decimal(d: rust_decimal::Decimal) -> rust_decimal::Decimal {
    rust_decimal::Decimal::ONE - d
}

pub fn string_neg(a: &str) -> String {
    a.parse::<Precise>().expect("invalid a").neg().to_string()
}

pub fn string_gt(a: &str, b: &str) -> bool {
    a.parse::<Precise>()
        .expect("invalid a")
        .gt(&b.parse::<Precise>().expect("invalid b"))
}

pub fn string_ge(a: &str, b: &str) -> bool {
    a.parse::<Precise>()
        .expect("invalid a")
        .ge(&b.parse::<Precise>().expect("invalid b"))
}

pub fn string_lt(a: &str, b: &str) -> bool {
    a.parse::<Precise>()
        .expect("invalid a")
        .lt(&b.parse::<Precise>().expect("invalid b"))
}

pub fn string_le(a: &str, b: &str) -> bool {
    a.parse::<Precise>()
        .expect("invalid a")
        .le(&b.parse::<Precise>().expect("invalid b"))
}

pub fn string_equals(a: &str, b: &str) -> bool {
    a.parse::<Precise>()
        .expect("invalid a")
        .equals(&b.parse::<Precise>().expect("invalid b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Precise {
        s.parse().unwrap()
    }

    #[test]
    fn parse_basic() {
        assert_eq!(p("0.1").to_string(), "0.1");
        assert_eq!(p("1.2300").to_string(), "1.2300");
        assert_eq!(p("-1.5").to_string(), "-1.5");
        assert_eq!(p(".5").to_string(), "0.5");
        assert_eq!(p("-0.000").to_string(), "0");
        assert_eq!(p("123").to_string(), "123");
    }

    #[test]
    fn parse_scientific() {
        assert_eq!(p("1e-5").to_string(), "0.00001");
        assert_eq!(p("1e5").to_string(), "100000");
        assert_eq!(p("1.5e2").to_string(), "150");
        assert_eq!(p("1.5e-2").to_string(), "0.015");
    }

    #[test]
    fn arithmetic_ccxt_examples() {
        assert_eq!(p("0.1").add(&p("0.2")).to_string(), "0.3");
        assert_eq!(p("0.8").sub(&p("0.2")).to_string(), "0.6");
        assert_eq!(p("0.9").mul(&p("0.2")).to_string(), "0.18");
        let q = p("0.9").div(&p("0.2"), DIV_PRECISION);
        assert_eq!(q.to_string(), "4.500000000000000000");
        assert_eq!(q.reduce().to_string(), "4.5");
    }

    #[test]
    fn rem_abs_neg() {
        assert_eq!(p("5.5").rem(&p("2")).to_string(), "1.5");
        assert_eq!(p("-1.5").abs().to_string(), "1.5");
        assert_eq!(p("1.5").neg().to_string(), "-1.5");
        assert_eq!(p("0").neg().to_string(), "0");
    }

    #[test]
    fn compare_and_equals() {
        assert!(p("0.1").equals(&p("0.10")));
        assert!(p("0.2").gt(&p("0.1")));
        assert!(p("0.1").lt(&p("0.2")));
        assert!(p("1.0").ge(&p("1")));
        assert!(p("1.0").le(&p("1")));
        assert_eq!(p("1.5").min(&p("2.5")).to_string(), "1.5");
        assert_eq!(p("1.5").max(&p("2.5")).to_string(), "2.5");
    }

    #[test]
    fn reduce_strips_trailing_zeros() {
        assert_eq!(p("1.2300").reduce().to_string(), "1.23");
        assert_eq!(p("100.000").reduce().to_string(), "100");
        assert_eq!(p("0").reduce().to_string(), "0");
    }

    #[test]
    fn string_helpers() {
        assert_eq!(string_add("0.1", "0.2"), "0.3");
        assert_eq!(string_sub("0.8", "0.2"), "0.6");
        assert_eq!(string_mul("0.9", "0.2"), "0.18");
        assert_eq!(string_mod("5.5", "2"), "1.5");
        assert!(string_ge("0.1", "0.10"));
        assert!(string_equals("0.1", "0.10"));
    }
}
