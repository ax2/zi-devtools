//! Native binds to the shared arithmetic and retains its own workbench tests.
#[cfg(any(test, feature = "ui-preview"))]
use super::Value;
pub(super) use zi_matrix_core::statistics::{compare, evaluate};

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
    use std::{cmp::Ordering, collections::BTreeMap};
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
