//! The bracket solver reuses a parsed expression at many coordinates.
use super::{Angle, Evaluation, Parser, Prepared, Value, binary, function, prefix, variable};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
enum Operand {
    Fixed(Value),
    Slot(usize),
}

impl Operand {
    fn read(self, values: &[Value]) -> Value {
        match self {
            Self::Fixed(value) => value,
            Self::Slot(index) => values[index],
        }
    }
}

enum Step {
    Prefix(&'static str, Operand),
    Binary(&'static str, Operand, Operand, usize),
    Function(String, Vec<Operand>),
}

pub(crate) struct BisectionExpression {
    steps: Vec<Step>,
    result: Operand,
    values: Vec<Value>,
    angle: Angle,
}

impl BisectionExpression {
    /// Count per-coordinate execution work independently of the native solver limits.
    pub(crate) fn work_weight(&self) -> usize {
        let costly_constant = |operand| match operand {
            Operand::Fixed(Value::Exact(n, d)) => d != 1 || i64::try_from(n).is_err(),
            Operand::Fixed(Value::Complex(..)) => true,
            _ => false,
        };
        self.steps
            .iter()
            .map(|step| match step {
                Step::Binary("^", ..) => 5,
                Step::Binary(_, left, right, _)
                    if costly_constant(*left) || costly_constant(*right) =>
                {
                    3
                }
                Step::Function(name, values)
                    if matches!(
                        name.as_str(),
                        "sum" | "mean" | "median" | "varp" | "vars" | "stdp" | "stds"
                    ) =>
                {
                    16 + values.len() * 10
                }
                Step::Function(_, values) => 8 + values.len() * 2,
                Step::Prefix(..) | Step::Binary(..) => 1,
            })
            .sum::<usize>()
            + 1
    }

    pub(crate) fn evaluate(&mut self, x: f64) -> Result<Value, String> {
        self.values.clear();
        self.values.push(Value::Approx(x));
        let mut arguments = Vec::new();
        for step in &self.steps {
            let value = match step {
                Step::Prefix(op, value) => prefix(op, value.read(&self.values))?,
                Step::Binary(op, left, right, column) => binary(
                    op,
                    left.read(&self.values),
                    right.read(&self.values),
                    *column,
                )?,
                Step::Function(name, inputs) => {
                    arguments.clear();
                    arguments.extend(inputs.iter().map(|value| value.read(&self.values)));
                    function(name, &arguments, self.angle)?
                }
            };
            self.values.push(value);
        }
        Ok(self.result.read(&self.values))
    }
}

struct Plan<'a> {
    variables: &'a BTreeMap<String, Value>,
    angle: Angle,
    steps: Vec<Step>,
}

impl Plan<'_> {
    fn push(&mut self, step: Step) -> Operand {
        self.steps.push(step);
        Operand::Slot(self.steps.len())
    }
}

impl Evaluation for Plan<'_> {
    type Output = Operand;

    fn number(value: Value) -> Operand {
        Operand::Fixed(value)
    }

    fn variable(&self, name: &str) -> Option<Result<Operand, String>> {
        if name == "x" {
            Some(Ok(Operand::Slot(0)))
        } else {
            variable(self.variables, name).map(|result| result.map(Operand::Fixed))
        }
    }

    fn prefix(&mut self, op: &str, value: Operand) -> Result<Operand, String> {
        if let Operand::Fixed(value) = value {
            return prefix(op, value).map(Operand::Fixed);
        }
        Ok(match op {
            "-" => self.push(Step::Prefix("-", value)),
            "~" => self.push(Step::Prefix("~", value)),
            _ => value,
        })
    }

    fn binary(
        &mut self,
        op: &'static str,
        left: Operand,
        right: Operand,
        column: usize,
    ) -> Result<Operand, String> {
        match (left, right) {
            (Operand::Fixed(left), Operand::Fixed(right)) => {
                binary(op, left, right, column).map(Operand::Fixed)
            }
            _ => Ok(self.push(Step::Binary(op, left, right, column))),
        }
    }

    fn function(&mut self, name: &str, args: Vec<Operand>) -> Result<Operand, String> {
        let fixed = args
            .iter()
            .map(|operand| match operand {
                Operand::Fixed(value) => Some(*value),
                Operand::Slot(_) => None,
            })
            .collect::<Option<Vec<_>>>();
        match fixed {
            Some(values) => function(name, &values, self.angle).map(Operand::Fixed),
            None => Ok(self.push(Step::Function(name.to_owned(), args))),
        }
    }
}

