//! Bounded expression evaluation; no scripts, file access or network operations.
pub mod exchange;
mod matrix;
mod matrix_ui;
mod parser;
mod plot;
#[cfg(feature = "ui-preview")]
mod plot_preview;
mod plot_render;
mod plot_ui;
mod ui;
mod worksheet;
mod worksheet_ui;
pub use ui::State;

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Exact(i128, i128),
    Approx(f64),
}

impl Value {
    fn exact(n: i128, d: i128) -> Result<Self, String> {
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
    fn approximate(x: f64) -> Result<Self, String> {
        if x.is_finite() {
            Ok(Self::Approx(x))
        } else {
            Err("结果不是有限实数；检查函数定义域或数值范围".into())
        }
    }
    fn float(self) -> f64 {
        match self {
            Self::Exact(n, d) => n as f64 / d as f64,
            Self::Approx(x) => x,
        }
    }
    pub fn integer(self) -> Result<i128, String> {
        match self {
            Self::Exact(n, 1) => Ok(n),
            _ => Err("此操作需要精确整数".into()),
        }
    }
    fn neg(self) -> Result<Self, String> {
        match self {
            Self::Exact(n, d) => Self::exact(n.checked_neg().ok_or("数值溢出")?, d),
            Self::Approx(x) => Self::approximate(-x),
        }
    }
    fn binary(self, op: &str, other: Self) -> Result<Self, String> {
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
            return Self::approximate(self.float().powf(other.float()));
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

#[derive(Clone, Copy)]
pub enum Angle {
    Radians,
    Degrees,
}

pub fn evaluate(
    expression: &str,
    variables: &BTreeMap<String, Value>,
    angle: Angle,
) -> Result<Value, String> {
    parser::evaluate(expression, variables, angle)
}

pub fn valid_variable(name: &str) -> bool {
    !matches!(name, "pi" | "e" | "ans")
        && (1..=32).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(s: &str) -> Value {
        evaluate(s, &BTreeMap::new(), Angle::Radians).unwrap()
    }
    #[test]
    fn exact_decimal_fraction_precedence_and_programmer_operations() {
        for (s, want) in [
            ("0.1+0.2", "0.3"),
            ("1/3+1/6", "0.5"),
            ("-2^2", "-4"),
            ("2^3^2", "512"),
            ("2^-3", "0.125"),
            ("(128+64)*3", "576"),
            ("0xff & 0x0f", "15"),
            ("0b1010 << 2", "40"),
            ("~0", "-1"),
            ("1e-3+0.002", "0.003"),
            ("1/3", "1/3"),
            ("5 km -> m", "5000"),
            ("1 h -> min", "60"),
            ("25 C -> F", "77"),
            ("32 F -> C", "0"),
            (
                "min(100000000000000000000,100000000000000000001)",
                "100000000000000000000",
            ),
            ("round(-1.5)", "-2"),
            ("floor(-1/3)", "-1"),
            ("1 GiB -> MB", "1073.741824"),
        ] {
            assert_eq!(eval(s).display(), want, "{s}");
        }
    }
    #[test]
    fn domains_limits_units_and_non_code_execution() {
        for s in [
            "1/0",
            "sqrt(-1)",
            "ln(0)",
            "2^129",
            "1<<128",
            "5 kg -> m",
            "5 C -> m",
            "999999999999999999999999999999999999999999999999",
            "std::fs::remove_file(1)",
            "1;2",
            "1 2",
            "sin(1,2)",
            "2+",
            "()",
            "1e99999",
        ] {
            assert!(
                evaluate(s, &BTreeMap::new(), Angle::Radians).is_err(),
                "{s}"
            );
        }
        assert!(evaluate(&"(".repeat(1000), &BTreeMap::new(), Angle::Radians).is_err());
        assert!(evaluate(&"1+".repeat(2000), &BTreeMap::new(), Angle::Radians).is_err());
    }
    #[test]
    fn angle_mode_variables_and_exactness_are_explicit() {
        let mut vars = BTreeMap::new();
        vars.insert("price".into(), eval("19.90"));
        assert_eq!(
            evaluate("price*3", &vars, Angle::Degrees)
                .unwrap()
                .display(),
            "59.7"
        );
        let v = evaluate("sin(30)", &vars, Angle::Degrees).unwrap();
        assert!((v.float() - 0.5).abs() < 1e-12);
        assert!(matches!(v, Value::Approx(_)));
        assert!(
            evaluate("missing+1", &vars, Angle::Radians)
                .unwrap_err()
                .contains("missing")
        );
        vars.insert("invalid".into(), Value::Exact(1, 0));
        assert!(evaluate("invalid+1", &vars, Angle::Radians).is_err());
        assert!(eval("exp(-50)").display().contains("e-"));
    }
}
