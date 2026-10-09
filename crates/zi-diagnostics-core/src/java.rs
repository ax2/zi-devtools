use super::bounded;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn threads(input: &str) -> Result<String> {
    threads_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn threads_limited(input: &str, report_limit: usize) -> Result<String> {
    bounded(input)?;
    let state = Regex::new(r"java.lang.Thread.State:\s+([A-Z_]+)")?;
    let lock = Regex::new(
        r"- (locked|waiting on|waiting to lock|parking to wait for)\s+<(0x[0-9a-fA-F]+)>",
    )?;
    let mut rows: Vec<Value> = vec![];
    let mut explicit_deadlock = false;
    for (i, line) in input.lines().enumerate() {
        if line.contains("Found one Java-level deadlock") || line.contains("Found 1 deadlock") {
            explicit_deadlock = true;
        }
        if line.contains("Found one Java-level deadlock") || line.contains("Found one Java-level") {
            break;
        }
        if let Some(name) = thread_name(line) {
            ensure!(rows.len() < 4096, "线程超过 4096 项");
            // Each mandatory thread object needs more than 120 UTF-8 bytes in
            // the pretty report, even with one-byte name/state and empty arrays.
            // This is a lower bound, never a truncated successful report.
            if (rows.len() + 1).saturating_mul(120) > report_limit {
                return Err(super::ReportTooLarge.into());
            }
            rows.push(json!({"index":rows.len(),"name":name,"line":i+1,"state":"UNKNOWN","held":[],"wait":[],"releasedForWait":[],"frames":[]}));
        } else if let Some(row) = rows.last_mut() {
            if let Some(c) = state.captures(line) {
                row["state"] = json!(&c[1]);
            }
            if let Some(c) = lock.captures(line) {
                let key = match &c[1] {
                    "locked" => "held",
                    "waiting on" => "releasedForWait",
                    _ => "wait",
                };
                row[key]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"lock":&c[2],"kind":&c[1],"line":i+1}));
            }
            if line.trim_start().starts_with("at ") {
                row["frames"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(line.trim()));
            }
        }
    }
    ensure!(
        !rows.is_empty(),
        "未发现标准 jstack / jcmd Thread.print 线程头；不支持 JSON 虚拟线程转储"
    );
    let mut owners: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut states: BTreeMap<String, usize> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        *states
            .entry(row["state"].as_str().unwrap().into())
            .or_default() += 1;
        for held in row["held"].as_array().unwrap() {
            if row["releasedForWait"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["lock"] == held["lock"])
            {
                continue;
            }
            owners
                .entry(held["lock"].as_str().unwrap().into())
                .or_default()
                .push(i);
        }
    }
    let mut edges = vec![];
    let mut adjacency: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        for wait in row["wait"].as_array().unwrap() {
            let lock = wait["lock"].as_str().unwrap();
            let targets: BTreeSet<usize> = owners
                .get(lock)
                .into_iter()
                .flatten()
                .copied()
                .filter(|j| *j != i)
                .collect();
            for j in targets {
                ensure!(edges.len() < 10000, "锁等待边超过 10000 项，请缩小范围");
                edges.push(json!({"waiter":i,"owner":j,"lock":lock,"line":wait["line"]}));
                adjacency.entry(i).or_default().push(j);
            }
        }
    }
    // Iterative reachability avoids stack overflow on large synthetic dumps.
    let mut cyclic = BTreeSet::new();
    let mut traversals = 0;
    for start in adjacency.keys() {
        let mut pending = adjacency[start].clone();
        let mut seen = BTreeSet::new();
        while let Some(node) = pending.pop() {
            traversals += 1;
            ensure!(traversals <= 200000, "锁等待图过于复杂，请缩小范围");
            if node == *start {
                cyclic.insert(*start);
                break;
            }
            if seen.insert(node)
                && let Some(next) = adjacency.get(&node)
            {
                pending.extend(next);
            }
        }
    }
    super::report_limited(
        json!({"threadCount":rows.len(),"states":states,"explicitJvmDeadlockReport":explicit_deadlock,"cycleThreadIndices":cyclic,"waitEdges":edges,"threads":rows,"notes":"仅单次标准平台线程文本采样。等待环是排查线索；仅 explicitJvmDeadlockReport 表示原文 JVM 明确报告。未解析 ownable synchronizers 的所有格式，不支持虚拟线程 JSON，不由 WAITING 推断线程池耗尽。"}),
        report_limit,
    )
}
fn thread_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('"')?;
    let (name, tail) = rest.split_once('"')?;
    if name.is_empty() {
        return None;
    }
    let numbered = tail.match_indices('#').any(|(at, _)| {
        tail[at + 1..]
            .chars()
            .next()
            .is_some_and(super::text_parse::digit)
    });
    (numbered || tail.contains("tid=") || tail.contains("nid=")).then_some(name)
}

