use super::{Angle, Value, exact_number as number};
use std::collections::BTreeMap;

mod bisection;

enum Token {
    Number(Value),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
    End,
}
trait Evaluation {
    type Output: Copy;

    fn number(value: Value) -> Self::Output;
    fn variable(&self, name: &str) -> Option<Result<Self::Output, String>>;
    fn prefix(&mut self, op: &str, value: Self::Output) -> Result<Self::Output, String>;
    fn binary(
        &mut self,
        op: &'static str,
        left: Self::Output,
        right: Self::Output,
        column: usize,
    ) -> Result<Self::Output, String>;
    fn function(&mut self, name: &str, args: Vec<Self::Output>) -> Result<Self::Output, String>;
}

struct Direct<'a> {
    variables: &'a BTreeMap<String, Value>,
    angle: Angle,
}

impl Evaluation for Direct<'_> {
    type Output = Value;

    fn number(value: Value) -> Value {
        value
    }

    fn variable(&self, name: &str) -> Option<Result<Value, String>> {
        variable(self.variables, name)
    }

    fn prefix(&mut self, op: &str, value: Value) -> Result<Value, String> {
        prefix(op, value)
    }

    fn binary(
        &mut self,
        op: &'static str,
        left: Value,
        right: Value,
        column: usize,
    ) -> Result<Value, String> {
        binary(op, left, right, column)
    }

    fn function(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        function(name, &args, self.angle)
    }
}

struct Parser<'a, E> {
    tokens: &'a [(Token, usize)],
    at: usize,
    depth: usize,
    evaluation: E,
}

pub fn evaluate(
    input: &str,
    variables: &BTreeMap<String, Value>,
    angle: Angle,
) -> Result<Value, String> {
    if input.len() > 2048 {
        return Err("表达式最多 2048 字节".into());
    }
    if let Some((left, target)) = input.split_once("->") {
        let (expression, unit) = left
            .trim()
            .rsplit_once(char::is_whitespace)
            .ok_or("单位转换示例：5 km -> m")?;
        let source = unit_definition(unit.trim()).ok_or("来源单位未支持")?;
        let target = unit_definition(target.trim()).ok_or("目标单位未支持")?;
        if source.0 != target.0 {
            return Err("单位维度不一致".into());
        }
        let value = evaluate(expression.trim(), variables, angle)?;
        return value
            .binary("*", source.1)?
            .binary("+", source.2)?
            .binary("-", target.2)?
            .binary("/", target.1);
    }
    let tokens = lex(input)?;
    Parser::new(&tokens, Direct { variables, angle }).parse()
}

/// Reuse bounded lexing when evaluating the same formula at many coordinates.
pub struct Prepared {
    tokens: Vec<(Token, usize)>,
}
impl Prepared {
    pub fn new(input: &str) -> Result<Self, String> {
        if input.len() > 2048 || input.trim().is_empty() {
            return Err("函数公式需为1–2048字节".into());
        }
        Ok(Self {
            tokens: lex(input)?,
        })
    }
    pub fn evaluate(
        &self,
        variables: &BTreeMap<String, Value>,
        angle: Angle,
    ) -> Result<Value, String> {
        Parser::new(&self.tokens, Direct { variables, angle }).parse()
    }
}

