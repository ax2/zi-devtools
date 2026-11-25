//! Real bisection over an explicitly supplied bracket.
use super::{Angle, Value, expression::Prepared};
use std::collections::BTreeMap;

const MAX_ITERATIONS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultValue {
    pub root: f64,
    pub residual: f64,
    pub iterations: usize,
    pub converged: bool,
}

pub struct Interval {
    pub lower: f64,
    pub upper: f64,
    pub tolerance: f64,
    pub iterations: usize,
}

pub fn solve(
    formula: &str,
    lower: f64,
    upper: f64,
    tolerance: f64,
    iterations: usize,
    variables: &BTreeMap<String, Value>,
    angle: Angle,
) -> Result<ResultValue, String> {
    solve_with_work_limit(
        formula,
        Interval {
            lower,
            upper,
            tolerance,
            iterations,
        },
        variables,
        angle,
        None,
    )
}

pub fn solve_with_work_limit(
    formula: &str,
    interval: Interval,
    variables: &BTreeMap<String, Value>,
    angle: Angle,
    mut remaining_work: Option<usize>,
) -> Result<ResultValue, String> {
    let Interval {
        lower,
        upper,
        tolerance,
        iterations,
    } = interval;
    if formula.trim().is_empty() || formula.len() > 2048 {
        return Err("函数公式需为1–2048字节".into());
    }
    if !lower.is_finite()
        || !upper.is_finite()
        || lower >= upper
        || lower.abs() > 1e12
        || upper.abs() > 1e12
    {
        return Err("区间端点必须有限、递增，且绝对值不超过 1e12".into());
    }
    if !tolerance.is_finite() || !(1e-12..=1e-2).contains(&tolerance) {
        return Err("区间宽度容差需在 1e-12 到 1e-2 之间".into());
    }
    if !(1..=MAX_ITERATIONS).contains(&iterations) {
        return Err(format!("迭代上限需在 1 到 {MAX_ITERATIONS} 之间"));
    }
    let prepared = Prepared::new(formula)?;
    let mut program = prepared.for_bisection(variables, angle, lower)?;
    let work = remaining_work.map(|_| program.work_weight());
    let mut evaluate = |x: f64| -> Result<f64, String> {
        if let (Some(remaining), Some(weight)) = (&mut remaining_work, work) {
            *remaining = remaining
                .checked_sub(weight)
                .ok_or("公式与区间的组合超出插件执行预算；请收窄区间、放宽宽度容差或简化公式")?;
        }
        program.evaluate(x)?.real()
    };

    let mut a = lower;
    let mut b = upper;
    let mut fa = evaluate(a)?;
    let mut fb = evaluate(b)?;
    if fa == 0.0 {
        return Ok(ResultValue {
            root: a,
            residual: 0.0,
            iterations: 0,
            converged: true,
        });
    }
    if fb == 0.0 {
        return Ok(ResultValue {
            root: b,
            residual: 0.0,
            iterations: 0,
            converged: true,
        });
    }
    if fa.is_sign_positive() == fb.is_sign_positive() {
        return Err("区间端点的函数值没有异号；请选择能夹住变号根的区间".into());
    }

    let mut midpoint = a;
    let mut fm = fa;
    for iteration in 1..=iterations {
        midpoint = a + (b - a) * 0.5;
        if midpoint == a || midpoint == b {
            return Ok(ResultValue {
                root: midpoint,
                residual: if midpoint == a { fa } else { fb },
                iterations: iteration - 1,
                converged: true,
            });
        }
        fm = evaluate(midpoint)?;
        if fm == 0.0 || (b - a).abs() <= tolerance {
            return Ok(ResultValue {
                root: midpoint,
                residual: fm,
                iterations: iteration,
                converged: true,
            });
        }
        if fa.is_sign_positive() != fm.is_sign_positive() {
            b = midpoint;
            fb = fm;
        } else {
            a = midpoint;
            fa = fm;
        }
    }
    Ok(ResultValue {
        root: midpoint,
        residual: fm,
        iterations,
        converged: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(formula: &str, lower: f64, upper: f64) -> Result<ResultValue, String> {
        solve(
            formula,
            lower,
            upper,
            1e-10,
            100,
            &BTreeMap::new(),
            Angle::Radians,
        )
    }

    #[test]
    fn adjacent_endpoint_keeps_residual_at_the_returned_point() {
        let formula = "(x-999999999999.9998)*10000-1/2";
        let variables = BTreeMap::new();
        let got = solve(
            formula,
            999999999999.9998,
            999999999999.9999,
            1e-12,
            100,
            &variables,
            Angle::Radians,
        )
        .unwrap();
        let at_point = BTreeMap::from([("x".into(), Value::Approx(got.root))]);
        let expected = Prepared::new(formula)
            .unwrap()
            .evaluate(&at_point, Angle::Radians)
            .unwrap()
            .real()
            .unwrap();
        assert!(got.converged);
        assert_eq!(got.residual.to_bits(), expected.to_bits());
    }

    #[test]
    fn additional_work_limit_does_not_apply_to_the_standalone_solver() {
        let interval = Interval {
            lower: 0.0,
            upper: 2.0,
            tolerance: 1e-10,
            iterations: 100,
        };
        assert!(
            solve_with_work_limit("x-1", interval, &BTreeMap::new(), Angle::Radians, Some(2))
                .unwrap_err()
                .contains("超出插件执行预算")
        );
        assert_eq!(run("x-1", 0.0, 2.0).unwrap().root, 1.0);
    }

    #[test]
    fn bisects_a_bracketed_root_and_reports_approximation() {
        let result = run("x^2-2", 1.0, 2.0).unwrap();
        assert!(result.converged);
        assert!((result.root - 2f64.sqrt()).abs() <= 1e-10);
        assert!(result.residual.abs() <= 1e-9);
        assert!(result.iterations <= 35);
    }

    #[test]
    fn accepts_endpoint_roots_and_existing_variables() {
        let at_endpoint = run("x-2", 2.0, 4.0).unwrap();
        assert_eq!(at_endpoint.root, 2.0);
        assert_eq!(at_endpoint.iterations, 0);

        let mut variables = BTreeMap::new();
        variables.insert("scale".into(), Value::Exact(3, 1));
        let result = solve(
            "scale*x-6",
            1.0,
            3.0,
            1e-10,
            100,
            &variables,
            Angle::Radians,
        )
        .unwrap();
        assert!((result.root - 2.0).abs() <= 1e-10);
    }

    #[test]
    fn rejects_missing_brackets_domains_complex_values_and_bad_limits() {
        assert!(run("x^2+1", -1.0, 1.0).unwrap_err().contains("没有异号"));
        assert!(run("1/x", -1.0, 1.0).is_err());
        assert!(run("sqrt(-1)+x", -1.0, 1.0).is_err());
        assert!(solve("x", 2.0, 1.0, 1e-10, 100, &BTreeMap::new(), Angle::Radians).is_err());
        assert!(solve("x", 0.0, 1.0, 0.0, 100, &BTreeMap::new(), Angle::Radians).is_err());
        assert!(solve("x", 0.0, 1.0, 1e-10, 201, &BTreeMap::new(), Angle::Radians).is_err());
    }

    #[test]
    fn iteration_limit_returns_an_explicit_nonconverged_result() {
        let result = solve(
            "x^2-2",
            1.0,
            2.0,
            1e-12,
            1,
            &BTreeMap::new(),
            Angle::Radians,
        )
        .unwrap();
        assert!(!result.converged);
        assert_eq!(result.iterations, 1);
    }
}
