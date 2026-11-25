//! Bounded expression evaluation; no scripts, file access or network operations.
mod dates;
pub mod exchange;
mod matrix_ui;
mod plot;
#[cfg(feature = "ui-preview")]
mod plot_preview;
mod plot_render;
mod plot_ui;
mod root_solver;
mod statistics;
mod ui;
mod worksheet;
mod worksheet_ui;
pub use ui::State;

use std::collections::BTreeMap;
pub use zi_matrix_core::{Angle, Value, valid_variable};
use zi_matrix_core::{expression as parser, matrix};

pub fn evaluate(
    expression: &str,
    variables: &BTreeMap<String, Value>,
    angle: Angle,
) -> Result<Value, String> {
    parser::evaluate(expression, variables, angle)
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
    fn complex_arithmetic_functions_and_real_only_boundaries() {
        let sqrt_negative = eval("sqrt(-1)");
        assert_eq!(sqrt_negative, Value::Complex(0.0, 1.0));
        let square = eval("i^2");
        let (real, imaginary) = square.parts();
        assert!((real + 1.0).abs() < 1e-12);
        assert!(imaginary.abs() < 1e-12);
        let principal_power = eval("(-1)^0.5");
        let (real, imaginary) = principal_power.parts();
        assert!(real.abs() < 1e-12);
        assert!((imaginary - 1.0).abs() < 1e-12);
        let quotient = eval("(3+4*i)/(1-2*i)");
        let (real, imaginary) = quotient.parts();
        assert!((real + 1.0).abs() < 1e-12);
        assert!((imaginary - 2.0).abs() < 1e-12);
        assert_eq!(eval("abs(3+4*i)"), Value::Approx(5.0));
        assert_eq!(eval("real(2+3*i)"), Value::Approx(2.0));
        assert_eq!(eval("imag(2+3*i)"), Value::Approx(3.0));
        let logarithm = eval("ln(-1)");
        let (real, imaginary) = logarithm.parts();
        assert!(real.abs() < 1e-12);
        assert!((imaginary - std::f64::consts::PI).abs() < 1e-12);
        assert!(eval("sqrt(1+i)").display().contains('i'));
        assert_eq!(Value::Complex(0.0, -1.0).display(), "≈ −i");
        assert!(evaluate("0^(-1)", &BTreeMap::new(), Angle::Radians).is_err());
        let tangent = eval("tan(1000*i)");
        let (real, imaginary) = tangent.parts();
        assert!(real.abs() < 1e-12);
        assert!((imaginary - 1.0).abs() < 1e-12);
        let large = Value::Complex(1e308, 1e308)
            .binary("/", Value::Complex(1e308, 1e308))
            .unwrap();
        let (real, imaginary) = large.parts();
        assert!((real - 1.0).abs() < 1e-12);
        assert!(imaginary.abs() < 1e-12);
        let (root_real, root_imaginary) = zi_matrix_core::complex_sqrt(1e308, 1e308);
        assert!(root_real.is_finite() && root_imaginary.is_finite());
        assert!(evaluate("floor(1+i)", &BTreeMap::new(), Angle::Radians).is_err());
        assert!(
            evaluate("sum(1+i,2)", &BTreeMap::new(), Angle::Radians)
                .unwrap_err()
                .contains("仅支持实数")
        );
        assert!(evaluate("min(i,2)", &BTreeMap::new(), Angle::Radians).is_err());
        assert!(!valid_variable("i"));
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
