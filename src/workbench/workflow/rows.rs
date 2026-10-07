use super::*;
use serde_json::Value;
use std::cmp::Ordering as Comparison;

// Canonical decimal magnitude, without converting large JSON integers to f64.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    exponent: i32,
}
impl Decimal {
    fn new(value: &serde_json::Number) -> Self {
        let text = value.to_string();
        let negative = text.starts_with('-');
        let unsigned = text.trim_start_matches('-');
        let (mantissa, exponent) = unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, 0), |(m, e)| {
                (m, e.parse::<i32>().expect("JSON number exponent"))
            });
        let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len() as i32);
        let mut digits: Vec<_> = mantissa
            .bytes()
            .filter(|b| *b != b'.')
            .skip_while(|b| *b == b'0')
            .collect();
        let mut exponent = exponent - fraction;
        while digits.last() == Some(&b'0') {
            digits.pop();
            exponent += 1;
        }
        if digits.is_empty() {
            return Self {
                negative: false,
                digits: vec![],
                exponent: 0,
            };
        }
        Self {
            negative,
            digits,
            exponent,
        }
    }
}
impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Comparison {
        if self.negative != other.negative {
            return other.negative.cmp(&self.negative);
        }
        let order = match (self.digits.is_empty(), other.digits.is_empty()) {
            (true, true) => Comparison::Equal,
            (true, false) => Comparison::Less,
            (false, true) => Comparison::Greater,
            (false, false) => (self.digits.len() as i32 + self.exponent)
                .cmp(&(other.digits.len() as i32 + other.exponent))
                .then_with(|| {
                    (0..self.digits.len().max(other.digits.len()))
                        .map(|i| {
                            self.digits
                                .get(i)
                                .copied()
                                .unwrap_or(b'0')
                                .cmp(&other.digits.get(i).copied().unwrap_or(b'0'))
                        })
                        .find(|order| *order != Comparison::Equal)
                        .unwrap_or(Comparison::Equal)
                }),
        };
        if self.negative {
            order.reverse()
        } else {
            order
        }
    }
}
impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Comparison> {
        Some(self.cmp(other))
    }
}

// One total type order prevents mixed numeric/text comparisons from becoming cyclic.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    Null,
    Bool(bool),
    Number(Decimal),
    Text(String),
    Array(String),
    Object(String),
}
impl From<&Value> for Key {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool(*v),
            Value::Number(v) => Self::Number(Decimal::new(v)),
            Value::String(v) => Self::Text(v.clone()),
            Value::Array(_) => Self::Array(value.to_string()),
            Value::Object(_) => Self::Object(value.to_string()),
        }
    }
}

fn column(data: &Dataset, name: &str) -> Result<usize> {
    data.headers
        .iter()
        .position(|n| n == name)
        .with_context(|| format!("找不到列：{name}"))
}