impl Prepared {
    pub(crate) fn for_bisection(
        &self,
        variables: &BTreeMap<String, Value>,
        angle: Angle,
        first_x: f64,
    ) -> Result<BisectionExpression, String> {
        let mut parser = Parser::new(
            &self.tokens,
            Plan {
                variables,
                angle,
                steps: Vec::new(),
            },
        );
        match parser.parse() {
            Ok(result) => {
                let steps = parser.evaluation.steps;
                let values = Vec::with_capacity(steps.len() + 1);
                Ok(BisectionExpression {
                    steps,
                    result,
                    values,
                    angle,
                })
            }
            Err(error) => {
                // The original evaluator decides which error occurs first at the left endpoint.
                let mut values = variables.clone();
                values.insert("x".into(), Value::Approx(first_x));
                Err(self.evaluate(&values, angle).err().unwrap_or(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_plan_matches_live_evaluation_at_every_coordinate() {
        let mut variables = BTreeMap::from([
            ("scale".to_owned(), Value::exact(3, 2).unwrap()),
            ("signal".to_owned(), Value::Approx(0.25)),
            ("x".to_owned(), Value::Exact(999, 1)),
        ]);
        let formulas = [
            "+x+0+0+0-0.3",
            "scale*x-signal",
            "sin(30)+sin(x)-1/2",
            "sqrt(x)^2-x",
            "1/(x-1)",
            "sum(1/3, x, mean(1,2,3))",
            "varp(x,1/3,2/3)",
            "real((1+i)*x)+abs(-x)",
            "((0x7 & 3) << 1)+x",
            "~x",
            "cos(x)+cos(x*(32/3))",
            "-x",
        ];
        for angle in [Angle::Radians, Angle::Degrees] {
            for formula in formulas {
                let prepared = Prepared::new(formula).unwrap();
                let mut planned = prepared.for_bisection(&variables, angle, -3.0).unwrap();
                for x in [-3.0, -0.0, 0.0, 0.5, 1.0, 1e12] {
                    variables.insert("x".to_owned(), Value::Approx(x));
                    let expected = prepared.evaluate(&variables, angle);
                    let actual = planned.evaluate(x);
                    match (expected, actual) {
                        (Ok(Value::Approx(a)), Ok(Value::Approx(b))) => {
                            assert_eq!(a.to_bits(), b.to_bits(), "{formula} at {x}");
                        }
                        (a, b) => assert_eq!(a, b, "{formula} at {x}"),
                    }
                }
            }
        }
    }

    #[test]
    fn preparation_errors_use_the_same_first_error_as_the_live_evaluator() {
        let variables = BTreeMap::from([("x".to_owned(), Value::Approx(0.0))]);
        for formula in ["1/x+1/0", "1/x+undefined", "1/x + (1,2)", "sin(1,2)+x"] {
            let prepared = Prepared::new(formula).unwrap();
            let ordinary = prepared.evaluate(&variables, Angle::Radians);
            let planned = prepared.for_bisection(&variables, Angle::Radians, 0.0);
            assert!(ordinary.is_err(), "{formula}");
            assert_eq!(planned.err(), ordinary.err(), "{formula}");
        }
    }

    #[test]
    fn integral_float_fast_path_preserves_rounding_at_word_boundaries() {
        for n in [
            i128::MIN,
            i64::MIN as i128 - 1,
            i64::MIN as i128,
            -(1 << 53) - 1,
            0,
            (1 << 53) + 1,
            i64::MAX as i128,
            i64::MAX as i128 + 1,
            i128::MAX,
        ] {
            assert_eq!(Value::Exact(n, 1).float().to_bits(), (n as f64).to_bits());
        }
    }
}
