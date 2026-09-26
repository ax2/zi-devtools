use super::{bounded, report};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn migrations(input: &str, sql: &str) -> Result<String> {
    bounded(input)?;
    bounded(sql)?;
    let row = Regex::new(r"^\s*\[([X ])\]\s+([\w.]+)(?:\s+\.\.\.\s+\(([^)]*)\))?")?;
    let app = Regex::new(r"^[A-Za-z_][\w]*$")?;
    let plan = Regex::new(r"^([\w]+\.[\w]+)$")?;
    let mut current = String::new();
    let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
    let mut edges = vec![];
    let mut warnings = vec![];
    for (i, line) in input.lines().enumerate() {
        if let Some(c) = row.captures(line) {
            let id = if c[2].contains('.') {
                c[2].to_owned()
            } else {
                ensure!(!current.is_empty(), "第 {} 行缺少应用名", i + 1);
                format!("{}.{}", current, &c[2])
            };
            let applied = &c[1] == "X";
            if nodes
                .insert(id.clone(), json!({"id":id,"applied":applied,"line":i+1}))
                .is_some()
            {
                warnings.push(format!("重复迁移条目：{id}"));
            }
            if let Some(deps) = c.get(3) {
                for dep in deps
                    .as_str()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    edges.push((id.clone(), dep.to_owned()));
                }
            }
        } else if app.is_match(line) {
            current = line.into();
        } else if let Some(c) = plan.captures(line) {
            nodes
                .entry(c[1].into())
                .or_insert(json!({"id":&c[1],"applied":null,"line":i+1}));
        }
    }
    ensure!(
        !nodes.is_empty()
            || input.contains("(no migrations)")
            || input.contains("No planned migration operations"),
        "未识别迁移；建议 showmigrations --plan --verbosity 2"
    );
    let mut parents = BTreeSet::new();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (child, parent) in &edges {
        parents.insert(parent.clone());
        graph.entry(child.clone()).or_default().push(parent.clone());
        if !nodes.contains_key(parent) {
            warnings.push(format!(
                "输入未包含依赖 {parent}（由 {child} 引用），可能是局部导出"
            ));
        }
        if nodes[child]["applied"] == true
            && nodes.get(parent).is_some_and(|n| n["applied"] == false)
        {
            warnings.push(format!("已应用 {child} 的依赖 {parent} 标记为未应用"));
        }
    }
    let mut cycle_nodes = BTreeSet::new();
    let mut traversals = 0;
    for start in graph.keys() {
        let mut todo = graph[start].clone();
        let mut seen = BTreeSet::new();
        while let Some(n) = todo.pop() {
            traversals += 1;
            ensure!(traversals <= 200000, "迁移依赖图过于复杂，请缩小范围");
            if n == *start {
                cycle_nodes.insert(start);
                break;
            }
            if seen.insert(n.clone())
                && let Some(next) = graph.get(&n)
            {
                todo.extend(next.clone());
            }
        }
    }
    let mut leaves: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for id in nodes.keys().filter(|id| !parents.contains(*id)) {
        leaves
            .entry(id.split('.').next().unwrap())
            .or_default()
            .push(id);
    }
    let branches: Vec<_> = leaves
        .into_iter()
        .filter(|(_, v)| !edges.is_empty() && v.len() > 1)
        .map(|(app, nodes)| json!({"app":app,"leaves":nodes}))
        .collect();
    let mut risks = vec![];
    let tokens = sql_tokens(sql)?;
    for (i, statement) in tokens.split(|t| t == ";").enumerate() {
        let normalized = statement.join(" ");
        let risk = if normalized.contains("drop table")
            || normalized.contains("drop column")
            || normalized.contains("truncate ")
        {
            Some("破坏性结构/数据操作")
        } else if normalized.starts_with("delete from ") && !normalized.contains(" where ") {
            Some("无 WHERE 的 DELETE")
        } else if normalized.contains("alter table") {
            Some("检查锁表、类型转换与表重写")
        } else {
            None
        };
        if let Some(risk) = risk {
            risks.push(json!({"statementIndex":i,"risk":risk}));
        }
    }
    report(
        json!({"migrations":nodes.values().collect::<Vec<_>>(),"dependencies":edges.iter().map(|(from,to)|json!({"migration":from,"dependsOn":to})).collect::<Vec<_>>(),"cycleNodes":cycle_nodes,"possibleBranches":branches,"warnings":warnings,"sqlRisks":risks,"notes":"依赖只来自 --plan --verbosity 2 明确输出，不由列表顺序推断。缺少依赖数据时叶节点不能证明分支冲突。SQL 按标记化语句提供保守提示，不是完整方言语法分析；不执行迁移、SQL 或 --fake。"}),
    )
}

