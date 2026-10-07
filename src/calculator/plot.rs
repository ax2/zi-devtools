//! Bounded, cancellable function sampling; no assignment or external execution.
use super::{Angle, Value, parser::Prepared};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) const MAX_POINTS: usize = 2049;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Curve {
    pub expression: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub curves: Vec<Curve>,
    pub x_min: f64,
    pub x_max: f64,
    pub points: usize,
    pub auto_y: bool,
    pub y_min: f64,
    pub y_max: f64,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            curves: vec![Curve {
                expression: "sin(x)".into(),
                enabled: true,
            }],
            x_min: -10.0,
            x_max: 10.0,
            points: 513,
            auto_y: true,
            y_min: -2.0,
            y_max: 2.0,
        }
    }
}
pub(super) fn valid_range(min: f64, max: f64) -> bool {
    min.is_finite()
        && max.is_finite()
        && min.abs() <= 1e12
        && max.abs() <= 1e12
        && max - min >= 1e-12
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=4).contains(&self.curves.len())
            || self.curves.iter().any(|c| c.expression.len() > 2048)
        {
            return Err("最多4条函数，每条公式最多2048字节".into());
        }
        if !(17..=MAX_POINTS).contains(&self.points)
            || !valid_range(self.x_min, self.x_max)
            || !valid_range(self.y_min, self.y_max)
        {
            return Err("范围须递增且有限（±1e12，跨度至少1e-12）；每条17–2049个采样点".into());
        }
        let step = (self.x_max - self.x_min) / (self.points - 1) as f64;
        if self.x_min + step <= self.x_min || self.x_max - step >= self.x_max {
            return Err("范围过窄，f64采样坐标无法区分；请扩大范围或减少采样点".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Key {
    pub curves: Vec<Curve>,
    pub x_min: f64,
    pub x_max: f64,
    pub points: usize,
    pub variables: BTreeMap<String, Value>,
    pub degrees: bool,
}
impl Key {
    pub fn new(saved: &Saved, variables: &BTreeMap<String, Value>, degrees: bool) -> Self {
        Self {
            curves: saved.curves.clone(),
            x_min: saved.x_min,
            x_max: saved.x_max,
            points: saved.points,
            variables: variables.clone(),
            degrees,
        }
    }
}
pub(super) struct Series {
    pub index: usize,
    pub expression: String,
    pub y: Vec<Option<f64>>,
    /// An interval rejected by midpoint domain/jump probing is never bridged.
    pub connect: Vec<bool>,
    pub invalid: usize,
    pub diagnostic: String,
}
pub(super) struct Output {
    pub key: Key,
    pub x: Vec<f64>,
    pub series: Vec<Series>,
    pub csv: String,
}
impl Output {
    pub fn y_range(&self) -> Option<(f64, f64)> {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for y in self.series.iter().flat_map(|s| s.y.iter().flatten()) {
            min = min.min(*y);
            max = max.max(*y);
        }
        if !min.is_finite() || !max.is_finite() || min.abs().max(max.abs()) > 1e12 {
            return None;
        }
        let padding = ((max - min) * 0.08)
            .max(min.abs().max(max.abs()) * 0.02)
            .max(1e-9);
        Some(((min - padding).max(-1e12), (max + padding).min(1e12)))
    }
}
pub(super) fn sample(key: Key, cancel: &AtomicBool) -> Result<Output, String> {
    let saved = Saved {
        curves: key.curves.clone(),
        x_min: key.x_min,
        x_max: key.x_max,
        points: key.points,
        ..Default::default()
    };
    saved.validate()?;
    if !key.curves.iter().any(|c| c.enabled) {
        return Err("请至少启用一条函数".into());
    }
    let angle = if key.degrees {
        Angle::Degrees
    } else {
        Angle::Radians
    };
    let mut vars = key.variables.clone();
    let x: Vec<_> = (0..key.points)
        .map(|i| key.x_min + (key.x_max - key.x_min) * (i as f64 / (key.points - 1) as f64))
        .collect();
    let mut series = Vec::new();
    for (index, curve) in key.curves.iter().enumerate().filter(|(_, c)| c.enabled) {
        let prepared =
            Prepared::new(&curve.expression).map_err(|e| format!("y{}：{e}", index + 1))?;
        let mut diagnostic = String::new();
        let mut evaluate = |at: f64| -> Option<f64> {
            vars.insert("x".into(), Value::Approx(at));
            match prepared.evaluate(&vars, angle) {
                Ok(v) if v.float().is_finite() => Some(v.float()),
                Ok(_) => {
                    diagnostic = "非有限结果".into();
                    None
                }
                Err(e) => {
                    diagnostic = e;
                    None
                }
            }
        };
        let mut y = Vec::with_capacity(x.len());
        for at in &x {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消绘制；旧图仍保留".into());
            }
            y.push(evaluate(*at));
        }
        let mut connect = Vec::with_capacity(x.len() - 1);
        for i in 0..x.len() - 1 {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消绘制；旧图仍保留".into());
            }
            let mid = evaluate(x[i] + (x[i + 1] - x[i]) / 2.0);
            let continuous = match (y[i], y[i + 1], mid) {
                (Some(a), Some(b), Some(m)) => {
                    let scale = a.abs().max(b.abs()).max(1e-100);
                    // Conservative jump probe, not a mathematical continuity proof.
                    (m / scale - (a / scale + b / scale) / 2.0).abs() <= 2.0
                        && !((a.is_sign_positive() != b.is_sign_positive())
                            && m.abs() > scale * 2.0)
                }
                _ => false,
            };
            connect.push(continuous);
        }
        let invalid = y.iter().filter(|v| v.is_none()).count();
        if invalid == y.len() {
            return Err(format!("y{} 在该范围无有效值：{diagnostic}", index + 1));
        }
        series.push(Series {
            index,
            expression: curve.expression.clone(),
            y,
            connect,
            invalid,
            diagnostic,
        });
    }
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(
            std::iter::once("x".to_owned())
                .chain(series.iter().map(|s| format!("y{}", s.index + 1))),
        )
        .map_err(|e| e.to_string())?;
    for (i, at) in x.iter().enumerate() {
        writer
            .write_record(
                std::iter::once(at.to_string()).chain(
                    series
                        .iter()
                        .map(|s| s.y[i].map_or_else(String::new, |y| y.to_string())),
                ),
            )
            .map_err(|e| e.to_string())?;
    }
    let csv = String::from_utf8(writer.into_inner().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if csv.len() > 1024 * 1024 {
        return Err("采样CSV超过1MiB，请减少采样点".into());
    }
    Ok(Output {
        key,
        x,
        series,
        csv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(expression: &str) -> Result<Output, String> {
        let saved = Saved {
            curves: vec![Curve {
                expression: expression.into(),
                enabled: true,
            }],
            x_min: -1.0,
            x_max: 1.0,
            points: 17,
            ..Default::default()
        };
        sample(
            Key::new(&saved, &BTreeMap::new(), false),
            &AtomicBool::new(false),
        )
    }
    #[test]
    fn domains_and_off_grid_poles_do_not_bridge_invalid_intervals() {
        let reciprocal = run("1/x").unwrap();
        assert!(reciprocal.series[0].y[8].is_none());
        assert!(!reciprocal.series[0].connect[7] && !reciprocal.series[0].connect[8]);
        let shifted = run("1/(x-0.063)").unwrap();
        assert!(!shifted.series[0].connect[8]);
        let root = run("sqrt(x)").unwrap();
        assert_eq!(root.series[0].invalid, 8);
        assert!(root.csv.lines().nth(1).unwrap().ends_with(','));
    }
    #[test]
    fn snapshot_variables_angles_and_unknown_formula_failures() {
        let saved = Saved {
            curves: vec![Curve {
                expression: "factor*sin(x)".into(),
                enabled: true,
            }],
            x_min: 0.0,
            x_max: 90.0,
            points: 17,
            ..Default::default()
        };
        let vars = BTreeMap::from([
            ("factor".into(), Value::Exact(2, 1)),
            ("ans".into(), Value::Exact(7, 1)),
            ("x".into(), Value::Exact(100, 1)),
        ]);
        let output = sample(Key::new(&saved, &vars, true), &AtomicBool::new(false)).unwrap();
        assert_eq!(output.series[0].y[16], Some(2.0));
        assert_eq!(output.key.variables, vars);
        for expression in ["price=3", "missing*x", "sin(x", "x;quit", "", "sqrt(-1)"] {
            assert!(run(expression).is_err(), "{expression}");
        }
        assert!(sample(output.key, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn limits_and_csv_precision() {
        let output = run("x/3").unwrap();
        let mut reader = csv::Reader::from_reader(output.csv.as_bytes());
        for (i, row) in reader.records().enumerate() {
            let row = row.unwrap();
            assert_eq!(
                row[0].parse::<f64>().unwrap().to_bits(),
                output.x[i].to_bits()
            );
            assert_eq!(
                row[1].parse::<f64>().unwrap().to_bits(),
                output.series[0].y[i].unwrap().to_bits()
            );
        }
        let mut saved = Saved {
            points: MAX_POINTS + 1,
            ..Default::default()
        };
        assert!(saved.validate().is_err());
        saved.points = 17;
        saved.x_min = f64::NAN;
        assert!(saved.validate().is_err());
    }
}
