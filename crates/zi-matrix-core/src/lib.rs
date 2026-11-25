//! Shared numeric values and bounded real matrix arithmetic.
use serde::{Deserialize, Serialize};

pub mod matrix;
pub mod statistics;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Exact(i128, i128),
    Approx(f64),
    Complex(f64, f64),
}

impl Value {
    pub fn exact(n: i128, d: i128) -> Result<Self, String> {
        if d == 0 {
            return Err("不能除以零".into());
        }
        let mut a = n.unsigned_abs();
        let mut b = d.unsigned_abs();
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let gcd = i128::try_from(a).map_err(|_| "数值超出精确计算范围")?;
        let n = n.checked_div(gcd).ok_or("数值超出精确计算范围")?;
        let d = d.checked_div(gcd).ok_or("数值超出精确计算范围")?;
        if d < 0 {
            Ok(Self::Exact(
                n.checked_neg().ok_or("数值溢出")?,
                d.checked_neg().ok_or("数值溢出")?,
            ))
        } else {
            Ok(Self::Exact(n, d))
        }
    }
    pub fn approximate(x: f64) -> Result<Self, String> {
        if x.is_finite() {
            Ok(Self::Approx(x))
        } else {
            Err("结果不是有限实数；检查函数定义域或数值范围".into())
        }
    }
    pub fn complex(real: f64, imaginary: f64) -> Result<Self, String> {
        if real.is_finite() && imaginary.is_finite() {
            Ok(Self::Complex(real, imaginary))
        } else {
            Err("结果不是有限复数；检查函数定义域或数值范围".into())
        }
    }
    pub fn parts(self) -> (f64, f64) {
        match self {
            Self::Exact(n, d) => (n as f64 / d as f64, 0.0),
            Self::Approx(x) => (x, 0.0),
            Self::Complex(real, imaginary) => (real, imaginary),
        }
    }
    pub fn is_approximate(self) -> bool {
        !matches!(self, Self::Exact(_, _))
    }
    pub fn real(self) -> Result<f64, String> {
        let (real, imaginary) = self.parts();
        if imaginary == 0.0 {
            Ok(real)
        } else {
            Err("此功能仅支持实数；请先使用 real(z) 提取实部".into())
        }
    }
    pub fn float(self) -> f64 {
        self.real().unwrap_or(f64::NAN)
    }
    pub fn integer(self) -> Result<i128, String> {
        match self {
            Self::Exact(n, 1) => Ok(n),
            _ => Err("此操作需要精确整数".into()),
        }
    }
    pub fn checked_neg(self) -> Result<Self, String> {
        match self {
            Self::Exact(n, d) => Self::exact(n.checked_neg().ok_or("数值溢出")?, d),
            Self::Approx(x) => Self::approximate(-x),
            Self::Complex(real, imaginary) => Self::complex(-real, -imaginary),
        }
    }
    pub fn binary(self, op: &str, other: Self) -> Result<Self, String> {
        let overflow = || "数值超出 i128 精确范围；不会自动改为近似值".to_owned();
        if matches!(op, "&" | "|" | "<<" | ">>" | "%") {
            let (a, b) = (self.integer()?, other.integer()?);
            return Ok(Self::Exact(
                match op {
                    "&" => a & b,
                    "|" => a | b,
                    "<<" | ">>" => {
                        let shift = u32::try_from(b)
                            .ok()
                            .filter(|b| *b < 128)
                            .ok_or("位移需为 0–127")?;
                        // Bit operations use a signed, fixed 128-bit two's-complement word.
                        if op == "<<" {
                            a.wrapping_shl(shift)
                        } else {
                            a >> shift
                        }
                    }
                    _ => a.checked_rem(b).ok_or("整数取余错误：零除或溢出")?,
                },
                1,
            ));
        }
        if op == "^" {
            if let (Self::Exact(a, d), Self::Exact(e, 1)) = (self, other) {
                let power = u32::try_from(e.unsigned_abs())
                    .ok()
                    .filter(|v| *v <= 128)
                    .ok_or("精确整数指数范围为 -128–128")?;
                let (n, d) = (
                    a.checked_pow(power).ok_or_else(overflow)?,
                    d.checked_pow(power).ok_or_else(overflow)?,
                );
                return if e < 0 {
                    Self::exact(d, n)
                } else {
                    Self::exact(n, d)
                };
            }
            if matches!(self, Self::Complex(_, _))
                || matches!(other, Self::Complex(_, _))
                || (self.float() < 0.0 && other.float().fract() != 0.0)
            {
                return complex_binary(self, op, other);
            }
            return Self::approximate(self.float().powf(other.float()));
        }
        if matches!(self, Self::Complex(_, _)) || matches!(other, Self::Complex(_, _)) {
            return complex_binary(self, op, other);
        }
        if let (Self::Exact(a, b), Self::Exact(c, d)) = (self, other) {
            // Cancel common factors before multiplying to preserve the exact range.
            let gcd = |x: i128, y: i128| {
                let mut x = x.unsigned_abs();
                let mut y = y.unsigned_abs();
                while y != 0 {
                    (x, y) = (y, x % y);
                }
                i128::try_from(x).unwrap_or(1)
            };
            return match op {
                "+" | "-" => {
                    let g = gcd(b, d);
                    let left = a.checked_mul(d / g).ok_or_else(overflow)?;
                    let right = c.checked_mul(b / g).ok_or_else(overflow)?;
                    Self::exact(
                        if op == "+" {
                            left.checked_add(right)
                        } else {
                            left.checked_sub(right)
                        }
                        .ok_or_else(overflow)?,
                        (b / g).checked_mul(d).ok_or_else(overflow)?,
                    )
                }
                "*" => {
                    let g = gcd(a, d);
                    let h = gcd(c, b);
                    Self::exact(
                        (a / g).checked_mul(c / h).ok_or_else(overflow)?,
                        (b / h).checked_mul(d / g).ok_or_else(overflow)?,
                    )
                }
                "/" => {
                    let g = gcd(a, c).max(1);
                    let h = gcd(b, d);
                    Self::exact(
                        (a / g).checked_mul(d / h).ok_or_else(overflow)?,
                        (b / h).checked_mul(c / g).ok_or_else(overflow)?,
                    )
                }
                _ => Err("不支持的运算符".into()),
            };
        }
        let (a, b) = (self.float(), other.float());
        Self::approximate(match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            _ => return Err("不支持的运算符".into()),
        })
    }
    pub fn display(self) -> String {
        match self {
            Self::Approx(x) => {
                if x != 0.0 && (x.abs() < 1e-6 || x.abs() >= 1e12) {
                    format!("≈ {x:.12e}")
                } else {
                    format!("≈ {x:.12}")
                }
            }
            Self::Complex(real, imaginary) => {
                let component = |value: f64| {
                    if value != 0.0 && (value.abs() < 1e-6 || value.abs() >= 1e12) {
                        format!("{value:.8e}")
                    } else {
                        format!("{value:.10}")
                            .trim_end_matches('0')
                            .trim_end_matches('.')
                            .to_owned()
                    }
                };
                if imaginary == 0.0 {
                    format!("≈ {}", component(real))
                } else {
                    let imag = match imaginary.abs() {
                        1.0 => "i".to_owned(),
                        n => format!("{}i", component(n)),
                    };
                    let head = if real == 0.0 {
                        if imaginary < 0.0 {
                            format!("−{imag}")
                        } else {
                            imag.clone()
                        }
                    } else {
                        format!(
                            "{}{}",
                            component(real),
                            if imaginary < 0.0 { "−" } else { "+" }
                        )
                    };
                    if real == 0.0 {
                        format!("≈ {head}")
                    } else {
                        format!("≈ {head}{imag}")
                    }
                }
            }
            Self::Exact(n, 1) => n.to_string(),
            Self::Exact(n, d) => {
                let mut remaining = d;
                let mut twos = 0;
                let mut fives = 0;
                while remaining % 2 == 0 {
                    remaining /= 2;
                    twos += 1;
                }
                while remaining % 5 == 0 {
                    remaining /= 5;
                    fives += 1;
                }
                let digits = twos.max(fives);
                if remaining == 1 && digits <= 38 {
                    let mut s = format!(
                        "{}{}",
                        if n < 0 { "-" } else { "" },
                        n.unsigned_abs() / d as u128
                    );
                    s.push('.');
                    let mut remainder = n.unsigned_abs() % d as u128;
                    for _ in 0..digits {
                        // Multiplication may overflow for huge denominators: retain fraction.
                        let Some(next) = remainder.checked_mul(10) else {
                            return format!("{n}/{d}");
                        };
                        s.push(char::from(b'0' + (next / d as u128) as u8));
                        remainder = next % d as u128;
                    }
                    s
                } else {
                    format!("{n}/{d}")
                }
            }
        }
    }
}