pub(super) fn propose(
    data: &Dataset,
    step: &Step,
    cancel: &AtomicBool,
) -> Result<transform::Proposal> {
    let mut indices = Vec::with_capacity(data.rows.len());
    let description = match step {
        Step::Filter {
            column: name,
            predicate,
            value,
            case_sensitive,
        } => {
            let position = column(data, name)?;
            let literal = if matches!(predicate, Predicate::Equals | Predicate::NotEquals) {
                Some(serde_json::from_str::<Value>(value)?)
            } else {
                None
            };
            let needle = if *case_sensitive {
                value.clone()
            } else {
                value.to_lowercase()
            };
            for (i, row) in data.rows.iter().enumerate() {
                check_cancel(cancel)?;
                let cell = &row[position];
                let keep = match predicate {
                    Predicate::IsNull => cell.is_null(),
                    Predicate::IsNotNull => !cell.is_null(),
                    Predicate::Equals => Some(cell) == literal.as_ref(),
                    Predicate::NotEquals => Some(cell) != literal.as_ref(),
                    Predicate::Contains | Predicate::NotContains => {
                        let text = super::super::cell_text(cell);
                        let text = if *case_sensitive {
                            text
                        } else {
                            text.to_lowercase()
                        };
                        text.contains(&needle) == (*predicate == Predicate::Contains)
                    }
                };
                if keep {
                    indices.push(i);
                }
            }
            format!(
                "筛选列 {name} · {} · 保留 {} 行",
                predicate.label(),
                indices.len()
            )
        }
        Step::Deduplicate { columns } => {
            let positions = columns
                .iter()
                .map(|name| column(data, name))
                .collect::<Result<Vec<_>>>()?;
            let mut seen = std::collections::HashSet::new();
            for (i, row) in data.rows.iter().enumerate() {
                check_cancel(cancel)?;
                let key =
                    serde_json::to_string(&positions.iter().map(|p| &row[*p]).collect::<Vec<_>>())?;
                if seen.insert(key) {
                    indices.push(i);
                }
            }
            format!("按 {} 去重 · 保留首条", columns.join("、"))
        }
        Step::Sort { keys } => {
            let positions = keys
                .iter()
                .map(|key| column(data, &key.column))
                .collect::<Result<Vec<_>>>()?;
            let mut decorated = Vec::with_capacity(data.rows.len());
            for (i, row) in data.rows.iter().enumerate() {
                check_cancel(cancel)?;
                decorated.push((
                    i,
                    positions
                        .iter()
                        .map(|p| Key::from(&row[*p]))
                        .collect::<Vec<_>>(),
                ));
            }
            decorated.sort_by(|(_, a), (_, b)| {
                a.iter()
                    .zip(b)
                    .zip(keys)
                    .map(|((a, b), key)| {
                        let order = a.cmp(b);
                        if key.descending {
                            order.reverse()
                        } else {
                            order
                        }
                    })
                    .find(|order| *order != Comparison::Equal)
                    .unwrap_or(Comparison::Equal)
            });
            check_cancel(cancel)?;
            indices.extend(decorated.into_iter().map(|(i, _)| i));
            format!(
                "稳定排序 · {}",
                keys.iter()
                    .map(|k| format!(
                        "{} {}",
                        k.column,
                        if k.descending { "降序" } else { "升序" }
                    ))
                    .collect::<Vec<_>>()
                    .join("、")
            )
        }
        _ => anyhow::bail!("不是行操作"),
    };
    let (changed, examples) = if matches!(step, Step::Sort { .. }) {
        let moved = indices
            .iter()
            .enumerate()
            .filter(|(to, from)| *to != **from)
            .collect::<Vec<_>>();
        (
            moved.len(),
            moved
                .into_iter()
                .take(6)
                .map(|(to, from)| (format!("第{}行", from + 1), format!("第{}行", to + 1)))
                .collect(),
        )
    } else {
        let retained: std::collections::HashSet<_> = indices.iter().copied().collect();
        (
            data.rows.len() - indices.len(),
            (0..data.rows.len())
                .filter(|i| !retained.contains(i))
                .take(6)
                .map(|i| (format!("第{}行", i + 1), "已移除".into()))
                .collect(),
        )
    };
    let mut result = Dataset {
        headers: data.headers.clone(),
        rows: Vec::with_capacity(indices.len()),
    };
    for i in indices {
        check_cancel(cancel)?;
        result.rows.push(data.rows[i].clone());
    }
    Ok(transform::Proposal {
        data: result,
        changed,
        examples,
        description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn run(values: Vec<Value>, steps: Vec<Step>) -> Preview {
        let data = Dataset {
            headers: vec!["x".into()],
            rows: values.into_iter().map(|v| vec![v]).collect(),
        };
        let original = data.clone();
        let result = Definition {
            output: None,
            version: 2,
            name: "行处理".into(),
            steps,
        }
        .preview(&data, &AtomicBool::new(false))
        .unwrap();
        assert_eq!(data, original);
        result
    }
    #[test]
    fn exact_numbers_total_order_and_stable_ties() {
        let values = vec![
            json!(9007199254740993u64),
            json!(9007199254740992u64),
            json!(18446744073709551615u64),
            json!(-10),
            json!(-2),
            json!(1.0),
            json!(1),
            json!(null),
            json!(false),
            json!("0"),
            json!([]),
            json!({}),
            json!(1e-200),
            json!(-0.0),
            json!(0),
        ];
        let keys = values.iter().map(Key::from).collect::<Vec<_>>();
        for a in &keys {
            for b in &keys {
                for c in &keys {
                    if a <= b && b <= c {
                        assert!(a <= c);
                    }
                    assert_eq!(a.cmp(b), b.cmp(a).reverse());
                }
            }
        }
        let result = run(
            values,
            vec![Step::Sort {
                keys: vec![SortKey {
                    column: "x".into(),
                    descending: false,
                }],
            }],
        );
        assert_eq!(
            result
                .result
                .rows
                .into_iter()
                .map(|r| r[0].clone())
                .collect::<Vec<_>>(),
            vec![
                json!(null),
                json!(false),
                json!(-10),
                json!(-2),
                json!(-0.0),
                json!(0),
                json!(1e-200),
                json!(1.0),
                json!(1),
                json!(9007199254740992u64),
                json!(9007199254740993u64),
                json!(18446744073709551615u64),
                json!("0"),
                json!([]),
                json!({})
            ]
        );
    }
    #[test]
    fn typed_equality_and_first_duplicate_retention() {
        let result = run(
            vec![
                json!("001"),
                json!(1),
                json!("1"),
                json!(1),
                json!(1.0),
                json!(null),
            ],
            vec![Step::Deduplicate {
                columns: vec!["x".into()],
            }],
        );
        assert_eq!(result.result.rows.len(), 5);
        assert_eq!(result.steps[0].changed, 1);
        let result = run(
            vec![json!("1"), json!(1), json!(null)],
            vec![Step::Filter {
                column: "x".into(),
                predicate: Predicate::Equals,
                value: "1".into(),
                case_sensitive: true,
            }],
        );
        assert_eq!(result.result.rows, vec![vec![json!(1)]]);
    }
    #[test]
    fn chain_rename_filter_multikey_sort_dedup_and_projection() {
        let data = Dataset::parse(r#"[{"id":"001","group":"A","n":2},{"id":"002","group":"a","n":3},{"id":"003","group":"A","n":1},{"id":"004","group":"B","n":4}]"#, super::super::super::DataFormat::Json, b',').unwrap();
        let definition = Definition {
            output: None,
            version: 2,
            name: "组合".into(),
            steps: vec![
                Step::Column {
                    column: "group".into(),
                    operation: ColumnOperation::Rename,
                    value: "kind".into(),
                },
                Step::Filter {
                    column: "kind".into(),
                    predicate: Predicate::Contains,
                    value: "a".into(),
                    case_sensitive: false,
                },
                Step::Sort {
                    keys: vec![
                        SortKey {
                            column: "n".into(),
                            descending: true,
                        },
                        SortKey {
                            column: "id".into(),
                            descending: false,
                        },
                    ],
                },
                Step::Deduplicate {
                    columns: vec!["kind".into()],
                },
                Step::SelectColumns {
                    columns: vec!["id".into(), "n".into()],
                },
            ],
        };
        let preview = definition.preview(&data, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            preview.result.rows,
            vec![vec![json!("002"), json!(3)], vec![json!("001"), json!(2)]]
        );
        assert_eq!(
            Definition::parse(&serde_json::to_vec(&definition).unwrap()).unwrap(),
            definition
        );
        let mut old = definition.clone();
        old.version = 1;
        assert!(old.validate().is_err());
        let mut missing = definition.clone();
        missing.steps.push(Step::Deduplicate {
            columns: vec!["missing".into()],
        });
        assert!(
            format!(
                "{:#}",
                missing
                    .preview(&data, &AtomicBool::new(false))
                    .err()
                    .unwrap()
            )
            .contains("第6步失败")
        );
        assert!(definition.preview(&data, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn invalid_parameters_and_empty_results() {
        let mut definition = Definition {
            output: None,
            version: 2,
            name: "x".into(),
            steps: vec![Step::Filter {
                column: "x".into(),
                predicate: Predicate::Equals,
                value: "bare text".into(),
                case_sensitive: true,
            }],
        };
        assert!(definition.validate().is_err());
        definition.steps = vec![Step::Sort {
            keys: vec![
                SortKey {
                    column: "x".into(),
                    descending: false
                };
                2
            ],
        }];
        assert!(definition.validate().is_err());
        definition.version = 3;
        assert!(definition.validate().is_err());
        let preview = run(
            vec![json!(1)],
            vec![Step::Filter {
                column: "x".into(),
                predicate: Predicate::IsNull,
                value: String::new(),
                case_sensitive: true,
            }],
        );
        assert!(preview.result.rows.is_empty());
    }
}
