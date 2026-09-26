//! Opt-in verification against a disposable fixture folder and explicitly selected runtimes.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, path::PathBuf};
use zi_devtools::framework::{Tool, analyze, runtime};
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    ensure!(
        a.len() == 6,
        "usage: framework_verify fixture-dir java.exe python.exe comparison-python.exe jfr.exe"
    );
    let root = PathBuf::from(&a[1]);
    let mut reports = vec![
        ("java-environment", runtime::java_environment(&a[2], &a[2])?),
        (
            "django-environment",
            runtime::python_environment(&a[3], &a[4])?,
        ),
        (
            "java-jfr",
            runtime::recording(&a[5], root.join("fixture.jfr").to_str().context("path")?)?,
        ),
    ];
    for (tool, file, second) in [
        (Tool::Threads, "threads.txt", None),
        (Tool::Gc, "gc.log", None),
        (
            Tool::Migrations,
            "migrations-plan.txt",
            Some("migration.sql"),
        ),
        (Tool::Sql, "queries.json", None),
        (Tool::Urls, "urls.json", None),
        (Tool::Checks, "checks.txt", None),
    ] {
        reports.push((
            tool.id(),
            analyze(
                tool,
                &fs::read_to_string(root.join(file))?,
                &second
                    .map(|p| fs::read_to_string(root.join(p)))
                    .transpose()?
                    .unwrap_or_default(),
            )?,
        ));
    }
    for (id, text) in reports {
        let v: Value = serde_json::from_str(&text)?;
        match id {
            "java-thread-dump" => {
                ensure!(v["explicitJvmDeadlockReport"] == true, "deadlock evidence");
                ensure!(
                    v["cycleThreadIndices"].as_array().unwrap().len() == 2,
                    "wait cycle"
                );
            }
            "java-jfr" => ensure!(v["eventCount"].as_u64().unwrap() > 0, "JFR events"),
            "django-migrations" => ensure!(
                !v["dependencies"].as_array().unwrap().is_empty(),
                "explicit migration dependencies"
            ),
            "django-environment" => {
                ensure!(!v["selected"]["django"].is_null(), "selected Django");
                ensure!(
                    v["comparison"]["django"].is_null(),
                    "comparison without Django"
                );
            }
            "django-sql" => ensure!(
                v["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["count"].as_u64().unwrap() >= 3),
                "repeated queries"
            ),
            _ => {}
        }
        fs::write(root.join(format!("report-{id}.json")), text)?;
        println!("PASS real fixture: {id}");
    }
    Ok(())
}