pub fn dependencies(input: &str) -> Result<String> {
    dependencies_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn dependencies_limited(input: &str, report_limit: usize) -> Result<String> {
    bounded(input)?;
    let omitted = Regex::new(r"omitted for conflict with ([^ )]+)")?;
    let mut entries = vec![];
    let mut output_budget = super::CollectionBudget::new(report_limit);
    let mut stack: Vec<(usize, String)> = vec![];
    let mut versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut skipped = 0;
    let mut path_segments = 0;
    for (i, original) in input.lines().enumerate() {
        let line = original
            .strip_prefix("[INFO] ")
            .unwrap_or(original)
            .trim_end();
        // Maven verbose omitted lines wrap the coordinate in parentheses.
        let cleaned = line.replace("+- (", "+- ").replace("\\- (", "\\- ");
        let Some((prefix, coordinate, tail)) = dependency_row(&cleaned) else {
            if !line.trim().is_empty() {
                skipped += 1;
            }
            continue;
        };
        ensure!(entries.len() < 10000, "依赖条目超过 10000 项");
        let pieces: Vec<_> = coordinate.trim_end_matches(')').split(':').collect();
        if pieces.len() < 3 {
            continue;
        }
        let key = format!("{}:{}", pieces[0], pieces[1]);
        let maven = pieces.len() >= 4;
        let requested = if maven {
            let last = pieces[pieces.len() - 1];
            if ["compile", "runtime", "test", "provided", "system", "import"].contains(&last) {
                pieces[pieces.len() - 2]
            } else {
                last
            }
        } else {
            pieces[2]
        };
        let selected = if let Some((_, after)) = tail.split_once(" -> ") {
            after.split_whitespace().next().unwrap_or(requested)
        } else if let Some(o) = omitted.captures(tail) {
            o.get(1).unwrap().as_str()
        } else {
            requested
        };
        let depth = if maven {
            prefix.len() / 3
        } else {
            prefix.len() / 5
        };
        while stack.last().is_some_and(|(d, _)| *d >= depth) {
            stack.pop();
        }
        let mut path: Vec<_> = stack.iter().map(|(_, k)| k.clone()).collect();
        path.push(key.clone());
        path_segments += path.len();
        ensure!(path_segments <= 100000, "依赖路径过多，请按模块导出");
        stack.push((depth, key.clone()));
        versions
            .entry(key.clone())
            .or_default()
            .insert(selected.to_owned());
        entries.push(json!({"module":key,"requested":requested,"selected":selected,"selectionChanged":requested!=selected,"constraint":tail.contains("(c)"),"repeatedSubtree":tail.contains("(*)"),"unresolved":tail.contains("FAILED"),"path":path,"line":i+1}));
        output_budget.add(entries.last().unwrap())?;
    }
    ensure!(
        !entries.is_empty(),
        "未发现 Maven/Gradle 坐标；请导入 dependency:tree / dependencies / dependencyInsight 文本"
    );
    let conflicts: Vec<_> = versions
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, v)| json!({"module":k,"versions":v}))
        .collect();
    super::report_limited(
        json!({"entries":entries,"multipleSelectedVersions":conflicts,"unparsedLines":skipped,"notes":"支持 Maven 标准坐标/树和 Gradle module:artifact:version、-> 版本选择；dependencyInsight 仅提取坐标行，反向路径不重建。不同 configuration/scope 中的版本差异不等于运行时冲突；约束/重复子树保留标记，不执行 wrapper。"}),
        report_limit,
    )
}
fn dependency_row(line: &str) -> Option<(&str, &str, &str)> {
    let prefix_end = line
        .find(|c| !matches!(c, ' ' | '|' | '+' | '\\' | ':' | '-'))
        .unwrap_or(line.len());
    for start in (0..=prefix_end).rev() {
        let rest = &line[start..];
        let coordinate_part = |s: &str| {
            s.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
                .unwrap_or(s.len())
        };
        let group = coordinate_part(rest);
        if group == 0 {
            continue;
        }
        let Some(artifact) = rest[group..].strip_prefix(':') else {
            continue;
        };
        let name = coordinate_part(artifact);
        if name == 0 {
            continue;
        }
        let Some(version) = artifact[name..].strip_prefix(':') else {
            continue;
        };
        let end = version.find(char::is_whitespace).unwrap_or(version.len());
        if end == 0 {
            continue;
        }
        let length = group + name + 2 + end;
        return Some((&line[..start], &rest[..length], &rest[length..]));
    }
    None
}