fn unit_definition(unit: &str) -> Option<(&'static str, Value, Value)> {
    let (dimension, n, d, offset) = match unit {
        "mm" => ("length", 1, 1000, Value::Exact(0, 1)),
        "cm" => ("length", 1, 100, Value::Exact(0, 1)),
        "m" => ("length", 1, 1, Value::Exact(0, 1)),
        "km" => ("length", 1000, 1, Value::Exact(0, 1)),
        "in" => ("length", 127, 5000, Value::Exact(0, 1)),
        "ft" => ("length", 381, 1250, Value::Exact(0, 1)),
        "mi" => ("length", 201168, 125, Value::Exact(0, 1)),
        "mg" => ("mass", 1, 1000000, Value::Exact(0, 1)),
        "g" => ("mass", 1, 1000, Value::Exact(0, 1)),
        "kg" => ("mass", 1, 1, Value::Exact(0, 1)),
        "lb" => ("mass", 45359237, 100000000, Value::Exact(0, 1)),
        "ms" => ("duration", 1, 1000, Value::Exact(0, 1)),
        "s" => ("duration", 1, 1, Value::Exact(0, 1)),
        "min" => ("duration", 60, 1, Value::Exact(0, 1)),
        "h" => ("duration", 3600, 1, Value::Exact(0, 1)),
        "day" => ("duration", 86400, 1, Value::Exact(0, 1)),
        "B" => ("storage", 1, 1, Value::Exact(0, 1)),
        "KB" => ("storage", 1000, 1, Value::Exact(0, 1)),
        "MB" => ("storage", 1000000, 1, Value::Exact(0, 1)),
        "GB" => ("storage", 1000000000, 1, Value::Exact(0, 1)),
        "KiB" => ("storage", 1024, 1, Value::Exact(0, 1)),
        "MiB" => ("storage", 1048576, 1, Value::Exact(0, 1)),
        "GiB" => ("storage", 1073741824, 1, Value::Exact(0, 1)),
        "K" => ("temperature", 1, 1, Value::Exact(0, 1)),
        "C" => ("temperature", 1, 1, Value::Exact(5463, 20)),
        "F" => ("temperature", 5, 9, Value::exact(45967, 180).ok()?),
        _ => return None,
    };
    Some((dimension, Value::exact(n, d).ok()?, offset))
}

fn lex(input: &str) -> Result<Vec<(Token, usize)>, String> {
    let b = input.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let t = if b[i].is_ascii_digit() || b[i] == b'.' {
            if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'b' | b'o') {
                i += 2;
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
            } else {
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                if i < b.len() && matches!(b[i], b'e' | b'E') {
                    i += 1;
                    if i < b.len() && matches!(b[i], b'+' | b'-') {
                        i += 1;
                    }
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            Token::Number(
                number(&input[start..i]).map_err(|e| format!("第 {} 列：{e}", start + 1))?,
            )
        } else if b[i].is_ascii_alphabetic() {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Token::Name(input[start..i].to_owned())
        } else {
            i += 1;
            match b[start] {
                b'+' => Token::Op("+"),
                b'-' => Token::Op("-"),
                b'*' => Token::Op("*"),
                b'/' => Token::Op("/"),
                b'^' => Token::Op("^"),
                b'%' => Token::Op("%"),
                b'&' => Token::Op("&"),
                b'|' => Token::Op("|"),
                b'~' => Token::Op("~"),
                b'(' => Token::Open,
                b')' => Token::Close,
                b',' => Token::Comma,
                b'<' | b'>' if i < b.len() && b[i] == b[start] => {
                    i += 1;
                    Token::Op(if b[start] == b'<' { "<<" } else { ">>" })
                }
                _ => return Err(format!("第 {} 列：不支持此字符", start + 1)),
            }
        };
        tokens.push((t, start));
        if tokens.len() > 512 {
            return Err("表达式最多 512 个词元".into());
        }
    }
    tokens.push((Token::End, input.len()));
    Ok(tokens)
}

impl<'a, E: Evaluation> Parser<'a, E> {
    fn new(tokens: &'a [(Token, usize)], evaluation: E) -> Self {
        Self {
            tokens,
            at: 0,
            depth: 0,
            evaluation,
        }
    }

    fn parse(&mut self) -> Result<E::Output, String> {
        let result = self.expression(0)?;
        if !matches!(self.tokens[self.at].0, Token::End) {
            return Err(self.error("存在多余内容"));
        }
        Ok(result)
    }

    fn column(&self) -> usize {
        self.tokens[self.at].1 + 1
    }

