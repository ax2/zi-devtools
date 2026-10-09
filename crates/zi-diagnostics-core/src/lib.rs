//! Pure imported-report diagnostics. No process, filesystem, network or desktop APIs.
mod config_json;
pub mod contracts;
pub mod django;
pub mod java;
#[cfg(test)]
mod parser_tests;
mod text_parse;
use anyhow::{Result, ensure};
use serde_json::Value;
fn bounded(input: &str) -> Result<()> {
    ensure!(input.len() <= 2 * 1024 * 1024, "输入超过 2 MiB");
    ensure!(input.lines().count() <= 20000, "输入超过 20000 行");
    Ok(())
}
#[derive(Debug)]
pub struct ReportTooLarge;
impl std::fmt::Display for ReportTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("report byte budget")
    }
}
impl std::error::Error for ReportTooLarge {}
fn string_size(s: &str, escaped: bool) -> usize {
    let mut size = if escaped { 4 } else { 2 };
    for byte in s.bytes() {
        size += match byte {
            b'"' | b'\\' => {
                if escaped {
                    4
                } else {
                    2
                }
            }
            b'\n' | b'\r' | b'\t' | 8 | 12 => {
                if escaped {
                    3
                } else {
                    2
                }
            }
            0..=31 => {
                if escaped {
                    7
                } else {
                    6
                }
            }
            _ => 1,
        };
    }
    size
}
fn measure(
    value: &Value,
    depth: usize,
    escaped: bool,
    total: &mut usize,
    limit: usize,
) -> Result<()> {
    let newline = if escaped { 2 } else { 1 };
    let count = match value {
        Value::Null => 4,
        Value::Bool(b) => {
            if *b {
                4
            } else {
                5
            }
        }
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => string_size(s, escaped),
        Value::Array(items) => {
            if items.is_empty() {
                2
            } else {
                *total += 2
                    + newline * (items.len() + 1)
                    + (items.len() - 1)
                    + (depth + 1) * 2 * items.len()
                    + depth * 2;
                for item in items {
                    measure(item, depth + 1, escaped, total, limit)?;
                }
                0
            }
        }
        Value::Object(items) => {
            if items.is_empty() {
                2
            } else {
                *total += 2
                    + newline * (items.len() + 1)
                    + (items.len() - 1)
                    + (depth + 1) * 2 * items.len()
                    + depth * 2;
                for (key, item) in items {
                    *total += string_size(key, escaped) + 2;
                    measure(item, depth + 1, escaped, total, limit)?;
                }
                0
            }
        }
    };
    *total += count;
    if *total > limit {
        return Err(ReportTooLarge.into());
    }
    Ok(())
}
fn report_limited(value: Value, limit: usize) -> Result<String> {
    let mut total = 0;
    measure(&value, 0, limit == 48 * 1024, &mut total, limit)?;
    Ok(serde_json::to_string_pretty(&value)?)
}
struct CollectionBudget {
    limit: usize,
    charge: usize,
}
impl CollectionBudget {
    fn new(limit: usize) -> Self {
        Self { limit, charge: 0 }
    }
    fn add(&mut self, value: &Value) -> Result<()> {
        if self.limit == 48 * 1024 {
            measure(value, 2, true, &mut self.charge, self.limit)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct Action {
    pub id: &'static str,
    pub source_tool_id: &'static str,
    pub scope: &'static str,
}
pub const ACTIONS: &[Action] = &[
    Action {
        id: "java.threads",
        source_tool_id: "java-thread-dump",
        scope: "导入jstack/jcmd文本分析，不采集进程",
    },
    Action {
        id: "java.dependencies",
        source_tool_id: "java-dependencies",
        scope: "导入Maven/Gradle依赖文本，不执行构建",
    },
    Action {
        id: "java.gc",
        source_tool_id: "java-gc-log",
        scope: "导入GC文本分析，不读取磁盘或采集进程",
    },
    Action {
        id: "java.jfr_json",
        source_tool_id: "java-jfr",
        scope: "导入JFR JSON，不读取二进制JFR或启动JDK",
    },
    Action {
        id: "spring.config",
        source_tool_id: "spring-config",
        scope: "导入两份JSON/YAML配置差异/脱敏，不请求Actuator",
    },
    Action {
        id: "django.migrations",
        source_tool_id: "django-migrations",
        scope: "导入迁移计划及SQL分析，不访问数据库或执行迁移",
    },
    Action {
        id: "django.sql",
        source_tool_id: "django-sql",
        scope: "导入SQL查询日志JSON数组归一化/重复分析，不执行SQL",
    },
    Action {
        id: "django.urls",
        source_tool_id: "django-urls",
        scope: "导入URL列表及检索，不加载Django项目",
    },
    Action {
        id: "django.openapi",
        source_tool_id: "django-drf",
        scope: "导入两份OpenAPI JSON差异，不调用远程API",
    },
    Action {
        id: "django.checks",
        source_tool_id: "django-checks",
        scope: "导入检查输出，不运行manage.py",
    },
    Action {
        id: "celery.report",
        source_tool_id: "celery-diagnostics",
        scope: "导入Celery任务日志文本分析，不连接Broker",
    },
];
pub fn execute(id: &str, input: &str, secondary: &str) -> Result<String> {
    bounded(input)?;
    bounded(secondary)?;
    match id {
        "java.threads" => java::threads(input),
        "java.dependencies" => java::dependencies(input),
        "java.gc" => java::gc(input),
        "java.jfr_json" => java::jfr(input),
        "spring.config" => contracts::config(input, secondary),
        "django.migrations" => django::migrations(input, secondary),
        "django.sql" => django::sql(input),
        "django.urls" => django::urls(input, secondary),
        "django.openapi" => contracts::openapi(input, secondary),
        "django.checks" => django::checks(input),
        "celery.report" => django::celery(input),
        _ => anyhow::bail!("未知报告分析能力"),
    }
}
pub fn execute_plugin(
    id: &str,
    input: &str,
    secondary: &str,
    report_limit: usize,
) -> Result<String> {
    bounded(input)?;
    bounded(secondary)?;
    match id {
        "java.threads" => java::threads_limited(input, report_limit),
        "java.dependencies" => java::dependencies_limited(input, report_limit),
        "java.gc" => java::gc_limited(input, report_limit),
        "java.jfr_json" => java::jfr_limited(input, report_limit),
        "spring.config" => contracts::config_limited(input, secondary, report_limit),
        "django.migrations" => django::migrations_limited(input, secondary, report_limit),
        "django.sql" => django::sql_limited(input, report_limit),
        "django.urls" => django::urls_limited(input, secondary, report_limit),
        "django.openapi" => contracts::openapi_limited(input, secondary, report_limit),
        "django.checks" => django::checks_limited(input, report_limit),
        "celery.report" => django::celery_limited(input, report_limit),
        _ => anyhow::bail!("未知报告分析能力"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_is_unique_and_input_budgets_apply_before_dispatch() {
        let mut ids = std::collections::BTreeSet::new();
        for action in ACTIONS {
            assert!(ids.insert(action.id));
            assert!(!action.source_tool_id.is_empty() && !action.scope.is_empty());
            assert!(execute(action.id, &"x".repeat(2 * 1024 * 1024 + 1), "").is_err());
        }
        assert_eq!(ids.len(), 11);
        assert!(execute("unknown", "", "").is_err());
        assert!(execute("django.sql", &"\n".repeat(20001), "").is_err());
    }
    #[test]
    fn report_actions_keep_evidence_and_explicit_dependency_edges() {
        let threads = java::threads("\"worker\" #1 nid=0x1\n java.lang.Thread.State: RUNNABLE\n at app.Main.run(Main.java:1)").unwrap();
        assert!(threads.contains("worker") && threads.contains("RUNNABLE"));
        let migration = django::migrations("[ ] a.0001\n[X] a.0002 ... (a.0001)", "").unwrap();
        assert!(migration.contains("a.0001") && migration.contains("a.0002"));
        assert!(java::jfr("not JSON").is_err());
    }
}
