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
pub fn compare(left: Value, right: Value) -> Ordering {
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
    for value in args {
        value.real()?;
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
pub fn evaluate(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "fact" | "perm" | "comb" | "gcd" | "lcm" => Some(combinatorics(name, args)),
        "sum" | "mean" | "median" | "varp" | "vars" | "stdp" | "stds" => {
            Some(statistics(name, args))
        }
        _ => None,
    }
}