/// Tokenizes query fingerprints, dropping comments and replacing data literals.
/// Double quoted/backtick identifiers stay identifiers. PostgreSQL dollar strings supported.
pub fn sql_tokens(s: &str) -> Result<Vec<String>> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut tokens = vec![];
    while i < c.len() {
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        if c[i] == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            let mut depth = 1;
            while i < c.len() && depth > 0 {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            ensure!(depth == 0, "SQL 块注释未结束");
            continue;
        }
        if c[i] == '\'' || c[i] == '"' || c[i] == '`' {
            let quote = c[i];
            let start = i;
            i += 1;
            let mut closed = false;
            while i < c.len() {
                if c[i] == quote {
                    i += 1;
                    if c.get(i) == Some(&quote) {
                        i += 1;
                        continue;
                    }
                    closed = true;
                    break;
                }
                if c[i] == '\\' && quote == '\'' && i + 1 < c.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            ensure!(closed, "SQL 引号未结束");
            tokens.push(if quote == '\'' {
                "?".into()
            } else {
                c[start..i].iter().collect()
            });
            continue;
        }
        if c[i] == '$' {
            let start = i;
            let mut j = i + 1;
            while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '_') {
                j += 1;
            }
            if c.get(j) == Some(&'$') {
                let tag = &c[start..=j];
                i = j + 1;
                let mut closed = false;
                while i + tag.len() <= c.len() {
                    if &c[i..i + tag.len()] == tag {
                        i += tag.len();
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                ensure!(closed, "SQL dollar 引号未结束");
                tokens.push("?".into());
                continue;
            }
        }
        if c[i].is_ascii_digit() {
            i += 1;
            while i < c.len()
                && (c[i].is_ascii_alphanumeric()
                    || c[i] == '.'
                    || ((c[i] == '+' || c[i] == '-')
                        && matches!(c.get(i.wrapping_sub(1)), Some('e' | 'E'))))
            {
                i += 1;
            }
            tokens.push("?".into());
            continue;
        }
        if c[i].is_alphabetic() || c[i] == '_' {
            let start = i;
            i += 1;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                i += 1;
            }
            tokens.push(c[start..i].iter().collect::<String>().to_lowercase());
            continue;
        }
        tokens.push(c[i].to_string());
        i += 1;
    }
    Ok(tokens)
}
pub fn sql(input: &str) -> Result<String> {
    bounded(input)?;
    let v: Value = serde_json::from_str(input)?;
    let records = v
        .as_array()
        .context("需要 [{requestId,sql,durationMs}] 数组")?;
    ensure!(records.len() <= 10000, "最多 10000 条查询");
    type QueryGroup = (usize, f64, BTreeSet<String>, Vec<usize>);
    let mut groups: BTreeMap<(Option<String>, String), QueryGroup> = BTreeMap::new();
    let mut total = 0.0;
    for (i, r) in records.iter().enumerate() {
        let query = r["sql"].as_str().context("查询缺少 sql 文本")?;
        let duration = r["durationMs"]
            .as_f64()
            .context("durationMs 必须是毫秒数值")?;
        ensure!(
            duration >= 0.0 && duration.is_finite(),
            "durationMs 必须非负有限值"
        );
        total += duration;
        ensure!(total.is_finite(), "总耗时过大");
        let request = match r.get("requestId") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => anyhow::bail!("requestId 需为字符串或 null"),
        };
        let fingerprint = sql_tokens(query)?.join(" ");
        ensure!(!fingerprint.is_empty(), "SQL 不能为空");
        let g = groups.entry((request, fingerprint)).or_default();
        g.0 += 1;
        g.1 += duration;
        g.2.insert(query.into());
        g.3.push(i);
    }
    let mut rows:Vec<_>=groups.into_iter().map(|((request,fingerprint),(count,ms,variants,indices))| {
        let candidate=count>=3&&fingerprint.starts_with("select ");
        json!({"requestId":request,"fingerprint":fingerprint,"count":count,"totalMs":ms,"meanMs":ms/count as f64,"distinctRawQueries":variants.len(),"recordIndices":indices,"assessment":if candidate {if request.is_some(){"同请求重复 SELECT，疑似 N+1"}else{"跨记录重复 SELECT；缺少请求上下文，无法判断 N+1"}}else{"未触发重复 SELECT 规则"}})
    }).collect();
    rows.sort_by(|a, b| {
        b["totalMs"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["totalMs"].as_f64().unwrap())
    });
    report(
        json!({"queryCount":records.len(),"totalMs":total,"groups":rows,"notes":"仅归一化字面量/注释与空白，不做 SQL 语义分析；不证明 N+1。保留标识符和 requestId；分享前脱敏。耗时总和不等于请求墙钟时间。"}),
    )
}