fn complex_binary(left: Value, op: &str, right: Value) -> Result<Value, String> {
    let ((a, b), (c, d)) = (left.parts(), right.parts());
    let result = match op {
        "+" => (a + c, b + d),
        "-" => (a - c, b - d),
        "*" => (a * c - b * d, a * d + b * c),
        "/" => {
            if c == 0.0 && d == 0.0 {
                return Err("不能除以零".into());
            }
            // Smith's scaled division avoids overflowing c²+d² for large operands.
            if c.abs() >= d.abs() {
                let ratio = d / c;
                let denominator = 1.0 + ratio * ratio;
                (
                    (a / c + (b / c) * ratio) / denominator,
                    (b / c - (a / c) * ratio) / denominator,
                )
            } else {
                let ratio = c / d;
                let denominator = 1.0 + ratio * ratio;
                (
                    ((a / d) * ratio + b / d) / denominator,
                    ((b / d) * ratio - a / d) / denominator,
                )
            }
        }
        "^" => {
            let magnitude = a.hypot(b);
            if magnitude == 0.0 {
                if c == 0.0 && d == 0.0 {
                    return Err("零的零次复数幂未定义".into());
                }
                if c > 0.0 && d == 0.0 {
                    return Value::complex(0.0, 0.0);
                }
                return Err("零的非正实数或复数幂未定义".into());
            }
            let log_magnitude = magnitude.ln();
            let phase = b.atan2(a);
            let real = c * log_magnitude - d * phase;
            let imaginary = c * phase + d * log_magnitude;
            let scale = real.exp();
            (scale * imaginary.cos(), scale * imaginary.sin())
        }
        _ => return Err("复数仅支持 +、−、×、÷ 和幂运算".into()),
    };
    Value::complex(result.0, result.1)
}

