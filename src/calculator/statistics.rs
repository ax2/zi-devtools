//! Bounded statistics and combinatorics using the calculator's existing values.
use super::Value;
use std::cmp::Ordering;

fn positive_compare(mut a: u128, mut b: u128, mut c: u128, mut d: u128) -> Ordering {
    let mut reverse = false;
    loop {
        let order = (a / b).cmp(&(c / d));
        let (r, s) = (a % b, c % d);
        let order = if !order.is_eq() {
            order
        } else if r == 0 || s == 0 {
            r.cmp(&s)
        } else {
            (a, b, c, d) = (b, r, d, s);
            reverse = !reverse;
            continue;
        };
        return if reverse { order.reverse() } else { order };
    }
}
pub(super) fn compare(left: Value, right: Value) -> Ordering {
    match (left, right) {
        (Value::Exact(a, b), Value::Exact(c, d)) => {
            if (a < 0) != (c < 0) {
                return a.cmp(&c);
            }
            let order = positive_compare(a.unsigned_abs(), b as u128, c.unsigned_abs(), d as u128);
            if a < 0 { order.reverse() } else { order }
        }
        _ => left
            .float()
            .partial_cmp(&right.float())
            .unwrap_or(Ordering::Equal),
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn exact_unsigned(n: u128) -> Result<Value, String> {
    Ok(Value::Exact(
        i128::try_from(n).map_err(|_| "结果超出 i128 精确范围")?,
        1,
    ))
}
fn combinatorics(name: &str, args: &[Value]) -> Result<Value, String> {
    let count = if name == "fact" { 1 } else { 2 };
    if args.len() != count {
        return Err(format!("{name} 需要 {count} 个精确整数参数"));
    }
    if matches!(name, "gcd" | "lcm") {
        let a = args[0].integer()?.unsigned_abs();
        let b = args[1].integer()?.unsigned_abs();
        return exact_unsigned(if name == "gcd" {
            gcd(a, b)
        } else if a == 0 || b == 0 {
            0
        } else {
            (a / gcd(a, b))
                .checked_mul(b)
                .ok_or("最小公倍数超出精确范围")?
        });
    }
    let n = args[0].integer()?;
    let r = if name == "fact" {
        n
    } else {
        args[1].integer()?
    };
    if !(0..=10000).contains(&n) || !(0..=n).contains(&r) {
        return Err("阶乘/排列组合要求 0≤r≤n≤10000 的精确整数".into());
    }
    let (n, r) = (n as u128, r as u128);
    let r = if name == "comb" { r.min(n - r) } else { r };
    let mut result = 1u128;
    for i in 1..=r {
        let mut coefficient = n - r + i;
        if name == "comb" {
            // Cancel the denominator on both factors before multiplying.
            let g = gcd(coefficient, i);
            coefficient /= g;
            let divisor = i / g;
            debug_assert_eq!(result % divisor, 0);
            result /= divisor;
        }
        result = result
            .checked_mul(coefficient)
            .filter(|v| *v <= i128::MAX as u128)
            .ok_or("阶乘/排列组合结果超出 i128 精确范围")?;
    }
    exact_unsigned(result)
}
fn statistics(name: &str, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() || args.len() > 64 {
        return Err(format!("{name} 需要 1–64 个数值"));
    }
    let sample = matches!(name, "vars" | "stds");
    if sample && args.len() < 2 {
        return Err("样本方差/标准差至少需要 2 个值；分母为 n−1".into());
    }
    let approximate = args.iter().any(|v| matches!(v, Value::Approx(_)));
    let result = if name == "sum" {
        args.iter()
            .try_fold(Value::Exact(0, 1), |total, value| total.binary("+", *value))?
    } else if name == "median" {
        let mut sorted: Vec<Value> = args
            .iter()
            .map(|v| {
                if approximate {
                    Value::Approx(v.float())
                } else {
                    *v
                }
            })
            .collect();
        sorted.sort_by(|a, b| compare(*a, *b));
        let middle = sorted.len() / 2;
        if sorted.len() % 2 == 1 {
            sorted[middle]
        } else {
            // Center the pair, avoiding overflow for identical large values.
            sorted[middle - 1].binary(
                "+",
                sorted[middle]
                    .binary("-", sorted[middle - 1])?
                    .binary("/", Value::Exact(2, 1))?,
            )?
        }
    } else {
        let base = args[0];
        let offsets: Vec<Value> = args
            .iter()
            .map(|v| v.binary("-", base))
            .collect::<Result<_, _>>()?;
        let mean_offset = offsets
            .iter()
            .try_fold(Value::Exact(0, 1), |total, v| total.binary("+", *v))?
            .binary("/", Value::Exact(args.len() as i128, 1))?;
        if name == "mean" {
            base.binary("+", mean_offset)?
        } else {
            let squared = offsets.iter().try_fold(Value::Exact(0, 1), |total, v| {
                let centered = v.binary("-", mean_offset)?;
                total.binary("+", centered.binary("*", centered)?)
            })?;
            let variance = squared.binary(
                "/",
                Value::Exact((args.len() - usize::from(sample)) as i128, 1),
            )?;
            if matches!(name, "stdp" | "stds") {
                Value::approximate(variance.float().sqrt())?
            } else {
                variance
            }
        }
    };
    if approximate {
        Value::approximate(result.float())
    } else {
        Ok(result)
    }
}
pub(super) fn evaluate(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "fact" | "perm" | "comb" | "gcd" | "lcm" => Some(combinatorics(name, args)),
        "sum" | "mean" | "median" | "varp" | "vars" | "stdp" | "stds" => {
            Some(statistics(name, args))
        }
        _ => None,
    }
}

#[cfg(feature = "ui-preview")]
impl super::State {
    pub fn preview_statistics_fixture(&mut self) {
        *self = Default::default();
        self.expression = "variance = vars(1,2,3)".into();
        self.variables.insert("ans".into(), Value::Exact(7, 1));
        self.examples_expanded = true;
    }
    pub fn preview_statistics_check(&self, phase: u8) {
        assert!(
            self.variables.contains_key("variance"),
            "phase={phase} expression={} message={} history={:?} preview={:?}",
            self.expression,
            self.message,
            self.history,
            self.preview()
        );
        assert_eq!(self.variables["variance"], Value::Exact(1, 1));
        assert_eq!(
            self.variables["ans"],
            Value::Exact(if phase == 3 { 2598960 } else { 1 }, 1)
        );
        assert_eq!(self.history.len(), if phase == 3 { 2 } else { 1 });
        if phase == 1 {
            assert_eq!(self.expression, "vars(1)");
            assert!(self.preview().is_err());
        }
        if phase >= 2 {
            assert_eq!(self.expression, "comb(52,5)");
            assert_eq!(self.preview().unwrap().1, Value::Exact(2598960, 1));
        }
    }
    pub fn preview_statistics_position(&self) -> eframe::egui::Pos2 {
        let (rect, clip) = self
            .preview_statistics_button
            .expect("combination example rendered");
        assert!(
            clip.contains(rect.center()),
            "statistics sample clipped: {rect:?} {clip:?}"
        );
        rect.center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calculator::{Angle, evaluate as expression};
    use std::collections::BTreeMap;
    fn eval(s: &str) -> Result<Value, String> {
        expression(s, &BTreeMap::new(), Angle::Radians)
    }
    #[test]
    fn exact_statistics_and_sample_population_distinction() {
        for (input, expected) in [
            ("sum(0.1,0.2,0.3)", Value::Exact(3, 5)),
            ("mean(1/3,2/3)", Value::Exact(1, 2)),
            ("median(5,1,2,4)", Value::Exact(3, 1)),
            ("varp(1,2,3)", Value::Exact(2, 3)),
            ("vars(1,2,3)", Value::Exact(1, 1)),
            ("varp(17)", Value::Exact(0, 1)),
            ("median(-1/3,-1/2,1)", Value::Exact(-1, 3)),
        ] {
            assert_eq!(eval(input).unwrap(), expected, "{input}");
        }
        assert!(matches!(eval("stds(1,2,3)").unwrap(), Value::Approx(v) if v == 1.0));
        let max = i128::MAX;
        assert_eq!(
            eval(&format!("mean({max},{max})")).unwrap(),
            Value::Exact(max, 1)
        );
        assert_eq!(
            eval(&format!("median({max},{max})")).unwrap(),
            Value::Exact(max, 1)
        );
        assert_eq!(
            eval(&format!("vars({max},{max})")).unwrap(),
            Value::Exact(0, 1)
        );
        assert_eq!(
            eval("varp(1000000000000000,1000000000000001,1000000000000002)").unwrap(),
            Value::Exact(2, 3)
        );
    }
    #[test]
    fn combinatorics_cancels_before_multiplying_and_rejects_overflow() {
        for (input, expected) in [
            ("fact(0)", 1),
            ("fact(20)", 2432902008176640000),
            ("perm(5,3)", 60),
            ("comb(5,3)", 10),
            ("comb(10000,0)", 1),
            ("comb(127,63)", 11975573020964041433067793888190275875),
            ("gcd(-48,18)", 6),
            ("lcm(-12,18)", 36),
            ("gcd(0,0)", 0),
            ("lcm(0,8)", 0),
        ] {
            assert_eq!(eval(input).unwrap(), Value::Exact(expected, 1), "{input}");
        }
        for input in [
            "fact(34)",
            "fact(-1)",
            "fact(10001)",
            "comb(5,6)",
            "comb(200,100)",
            "perm(100,40)",
            "gcd(1/2,2)",
            "lcm(170141183460469231731687303715884105727,2)",
        ] {
            assert!(eval(input).is_err(), "{input}");
        }
        assert_eq!(
            evaluate("gcd", &[Value::Exact(i128::MIN, 1), Value::Exact(2, 1)])
                .unwrap()
                .unwrap(),
            Value::Exact(2, 1)
        );
        assert!(
            evaluate("gcd", &[Value::Exact(i128::MIN, 1), Value::Exact(0, 1)])
                .unwrap()
                .is_err()
        );
    }
    #[test]
    fn exact_order_avoids_cross_products_and_matches_small_rationals() {
        let max = i128::MAX;
        assert_eq!(
            compare(Value::Exact(max, max - 1), Value::Exact(max - 1, max - 2)),
            Ordering::Less
        );
        for a in -12i128..12 {
            for b in 1i128..12 {
                for c in -12i128..12 {
                    for d in 1i128..12 {
                        assert_eq!(
                            compare(Value::Exact(a, b), Value::Exact(c, d)),
                            (a * d).cmp(&(c * b))
                        );
                    }
                }
            }
        }
        assert_eq!(
            eval(&format!("min({max}/2,({max}-2)/3)")).unwrap(),
            Value::exact(max - 2, 3).unwrap()
        );
    }
    #[test]
    fn approximate_input_limits_and_variable_snapshots() {
        let vars = BTreeMap::from([
            ("x".into(), Value::Approx(1e12)),
            ("ans".into(), Value::Exact(7, 1)),
        ]);
        assert!(
            matches!(expression("varp(x,x+1,x+2)",&vars,Angle::Radians).unwrap(),Value::Approx(v) if (v-2.0/3.0).abs()<1e-12)
        );
        assert!(
            matches!(expression("median(1,2,x)",&vars,Angle::Radians).unwrap(),Value::Approx(v) if v==2.0)
        );
        assert_eq!(vars["ans"], Value::Exact(7, 1));
        let values = vec!["1"; 64].join(",");
        assert_eq!(
            eval(&format!("sum({values})")).unwrap(),
            Value::Exact(64, 1)
        );
        assert!(eval(&format!("sum({values},1)")).is_err());
        for input in [
            "sum()",
            "vars(1)",
            "stds(1)",
            "median()",
            "gcd(1)",
            "sin(1,2)",
            "mean(1;2)",
        ] {
            assert!(eval(input).is_err(), "{input}");
        }
    }
}
