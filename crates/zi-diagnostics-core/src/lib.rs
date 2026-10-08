//! Pure imported-report diagnostics. No process, filesystem, network or desktop APIs.
pub mod contracts;
pub mod django;
pub mod java;
use anyhow::{Result, ensure};
use serde_json::Value;
fn bounded(input: &str) -> Result<()> {
    ensure!(input.len() <= 2 * 1024 * 1024, "输入超过 2 MiB");
    ensure!(input.lines().count() <= 20000, "输入超过 20000 行");
    Ok(())
}
fn report(value: Value) -> Result<String> {
    let text = serde_json::to_string_pretty(&value)?;
    ensure!(
        text.len() <= 8 * 1024 * 1024,
        "报告超过 8 MiB，请缩小输入范围"
    );
    Ok(text)
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