pub fn complex_sqrt(real: f64, imaginary: f64) -> (f64, f64) {
    if imaginary == 0.0 {
        if real < 0.0 {
            return (0.0, (-real).sqrt());
        }
        return (real.sqrt(), 0.0);
    }
    let magnitude = real.hypot(imaginary);
    if real >= 0.0 {
        let root = magnitude.sqrt() * ((1.0 + real / magnitude) / 2.0).sqrt();
        (root, imaginary / (2.0 * root))
    } else {
        let root = magnitude.sqrt() * ((1.0 - real / magnitude) / 2.0).sqrt();
        (imaginary.abs() / (2.0 * root), imaginary.signum() * root)
    }
}

/// Parses one real literal for a bounded tool input without interpreting a formula.
pub fn parse_real_literal(text: &str, approximate: bool) -> Result<Value, String> {
    if approximate {
        let number = text.parse().map_err(|_| "需要有限小数或科学计数法")?;
        return Value::approximate(number);
    }
    if let Some((numerator, denominator)) = text.split_once('/') {
        let n = numerator.trim().parse().map_err(|_| "分子需为 i128 整数")?;
        let d = denominator
            .trim()
            .parse()
            .map_err(|_| "分母需为 i128 整数")?;
        Value::exact(n, d)
    } else {
        exact_number(text)
    }
}

