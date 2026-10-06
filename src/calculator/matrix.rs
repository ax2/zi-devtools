use super::Value;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    Add,
    Subtract,
    Multiply,
    Transpose,
    Determinant,
    Inverse,
    Solve,
}

impl Operation {
    pub const ALL: [Self; 7] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Transpose,
        Self::Determinant,
        Self::Inverse,
        Self::Solve,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "A + B",
            Self::Subtract => "A − B",
            Self::Multiply => "A × B",
            Self::Transpose => "转置 A",
            Self::Determinant => "行列式 det(A)",
            Self::Inverse => "逆矩阵 A",
            Self::Solve => "解 AX = B",
        }
    }
    pub fn needs_b(self) -> bool {
        matches!(
            self,
            Self::Add | Self::Subtract | Self::Multiply | Self::Solve
        )
    }
}

impl Matrix {
    pub fn new(rows: usize, cols: usize, cells: Vec<Value>) -> Result<Self, String> {
        if !(1..=8).contains(&rows) || !(1..=8).contains(&cols) || cells.len() != rows * cols {
            return Err("矩阵每轴需为 1–8，单元格数量须与维度一致".into());
        }
        if cells.iter().any(|v| match v {
            Value::Exact(_, d) => *d <= 0,
            Value::Approx(x) => !x.is_finite(),
        }) {
            return Err("矩阵包含无效数值".into());
        }
        Ok(Self { rows, cols, cells })
    }
    fn get(&self, row: usize, col: usize) -> Value {
        self.cells[row * self.cols + col]
    }
    pub fn approximate(&self) -> bool {
        self.cells.iter().any(|v| matches!(v, Value::Approx(_)))
    }
    pub fn tsv(&self) -> String {
        self.cells
            .chunks(self.cols)
            .map(|row| {
                row.iter()
                    .map(|v| v.display().trim_start_matches("≈ ").to_owned())
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn square(&self) -> Result<(), String> {
        if self.rows == self.cols {
            Ok(())
        } else {
            Err("此操作要求 A 为方阵".into())
        }
    }
    fn identity(size: usize) -> Self {
        Self {
            rows: size,
            cols: size,
            cells: (0..size * size)
                .map(|i| Value::Exact(i128::from(i / size == i % size), 1))
                .collect(),
        }
    }
    fn pivot(&self, col: usize) -> Option<usize> {
        // Exact comparisons avoid losing a tiny nonzero rational in float conversion.
        let zero = |v| match v {
            Value::Exact(n, _) => n == 0,
            Value::Approx(x) => x == 0.0,
        };
        (col..self.rows)
            .filter(|r| !zero(self.get(*r, col)))
            .max_by(|a, b| {
                self.get(*a, col)
                    .float()
                    .abs()
                    .total_cmp(&self.get(*b, col).float().abs())
            })
    }
    fn swap_rows(&mut self, a: usize, b: usize) {
        for c in 0..self.cols {
            self.cells.swap(a * self.cols + c, b * self.cols + c);
        }
    }
    fn solve(&self, rhs: &Self) -> Result<Self, String> {
        self.square()?;
        if rhs.rows != self.rows {
            return Err("解 AX=B 要求 B 的行数与 A 相同".into());
        }
        let mut a = self.clone();
        let mut b = rhs.clone();
        for col in 0..a.cols {
            let pivot = a.pivot(col).ok_or("A 为奇异矩阵，不能给出唯一解或逆矩阵")?;
            a.swap_rows(col, pivot);
            b.swap_rows(col, pivot);
            let divisor = a.get(col, col);
            for c in col..a.cols {
                a.cells[col * a.cols + c] = a.get(col, c).binary("/", divisor)?;
            }
            for c in 0..b.cols {
                b.cells[col * b.cols + c] = b.get(col, c).binary("/", divisor)?;
            }
            for r in 0..a.rows {
                if r == col {
                    continue;
                }
                let factor = a.get(r, col);
                for c in col..a.cols {
                    a.cells[r * a.cols + c] = a
                        .get(r, c)
                        .binary("-", factor.binary("*", a.get(col, c))?)?;
                }
                for c in 0..b.cols {
                    b.cells[r * b.cols + c] = b
                        .get(r, c)
                        .binary("-", factor.binary("*", b.get(col, c))?)?;
                }
            }
        }
        Ok(b)
    }
    fn determinant(&self) -> Result<Self, String> {
        self.square()?;
        let mut a = self.clone();
        let mut result = Value::Exact(1, 1);
        for col in 0..a.cols {
            let Some(pivot) = a.pivot(col) else {
                return Self::new(
                    1,
                    1,
                    vec![if self.approximate() {
                        Value::Approx(0.0)
                    } else {
                        Value::Exact(0, 1)
                    }],
                );
            };
            if pivot != col {
                a.swap_rows(col, pivot);
                result = result.neg()?;
            }
            let divisor = a.get(col, col);
            result = result.binary("*", divisor)?;
            for r in col + 1..a.rows {
                let factor = a.get(r, col).binary("/", divisor)?;
                for c in col + 1..a.cols {
                    a.cells[r * a.cols + c] = a
                        .get(r, c)
                        .binary("-", factor.binary("*", a.get(col, c))?)?;
                }
                a.cells[r * a.cols + col] = Value::Exact(0, 1);
            }
        }
        Self::new(1, 1, vec![result])
    }
    pub fn apply(&self, operation: Operation, b: Option<&Self>) -> Result<Self, String> {
        if operation == Operation::Transpose {
            return Self::new(
                self.cols,
                self.rows,
                (0..self.cols)
                    .flat_map(|c| (0..self.rows).map(move |r| self.get(r, c)))
                    .collect(),
            );
        }
        if operation == Operation::Determinant {
            return self.determinant();
        }
        if operation == Operation::Inverse {
            self.square()?;
            return self.solve(&Self::identity(self.rows));
        }
        let b = b.ok_or("此操作需要矩阵 B")?;
        if operation == Operation::Solve {
            return self.solve(b);
        }
        if operation == Operation::Multiply {
            if self.cols != b.rows {
                return Err("A×B 要求 A 列数等于 B 行数".into());
            }
            let mut cells = Vec::with_capacity(self.rows * b.cols);
            for r in 0..self.rows {
                for c in 0..b.cols {
                    let mut sum = Value::Exact(0, 1);
                    for k in 0..self.cols {
                        sum = sum.binary("+", self.get(r, k).binary("*", b.get(k, c))?)?;
                    }
                    cells.push(sum);
                }
            }
            return Self::new(self.rows, b.cols, cells);
        }
        if self.rows != b.rows || self.cols != b.cols {
            return Err("加减要求 A 与 B 行列数相同".into());
        }
        Self::new(
            self.rows,
            self.cols,
            self.cells
                .iter()
                .zip(&b.cells)
                .map(|(a, b)| {
                    a.binary(
                        if operation == Operation::Add {
                            "+"
                        } else {
                            "-"
                        },
                        *b,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(rows: usize, cols: usize, nums: &[i128]) -> Matrix {
        Matrix::new(
            rows,
            cols,
            nums.iter().map(|n| Value::Exact(*n, 1)).collect(),
        )
        .unwrap()
    }
    #[test]
    fn rectangular_product_transpose_and_dimensions() {
        let a = m(2, 3, &[1, 2, 3, 4, 5, 6]);
        let b = m(3, 2, &[7, 8, 9, 10, 11, 12]);
        assert_eq!(
            a.apply(Operation::Multiply, Some(&b)).unwrap(),
            m(2, 2, &[58, 64, 139, 154])
        );
        assert_eq!(
            a.apply(Operation::Transpose, None).unwrap(),
            m(3, 2, &[1, 4, 2, 5, 3, 6])
        );
        assert!(a.apply(Operation::Add, Some(&b)).is_err());
        assert!(a.apply(Operation::Inverse, None).is_err());
        assert!(a.apply(Operation::Multiply, Some(&a)).is_err());
        assert_eq!(
            a.apply(Operation::Subtract, Some(&a)).unwrap(),
            m(2, 3, &[0; 6])
        );
    }
    #[test]
    fn pivoted_inverse_multirhs_and_exact_residual() {
        let a = m(3, 3, &[0, 2, 1, 1, 1, 0, 2, 0, 1]);
        let rhs = m(3, 2, &[5, 7, 3, 4, 4, 6]);
        let x = a.apply(Operation::Solve, Some(&rhs)).unwrap();
        assert_eq!(a.apply(Operation::Multiply, Some(&x)).unwrap(), rhs);
        let inverse = a.apply(Operation::Inverse, None).unwrap();
        assert_eq!(
            a.apply(Operation::Multiply, Some(&inverse)).unwrap(),
            Matrix::identity(3)
        );
        assert_eq!(a.apply(Operation::Determinant, None).unwrap().tsv(), "-4");
        assert_eq!(
            m(1, 1, &[2]).apply(Operation::Inverse, None).unwrap().tsv(),
            "0.5"
        );
    }
    #[test]
    fn singular_overflow_and_invalid_payload_do_not_guess() {
        let a = m(2, 2, &[1, 2, 2, 4]);
        assert_eq!(a.apply(Operation::Determinant, None).unwrap().tsv(), "0");
        assert!(a.apply(Operation::Inverse, None).is_err());
        assert!(a.apply(Operation::Solve, Some(&m(2, 1, &[1, 3]))).is_err());
        assert!(
            m(1, 1, &[i128::MAX])
                .apply(Operation::Multiply, Some(&m(1, 1, &[2])))
                .is_err()
        );
        assert!(Matrix::new(9, 1, vec![Value::Exact(1, 1); 9]).is_err());
        assert!(Matrix::new(1, 1, vec![Value::Approx(f64::NAN)]).is_err());
    }
    #[test]
    fn approximate_solution_retains_marker_and_residual() {
        let a = Matrix::new(
            2,
            2,
            vec![
                Value::Approx(0.1),
                Value::Exact(2, 1),
                Value::Exact(3, 1),
                Value::Exact(4, 1),
            ],
        )
        .unwrap();
        let b = m(2, 1, &[5, 6]);
        let x = a.apply(Operation::Solve, Some(&b)).unwrap();
        assert!(x.approximate());
        let residual = a.apply(Operation::Multiply, Some(&x)).unwrap();
        for (got, want) in residual.cells.iter().zip(b.cells) {
            assert!((got.float() - want.float()).abs() < 1e-12);
        }
        assert!(!x.tsv().contains('≈'));
    }
}