    fn error(&self, message: &str) -> String {
        format!("第 {} 列：{message}", self.column())
    }
    fn expression(&mut self, min: u8) -> Result<E::Output, String> {
        if self.depth >= 64 {
            return Err(self.error("表达式嵌套最多 64 层"));
        }
        self.depth += 1;
        let tokens = self.tokens;
        let mut left = match &tokens[self.at].0 {
            Token::Number(v) => {
                self.at += 1;
                E::number(*v)
            }
            Token::Op(op @ ("+" | "-" | "~")) => {
                self.at += 1;
                let v = self.expression(60)?;
                self.evaluation.prefix(op, v)?
            }
            Token::Open => {
                self.at += 1;
                let v = self.expression(0)?;
                if !matches!(self.tokens[self.at].0, Token::Close) {
                    return Err(self.error("缺少右括号"));
                }
                self.at += 1;
                v
            }
            Token::Name(name) => {
                self.at += 1;
                if matches!(self.tokens[self.at].0, Token::Open) {
                    self.at += 1;
                    let mut args = Vec::new();
                    if !matches!(self.tokens[self.at].0, Token::Close) {
                        loop {
                            args.push(self.expression(0)?);
                            if args.len() > 64 {
                                return Err(self.error("函数最多 64 个参数"));
                            }
                            if !matches!(self.tokens[self.at].0, Token::Comma) {
                                break;
                            }
                            self.at += 1;
                        }
                    }
                    if !matches!(self.tokens[self.at].0, Token::Close) {
                        return Err(self.error("函数缺少右括号"));
                    }
                    self.at += 1;
                    self.evaluation.function(name, args)?
                } else {
                    self.evaluation
                        .variable(name)
                        .ok_or_else(|| self.error(&format!("未知变量 {name}")))??
                }
            }
            _ => return Err(self.error("需要数字、变量或左括号")),
        };
        while let Token::Op(op) = self.tokens[self.at].0 {
            let (l, r) = match op {
                "|" => (10, 11),
                "&" => (20, 21),
                "<<" | ">>" => (30, 31),
                "+" | "-" => (40, 41),
                "*" | "/" | "%" => (50, 51),
                "^" => (70, 70),
                _ => break,
            };
            if l < min {
                break;
            }
            self.at += 1;
            let right = self.expression(r)?;
            left = self.evaluation.binary(op, left, right, self.column())?;
        }
        self.depth -= 1;
        Ok(left)
    }
}

fn variable(variables: &BTreeMap<String, Value>, name: &str) -> Option<Result<Value, String>> {
    Some(match name {
        "pi" => Ok(Value::Approx(std::f64::consts::PI)),
        "e" => Ok(Value::Approx(std::f64::consts::E)),
        "i" => Ok(Value::Complex(0.0, 1.0)),
        _ => match *variables.get(name)? {
            Value::Exact(n, d) => Value::exact(n, d),
            Value::Approx(x) => Value::approximate(x),
            Value::Complex(real, imaginary) => Value::complex(real, imaginary),
        },
    })
}

fn prefix(op: &str, value: Value) -> Result<Value, String> {
    match op {
        "-" => value.checked_neg(),
        "~" => Ok(Value::Exact(!value.integer()?, 1)),
        _ => Ok(value),
    }
}

fn binary(op: &str, left: Value, right: Value, column: usize) -> Result<Value, String> {
    left.binary(op, right)
        .map_err(|reason| format!("第 {column} 列：{reason}"))
}