pub fn urls(input: &str, query: &str) -> Result<String> {
    bounded(input)?;
    bounded(query)?;
    let v: Value = serde_json::from_str(input)?;
    let rows = v.as_array().context("需要 URL 清单数组")?;
    ensure!(rows.len() <= 10000, "URL 超过 10000 项");
    let converter = Regex::new(r"<(?:(\w+):)?(\w+)>")?;
    let mut names: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut unnamed = 0;
    for (i, row) in rows.iter().enumerate() {
        let route = row["route"].as_str().context("URL 缺少 route")?;
        let Some(name) = row["name"].as_str().filter(|n| !n.is_empty()) else {
            unnamed += 1;
            continue;
        };
        let namespace = row["namespace"].as_str().unwrap_or("");
        let full = if namespace.is_empty() {
            name.to_owned()
        } else {
            format!("{namespace}:{name}")
        };
        let parameters: Vec<String> = if let Some(p) = row.get("parameters") {
            p.as_array()
                .context("parameters 应为字符串数组")?
                .iter()
                .map(|v| v.as_str().map(str::to_owned).context("参数名需为字符串"))
                .collect::<Result<_>>()?
        } else {
            converter
                .captures_iter(route)
                .map(|c| c[2].into())
                .collect()
        };
        names.entry(full).or_default().push(json!({"index":i,"route":route,"parameters":parameters,"regexRoute":row["regexRoute"].as_bool().unwrap_or(false)}));
    }
    let duplicates: Vec<_> = names
        .iter()
        .filter(|(_, routes)| routes.len() > 1)
        .map(|(name, routes)| json!({"qualifiedName":name,"routes":routes}))
        .collect();
    let reverse = if query.trim().is_empty() {
        Value::Null
    } else {
        let q: Value = serde_json::from_str(query)?;
        let name = q["name"].as_str().context("查询缺少 name")?;
        let kwargs = q["kwargs"]
            .as_object()
            .context("查询需提供 kwargs 对象，可为空")?;
        let provided: BTreeSet<_> = kwargs.keys().cloned().collect();
        let candidates:Vec<_>=names.get(name).into_iter().flatten().map(|r| {
            let expected:BTreeSet<_>=r["parameters"].as_array().unwrap().iter().map(|p|p.as_str().unwrap().to_owned()).collect();
            json!({"route":r["route"],"missing":expected.difference(&provided).collect::<Vec<_>>(),"extra":provided.difference(&expected).collect::<Vec<_>>(),"parameterNamesMatch":expected==provided})
        }).collect();
        json!({"name":name,"found":!candidates.is_empty(),"candidates":candidates})
    };
    report(
        json!({"routeCount":rows.len(),"unnamedCount":unnamed,"duplicateNames":duplicates,"reverseCheck":reverse,"notes":"只检查导出清单与 kwargs 名称，不实际 reverse、不验证 converter 值/正则/defaults。重复名称可能是合法重载。正则路由应显式导出 parameters，缺失时结果不完整。"}),
    )
}

