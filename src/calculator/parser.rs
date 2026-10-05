use super::{Angle, Value};
use std::collections::BTreeMap;

#[derive(Clone)]
enum Token {
    Number(Value),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
    End,
}
struct Parser<'a> {
    tokens: Vec<(Token, usize)>,
    at: usize,
    depth: usize,
    variables: &'a BTreeMap<String, Value>,
    angle: Angle,
}

pub(super) fn evaluate(
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
    let mut p = Parser {
        tokens,
        at: 0,
        depth: 0,
        variables,
        angle,
    };
    let result = p.expression(0)?;
    if !matches!(p.tokens[p.at].0, Token::End) {
        return Err(p.error("存在多余内容"));
    }
    Ok(result)
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

fn number(text: &str) -> Result<Value, String> {
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

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("第 {} 列：{message}", self.tokens[self.at].1 + 1)
    }
    fn expression(&mut self, min: u8) -> Result<Value, String> {
        if self.depth >= 64 {
            return Err(self.error("表达式嵌套最多 64 层"));
        }
        self.depth += 1;
        let mut left = match self.tokens[self.at].0.clone() {
            Token::Number(v) => {
                self.at += 1;
                v
            }
            Token::Op(op @ ("+" | "-" | "~")) => {
                self.at += 1;
                let v = self.expression(60)?;
                match op {
                    "-" => v.neg()?,
                    "~" => Value::Exact(!v.integer()?, 1),
                    _ => v,
                }
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
                            if args.len() > 8 {
                                return Err(self.error("函数最多 8 个参数"));
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
                    function(&name, &args, self.angle)?
                } else {
                    match name.as_str() {
                        "pi" => Value::Approx(std::f64::consts::PI),
                        "e" => Value::Approx(std::f64::consts::E),
                        _ => match *self
                            .variables
                            .get(&name)
                            .ok_or_else(|| self.error(&format!("未知变量 {name}")))?
                        {
                            Value::Exact(n, d) => Value::exact(n, d)?,
                            Value::Approx(x) => Value::approximate(x)?,
                        },
                    }
                }
            }
            _ => return Err(self.error("需要数字、变量或左括号")),
        };
        loop {
            let Token::Op(op) = self.tokens[self.at].0 else {
                break;
            };
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
            left = left.binary(op, right).map_err(|e| self.error(&e))?;
        }
        self.depth -= 1;
        Ok(left)
    }
}

fn function(name: &str, args: &[Value], angle: Angle) -> Result<Value, String> {
    if matches!(name, "min" | "max" | "xor") && args.len() == 2 {
        return if name == "xor" {
            Ok(Value::Exact(args[0].integer()? ^ args[1].integer()?, 1))
        } else {
            let less = match (args[0], args[1]) {
                (Value::Exact(a, b), Value::Exact(c, d)) => {
                    if b == d {
                        a < c
                    } else {
                        a.checked_mul(d).ok_or("精确比较超出范围")?
                            < c.checked_mul(b).ok_or("精确比较超出范围")?
                    }
                }
                _ => args[0].float() < args[1].float(),
            };
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
    let x = v.float();
    let radians = match angle {
        Angle::Degrees => x.to_radians(),
        Angle::Radians => x,
    };
    if name == "abs" {
        return if x < 0.0 { v.neg() } else { Ok(v) };
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
            Value::Approx(_) => Value::approximate(match name {
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                _ => x.round(),
            }),
        };
    }
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