fn function(name: &str, args: &[Value], angle: Angle) -> Result<Value, String> {
    if let Some(result) = super::statistics::evaluate(name, args) {
        return result;
    }
    if matches!(name, "min" | "max" | "xor") && args.len() == 2 {
        return if name == "xor" {
            Ok(Value::Exact(args[0].integer()? ^ args[1].integer()?, 1))
        } else {
            args[0].real()?;
            args[1].real()?;
            let less = super::statistics::compare(args[0], args[1]).is_lt();
            Ok(if less == (name == "min") {
                args[0]
            } else {
                args[1]
            })
        };
    }
    if args.len() != 1 {
        return Err(format!("{name} 需要一个参数；min/max/xor 需要两个"));
    }
    let v = args[0];
    if name == "abs" {
        return match v {
            Value::Complex(real, imaginary) => Value::approximate(real.hypot(imaginary)),
            _ => {
                let x = v.real()?;
                if x < 0.0 { v.checked_neg() } else { Ok(v) }
            }
        };
    }
    if matches!(name, "real" | "re" | "imag" | "im" | "conj" | "arg") {
        let (real, imaginary) = v.parts();
        return match name {
            "real" | "re" if imaginary == 0.0 => Ok(v),
            "real" | "re" => Value::approximate(real),
            "imag" | "im" if imaginary == 0.0 => Ok(Value::Exact(0, 1)),
            "imag" | "im" => Value::approximate(imaginary),
            "conj" if imaginary == 0.0 => Ok(v),
            "conj" => Value::complex(real, -imaginary),
            "arg" => {
                let radians = imaginary.atan2(real);
                Value::approximate(if angle == Angle::Degrees {
                    radians.to_degrees()
                } else {
                    radians
                })
            }
            _ => unreachable!(),
        };
    }
    if matches!(name, "floor" | "ceil" | "round") {
        return match v {
            Value::Exact(n, d) => {
                let floor = n.div_euclid(d);
                let ceil = floor + i128::from(n.rem_euclid(d) != 0);
                Ok(Value::Exact(
                    match name {
                        "floor" => floor,
                        "ceil" => ceil,
                        _ => {
                            let fraction = n.rem_euclid(d);
                            if fraction > d / 2 || (d % 2 == 0 && fraction == d / 2 && n >= 0) {
                                ceil
                            } else {
                                floor
                            }
                        }
                    },
                    1,
                ))
            }
            Value::Approx(x) => Value::approximate(match name {
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                _ => x.round(),
            }),
            Value::Complex(_, 0.0) => {
                let x = v.real()?;
                Value::approximate(match name {
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    _ => x.round(),
                })
            }
            Value::Complex(_, _) => Err("floor/ceil/round 仅支持实数".into()),
        };
    }
    let (x, y) = v.parts();
    let factor = if angle == Angle::Degrees {
        std::f64::consts::PI / 180.0
    } else {
        1.0
    };
    if matches!(name, "sin" | "cos" | "tan") && y != 0.0 {
        let a = x * factor;
        let b = y * factor;
        let result = match name {
            "sin" => (a.sin() * b.cosh(), a.cos() * b.sinh()),
            "cos" => (a.cos() * b.cosh(), -a.sin() * b.sinh()),
            "tan" => {
                let decay = (-2.0 * b.abs()).exp();
                let denominator = 1.0 + 2.0 * (2.0 * a).cos() * decay + decay * decay;
                (
                    2.0 * (2.0 * a).sin() * decay / denominator,
                    b.signum() * (1.0 - decay * decay) / denominator,
                )
            }
            _ => unreachable!(),
        };
        return Value::complex(result.0, result.1);
    }
    if y != 0.0 {
        return match name {
            "sqrt" => {
                let result = super::complex_sqrt(x, y);
                Value::complex(result.0, result.1)
            }
            "ln" => Value::complex(x.hypot(y).ln(), y.atan2(x)),
            "log10" => Value::complex(x.hypot(y).log10(), y.atan2(x) / std::f64::consts::LN_10),
            "exp" => {
                let scale = x.exp();
                Value::complex(scale * y.cos(), scale * y.sin())
            }
            _ => Err(format!("{name} 不支持复数参数")),
        };
    }
    if x < 0.0 && matches!(name, "sqrt" | "ln" | "log10") {
        return match name {
            "sqrt" => Value::complex(0.0, (-x).sqrt()),
            "ln" => Value::complex(x.abs().ln(), std::f64::consts::PI),
            "log10" => Value::complex(
                x.abs().log10(),
                std::f64::consts::PI / std::f64::consts::LN_10,
            ),
            _ => unreachable!(),
        };
    }
    let x = v.real()?;
    let radians = if angle == Angle::Degrees {
        x.to_radians()
    } else {
        x
    };
    Value::approximate(match name {
        "sin" => radians.sin(),
        "cos" => radians.cos(),
        "tan" => radians.tan(),
        "sqrt" => x.sqrt(),
        "ln" => x.ln(),
        "log10" => x.log10(),
        "exp" => x.exp(),
        _ => return Err(format!("未知函数 {name}")),
    })
}