pub fn checks(input: &str) -> Result<String> {
    bounded(input)?;
    let re = Regex::new(r"\(([A-Za-z_][\w]*\.([EWICD])\d+)\)\s*(.*)")?;
    let mut items = vec![];
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, line) in input.lines().enumerate() {
        if let Some(c) = re.captures(line) {
            let level = match &c[2] {
                "E" => "error",
                "W" => "warning",
                "C" => "critical",
                "I" => "info",
                _ => "debug",
            };
            *counts.entry(level).or_default() += 1;
            let hint = match &c[1] {
                "security.W004" => "核对 HSTS 期限与全链路 HTTPS；先验证部署环境再启用。",
                "security.W008" => "检查 HTTPS 重定向与反向代理协议识别，避免循环重定向。",
                "security.W009" => "使用足够强度的独立密钥；不要将密钥放进报告。",
                "security.W012" | "security.W016" => "确认 HTTPS 后配置 secure Cookie。",
                "security.W018" => "生产环境关闭 DEBUG，并验证错误页不暴露内部信息。",
                "security.W020" => "明确配置允许的 Host，避免通配所有主机。",
                _ => "结合当前 Django 版本官方检查编号文档与原始消息复核。",
            };
            items.push(json!({"id":&c[1],"level":level,"message":&c[3],"line":i+1,"hint":hint}));
        }
    }
    ensure!(
        !items.is_empty() || input.contains("System check identified no issues"),
        "未发现 Django check 编号或明确的无问题报告"
    );
    report(
        json!({"counts":counts,"items":items,"explicitNoIssues":input.contains("System check identified no issues"),"notes":"只阅读输入的 check --deploy 输出，无法证明线上配置已生效；未知编号保留原文，不自动修复。"}),
    )
}

pub fn celery(input: &str) -> Result<String> {
    bounded(input)?;
    let re = Regex::new(
        r"Task ([\w.]+)\[([^\]]+)\] (received|started|succeeded|failed|raised unexpected|retry|retried|revoked)(.*)",
    )?;
    let elapsed = Regex::new(r"in (\d+(?:\.\d+)?)s")?;
    let exception = Regex::new(r"^:\s*([A-Za-z_][\w.]*)\(")?;
    let mut tasks: BTreeMap<String, Value> = BTreeMap::new();
    let mut ignored = 0;
    for (i, line) in input.lines().enumerate() {
        if let Some(c) = re.captures(line) {
            let task=tasks.entry(c[2].into()).or_insert(json!({"id":&c[2],"name":&c[1],"events":[],"failures":0,"retries":0,"lastObservedState":"unknown"}));
            if task["name"] != c[1] {
                anyhow::bail!("同一任务 ID 对应多个名称，输入混合或无效");
            }
            let status = &c[3];
            if status == "failed" || status == "raised unexpected" {
                task["failures"] = json!(task["failures"].as_u64().unwrap() + 1);
            }
            if status == "retry" || status == "retried" {
                task["retries"] = json!(task["retries"].as_u64().unwrap() + 1);
            }
            task["lastObservedState"] = json!(status);
            let seconds = if status == "succeeded" {
                elapsed
                    .captures(&c[4])
                    .and_then(|t| t[1].parse::<f64>().ok())
            } else {
                None
            };
            task["events"]
                .as_array_mut()
                .unwrap()
                .push(json!({"line":i+1,"state":status,"durationSeconds":seconds,
                    "retryDelaySeconds":if status=="retry"||status=="retried" {elapsed.captures(&c[4]).and_then(|t|t[1].parse::<f64>().ok())}else{None},
                    "exceptionType":if status=="failed"||status=="raised unexpected" {exception.captures(&c[4]).map(|e|e[1].to_owned())}else{None}}));
        } else if !line.trim().is_empty() {
            ignored += 1;
        }
    }
    ensure!(
        !tasks.is_empty(),
        "未识别 Celery 标准 Task name[id] 日志；先按时间排序并脱敏"
    );
    report(
        json!({"tasks":tasks.values().collect::<Vec<_>>(),"unparsedLines":ignored,"notes":"按输入顺序关联任务 ID；只支持标准 worker Task 日志。lastObservedState 不是 broker 实时状态，缺失事件无法还原；不输出任务参数/返回值，不连接 broker 或重试任务。"}),
    )
}