pub fn gc(input: &str) -> Result<String> {
    gc_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn gc_limited(input: &str, report_limit: usize) -> Result<String> {
    bounded(input)?;
    let mut pauses = vec![];
    let mut times = vec![];
    let mut collectors = BTreeSet::new();
    for (i, line) in input.lines().enumerate() {
        if let Some((_, name)) = line.split_once("Using ") {
            collectors.insert(name.trim().to_owned());
        }
        if let Some((id, kind, duration, unit)) = gc_event(line) {
            let ms = duration.parse::<f64>()? * if unit == "s" { 1000.0 } else { 1.0 };
            ensure!(ms.is_finite(), "暂停时间超出范围");
            times.push(ms);
            let mut row = json!({"gcId":id,"kind":kind,"pauseMs":ms,"line":i+1,"uptimeSeconds":gc_uptime(line)});
            if let Some((before, after, capacity)) = gc_heap(line) {
                row["heap"] = json!({"before":before,"after":after,"capacity":capacity});
            }
            pauses.push(row);
        }
    }
    ensure!(
        !pauses.is_empty(),
        "未发现支持的统一 GC 日志 Pause 完成行；支持 JDK 11–21 G1/Parallel/Serial 常规格式"
    );
    times.sort_by(f64::total_cmp);
    let timestamps: Vec<f64> = pauses
        .iter()
        .filter_map(|p| p["uptimeSeconds"].as_f64())
        .collect();
    let span = timestamps
        .iter()
        .copied()
        .reduce(f64::max)
        .zip(timestamps.iter().copied().reduce(f64::min))
        .map(|(max, min)| max - min);
    super::report_limited(
        json!({"collectors":collectors,"pauseCount":times.len(),"totalPauseMs":times.iter().sum::<f64>(),"maxPauseMs":times.last(),"p95PauseMs":times[(times.len()*95).div_ceil(100)-1],"sampleSpanSeconds":span,"pausesPerMinute":span.filter(|s|*s>0.0).map(|s|times.len() as f64*60.0/s),"timeline":pauses,"notes":"仅统计识别到的 Pause 完成行；频率基于首末暂停时间，不代表完整进程时长。不含并发阶段 CPU 开销，不解释 ZGC/Shenandoah 特有事件。"}),
        report_limit,
    )
}
fn gc_uptime(line: &str) -> Option<f64> {
    for (at, _) in line.match_indices('[') {
        if let Some((number, tail)) = super::text_parse::decimal(&line[at + 1..])
            && tail.starts_with("s]")
        {
            return number.parse().ok();
        }
    }
    None
}
fn gc_heap(line: &str) -> Option<(&str, &str, &str)> {
    fn size(s: &str) -> Option<(&str, &str)> {
        let digits = s.find(|c| !super::text_parse::digit(c)).unwrap_or(s.len());
        if digits == 0 || !s[digits..].starts_with(['B', 'K', 'M', 'G']) {
            return None;
        }
        Some((&s[..digits + 1], &s[digits + 1..]))
    }
    for (at, c) in line.char_indices() {
        if !super::text_parse::digit(c) {
            continue;
        }
        if let Some((before, rest)) = size(&line[at..])
            && let Some(rest) = rest.strip_prefix("->")
            && let Some((after, rest)) = size(rest)
            && let Some(rest) = rest.strip_prefix('(')
            && let Some((capacity, rest)) = size(rest)
            && rest.starts_with(')')
        {
            return Some((before, after, capacity));
        }
    }
    None
}
fn gc_event(line: &str) -> Option<(&str, &str, &str, &str)> {
    for (at, _) in line.match_indices("GC(") {
        let rest = &line[at + 3..];
        let end = rest
            .find(|c| !super::text_parse::digit(c))
            .unwrap_or(rest.len());
        if end == 0 {
            continue;
        }
        let Some(tail) = rest[end..].strip_prefix(')') else {
            continue;
        };
        for (pause, _) in tail.match_indices("Pause ") {
            let body = tail[pause + 6..].trim_end();
            for (offset, c) in body.char_indices() {
                if !c.is_whitespace() {
                    continue;
                }
                let number = body[offset..].trim_start();
                if let Some((duration, unit)) = super::text_parse::decimal(number)
                    && (unit == "ms" || unit == "s")
                    && offset > 0
                {
                    let kind = body[..offset].trim_end();
                    if !kind.is_empty() {
                        return Some((&rest[..end], kind, duration, unit));
                    }
                }
            }
        }
    }
    None
}

fn duration_seconds(s: &str) -> Option<f64> {
    let mut rest = s.strip_prefix("PT")?;
    let mut seconds = 0.0;
    for (suffix, multiplier) in [('H', 3600.0), ('M', 60.0), ('S', 1.0)] {
        if let Some((number, tail)) = super::text_parse::decimal(rest)
            && let Some(next) = tail.strip_prefix(suffix)
        {
            if let Ok(value) = number.parse::<f64>() {
                seconds += value * multiplier;
            }
            rest = next;
        }
    }
    rest.is_empty().then_some(seconds)
}
pub fn jfr(input: &str) -> Result<String> {
    jfr_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn jfr_limited(input: &str, report_limit: usize) -> Result<String> {
    ensure!(input.len() <= 8 * 1024 * 1024, "JFR JSON 超过 8 MiB");
    let v: Value = serde_json::from_str(input).context("需要 jfr print --json 的结果")?;
    let events = v
        .pointer("/recording/events")
        .and_then(Value::as_array)
        .context("缺少 recording.events 数组")?;
    ensure!(
        events.len() <= 20000,
        "事件超过 20000 项；缩小录制或按事件过滤"
    );
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut hotspots: BTreeMap<String, usize> = BTreeMap::new();
    let mut timeline = vec![];
    for (index, e) in events.iter().enumerate() {
        let kind = e["type"].as_str().context("事件缺少 type")?;
        *counts.entry(kind.into()).or_default() += 1;
        if kind == "jdk.ExecutionSample"
            && let Some(method) = e.pointer("/values/stackTrace/frames/0/method")
        {
            let class = method
                .pointer("/type/name")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let name = method["name"].as_str().unwrap_or("?");
            *hotspots.entry(format!("{class}.{name}")).or_default() += 1;
        }
        timeline.push(json!({"index":index,"type":kind,"startTime":e.pointer("/values/startTime"),"durationSeconds":e.pointer("/values/duration").and_then(Value::as_str).and_then(duration_seconds)}));
    }
    let mut hot: Vec<_> = hotspots.into_iter().collect();
    hot.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    super::report_limited(
        json!({"eventCount":events.len(),"counts":counts,"sampleTopFrames":hot.into_iter().take(30).map(|(name,count)|json!({"method":name,"samples":count})).collect::<Vec<_>>(),"timeline":timeline,"notes":"支持 JDK jfr print --json 事件结构；热点为 ExecutionSample 顶层帧样本数，不是 CPU 百分比。没有完整调用树或火焰图，不替代 JMC。"}),
        report_limit,
    )
}