pub fn exact_number(text: &str) -> Result<Value, String> {
    if let Some((digits, radix)) = text
        .strip_prefix("0x")
        .map(|s| (s, 16))
        .or_else(|| text.strip_prefix("0b").map(|s| (s, 2)))
        .or_else(|| text.strip_prefix("0o").map(|s| (s, 8)))
    {
        return i128::from_str_radix(digits, radix)
            .map(|v| Value::Exact(v, 1))
            .map_err(|_| "进制整数无效或溢出".into());
    }
    let (mantissa, exponent) = if let Some((m, e)) = text.split_once(['e', 'E']) {
        (m, e.parse::<i32>().map_err(|_| "科学计数法指数无效")?)
    } else {
        (text, 0)
    };
    let (digits, fraction) = if let Some((a, b)) = mantissa.split_once('.') {
        (format!("{a}{b}"), b.len() as i32)
    } else {
        (mantissa.to_owned(), 0)
    };
    let n = digits
        .parse::<i128>()
        .map_err(|_| "数字无效或超出精确范围")?;
    let scale = fraction.checked_sub(exponent).ok_or("小数位数溢出")?;
    if scale.unsigned_abs() > 38 {
        return Err("小数/科学计数法精确位数最多 38".into());
    }
    let factor = 10i128.checked_pow(scale.unsigned_abs()).ok_or("数字溢出")?;
    if scale >= 0 {
        Value::exact(n, factor)
    } else {
        Value::exact(n.checked_mul(factor).ok_or("数字溢出")?, 1)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Number {
    Exact {
        numerator: String,
        denominator: String,
    },
    Approx {
        value: String,
    },
    Complex {
        real: String,
        imaginary: String,
    },
}
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        validate_value(*self).map_err(serde::ser::Error::custom)?;
        match self {
            Self::Exact(n, d) => Number::Exact {
                numerator: n.to_string(),
                denominator: d.to_string(),
            },
            Self::Approx(value) => Number::Approx {
                value: format!("{value:e}"),
            },
            Self::Complex(real, imaginary) => Number::Complex {
                real: format!("{real:e}"),
                imaginary: format!("{imaginary:e}"),
            },
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let parse = |s: String| -> Result<i128, String> {
            if s.len() > 40 {
                return Err("精确整数文本过长".into());
            }
            let n = s.parse::<i128>().map_err(|_| "精确整数超范围")?;
            if n.to_string() != s {
                return Err("精确整数必须是规范十进制字符串".into());
            }
            Ok(n)
        };
        let value = match Number::deserialize(deserializer)? {
            Number::Exact {
                numerator,
                denominator,
            } => Self::Exact(
                parse(numerator).map_err(serde::de::Error::custom)?,
                parse(denominator).map_err(serde::de::Error::custom)?,
            ),
            Number::Approx { value } => {
                if value.len() > 64 {
                    return Err(serde::de::Error::custom("近似数值文本过长"));
                }
                Self::Approx(value.parse::<f64>().map_err(serde::de::Error::custom)?)
            }
            Number::Complex { real, imaginary } => {
                if real.len() > 64 || imaginary.len() > 64 {
                    return Err(serde::de::Error::custom("复数分量文本过长"));
                }
                let parse = |text: String| -> Result<f64, String> {
                    let value = text.parse::<f64>().map_err(|_| "复数分量无效")?;
                    if !value.is_finite() || format!("{value:e}") != text {
                        return Err("复数分量必须是有限规范科学计数法".into());
                    }
                    Ok(value)
                };
                Self::Complex(
                    parse(real).map_err(serde::de::Error::custom)?,
                    parse(imaginary).map_err(serde::de::Error::custom)?,
                )
            }
        };
        validate_value(value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
pub fn validate_value(value: Value) -> Result<(), String> {
    match value {
        Value::Exact(n, d) if d > 0 && Value::exact(n, d)? == value => Ok(()),
        Value::Approx(x) if x.is_finite() => Ok(()),
        Value::Complex(real, imaginary) if real.is_finite() && imaginary.is_finite() => Ok(()),
        _ => Err("工作表数值必须是约分后的正分母有理数或有限近似值".into()),
    }
}
