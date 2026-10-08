use super::{bounded, report};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn threads(input: &str) -> Result<String> {
    bounded(input)?;
    let header = Regex::new(r#"^"([^"]+)".*(?:#\d+|tid=|nid=)"#)?;
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
        if let Some(c) = header.captures(line) {
            ensure!(rows.len() < 4096, "线程超过 4096 项");
            rows.push(json!({"index":rows.len(),"name":&c[1],"line":i+1,"state":"UNKNOWN","held":[],"wait":[],"releasedForWait":[],"frames":[]}));
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
    report(
        json!({"threadCount":rows.len(),"states":states,"explicitJvmDeadlockReport":explicit_deadlock,"cycleThreadIndices":cyclic,"waitEdges":edges,"threads":rows,"notes":"仅单次标准平台线程文本采样。等待环是排查线索；仅 explicitJvmDeadlockReport 表示原文 JVM 明确报告。未解析 ownable synchronizers 的所有格式，不支持虚拟线程 JSON，不由 WAITING 推断线程池耗尽。"}),
    )
}

pub fn dependencies(input: &str) -> Result<String> {
    bounded(input)?;
    let tree = Regex::new(
        r"^(?P<prefix>[ |+\\:\-]*)(?P<coord>[A-Za-z0-9_.-]+:[A-Za-z0-9_.-]+:[^\s]+)(?P<tail>.*)$",
    )?;
    let omitted = Regex::new(r"omitted for conflict with ([^ )]+)")?;
    let mut entries = vec![];
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
        let Some(c) = tree.captures(&cleaned) else {
            if !line.trim().is_empty() {
                skipped += 1;
            }
            continue;
        };
        ensure!(entries.len() < 10000, "依赖条目超过 10000 项");
        let pieces: Vec<_> = c["coord"].trim_end_matches(')').split(':').collect();
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
        let tail = &c["tail"];
        let selected = if let Some((_, after)) = tail.split_once(" -> ") {
            after.split_whitespace().next().unwrap_or(requested)
        } else if let Some(o) = omitted.captures(tail) {
            o.get(1).unwrap().as_str()
        } else {
            requested
        };
        let depth = if maven {
            c["prefix"].len() / 3
        } else {
            c["prefix"].len() / 5
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
    report(
        json!({"entries":entries,"multipleSelectedVersions":conflicts,"unparsedLines":skipped,"notes":"支持 Maven 标准坐标/树和 Gradle module:artifact:version、-> 版本选择；dependencyInsight 仅提取坐标行，反向路径不重建。不同 configuration/scope 中的版本差异不等于运行时冲突；约束/重复子树保留标记，不执行 wrapper。"}),
    )
}

pub fn gc(input: &str) -> Result<String> {
    bounded(input)?;
    let event = Regex::new(r"GC\((\d+)\).*?Pause (.+?)\s+(\d+(?:\.\d+)?)(ms|s)\s*$")?;
    let uptime = Regex::new(r"\[(\d+(?:\.\d+)?)s\]")?;
    let heap = Regex::new(r"(\d+)([BKMG])->(\d+)([BKMG])\((\d+)([BKMG])\)")?;
    let mut pauses = vec![];
    let mut times = vec![];
    let mut collectors = BTreeSet::new();
    for (i, line) in input.lines().enumerate() {
        if let Some((_, name)) = line.split_once("Using ") {
            collectors.insert(name.trim().to_owned());
        }
        if let Some(c) = event.captures(line) {
            let ms = c[3].parse::<f64>()? * if &c[4] == "s" { 1000.0 } else { 1.0 };
            ensure!(ms.is_finite(), "暂停时间超出范围");
            times.push(ms);
            let mut row = json!({"gcId":&c[1],"kind":&c[2],"pauseMs":ms,"line":i+1,"uptimeSeconds":uptime.captures(line).and_then(|t|t[1].parse::<f64>().ok())});
            if let Some(h) = heap.captures(line) {
                row["heap"] = json!({"before":format!("{}{}",&h[1],&h[2]),"after":format!("{}{}",&h[3],&h[4]),"capacity":format!("{}{}",&h[5],&h[6])});
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
    report(
        json!({"collectors":collectors,"pauseCount":times.len(),"totalPauseMs":times.iter().sum::<f64>(),"maxPauseMs":times.last(),"p95PauseMs":times[(times.len()*95).div_ceil(100)-1],"sampleSpanSeconds":span,"pausesPerMinute":span.filter(|s|*s>0.0).map(|s|times.len() as f64*60.0/s),"timeline":pauses,"notes":"仅统计识别到的 Pause 完成行；频率基于首末暂停时间，不代表完整进程时长。不含并发阶段 CPU 开销，不解释 ZGC/Shenandoah 特有事件。"}),
    )
}

fn duration_seconds(s: &str) -> Option<f64> {
    let re =
        Regex::new(r"^PT(?:(\d+(?:\.\d+)?)H)?(?:(\d+(?:\.\d+)?)M)?(?:(\d+(?:\.\d+)?)S)?$").ok()?;
    let c = re.captures(s)?;
    Some(
        [(1, 3600.0), (2, 60.0), (3, 1.0)]
            .into_iter()
            .filter_map(|(i, w)| c.get(i)?.as_str().parse::<f64>().ok().map(|v| v * w))
            .sum(),
    )
}
pub fn jfr(input: &str) -> Result<String> {
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
    report(
        json!({"eventCount":events.len(),"counts":counts,"sampleTopFrames":hot.into_iter().take(30).map(|(name,count)|json!({"method":name,"samples":count})).collect::<Vec<_>>(),"timeline":timeline,"notes":"支持 JDK jfr print --json 事件结构；热点为 ExecutionSample 顶层帧样本数，不是 CPU 百分比。没有完整调用树或火焰图，不替代 JMC。"}),
    )
}
