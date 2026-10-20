use super::bounded;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub fn threads(input: &str) -> Result<String> {
    threads_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn threads_limited(input: &str, report_limit: usize) -> Result<String> {
    bounded(input)?;
    if input.trim_start().starts_with('{') {
        return threads_json_limited(input, report_limit);
    }
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
            if let Some(state) = thread_state(line) {
                row["state"] = json!(state);
            }
            if let Some((kind, lock)) = thread_lock(line) {
                let key = match kind {
                    "locked" => "held",
                    "waiting on" => "releasedForWait",
                    _ => "wait",
                };
                row[key]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"lock":lock,"kind":kind,"line":i+1}));
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
        json!({"sourceFormat":"jstack-text","threadCount":rows.len(),"states":states,"explicitJvmDeadlockReport":explicit_deadlock,"cycleThreadIndices":cyclic,"waitEdges":edges,"threads":rows,"notes":"仅解析可识别的标准 jstack / Thread.print 文本。等待环是排查线索；仅 explicitJvmDeadlockReport 表示原文 JVM 明确报告。未解析 ownable synchronizers 的所有格式，不由 WAITING 推断线程池耗尽。"}),
        report_limit,
    )
}
fn json_scalar_text(value: Option<&Value>, field: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        _ => anyhow::bail!("JSON 字段 {field} 必须是字符串、数字或 null"),
    }
}

fn json_optional_string(value: Option<&Value>, field: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => anyhow::bail!("JSON 字段 {field} 必须是字符串或 null"),
    }
}

fn threads_json_limited(input: &str, report_limit: usize) -> Result<String> {
    let document: Value = serde_json::from_str(input).context("线程转储 JSON 格式无效")?;
    let dump = document
        .get("threadDump")
        .and_then(Value::as_object)
        .context("缺少 JDK threadDump 对象")?;
    let containers = dump
        .get("threadContainers")
        .and_then(Value::as_array)
        .context("缺少 JDK threadContainers 数组")?;
    ensure!(containers.len() <= 4096, "线程容器超过 4096 项");

    let mut rows = Vec::new();
    let mut states: BTreeMap<String, usize> = BTreeMap::new();
    let mut virtual_thread_count = 0usize;
    let mut virtual_type_unknown_count = 0usize;
    for (container_index, container) in containers.iter().enumerate() {
        let container = container
            .as_object()
            .context("threadContainers 中的容器必须是对象")?;
        let container_name = container
            .get("container")
            .and_then(Value::as_str)
            .context("线程容器缺少 container 名称")?;
        let parent = json_scalar_text(container.get("parent"), "parent")?;
        let owner = json_scalar_text(container.get("owner"), "owner")?;
        let threads = container
            .get("threads")
            .and_then(Value::as_array)
            .context("线程容器缺少 threads 数组")?;
        for thread in threads {
            ensure!(rows.len() < 4096, "线程超过 4096 项");
            let thread = thread.as_object().context("线程条目必须是对象")?;
            let name = thread
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .context("线程条目缺少非空 name")?;
            let tid = json_scalar_text(thread.get("tid"), "tid")?.context("线程条目缺少 tid")?;
            let state = match thread.get("state") {
                None => "UNKNOWN",
                Some(Value::String(state)) => state.as_str(),
                Some(_) => anyhow::bail!("线程字段 state 必须是字符串"),
            };
            let virtual_thread = match thread.get("virtual") {
                None => {
                    virtual_type_unknown_count += 1;
                    None
                }
                Some(Value::Bool(value)) => {
                    if *value {
                        virtual_thread_count += 1;
                    }
                    Some(*value)
                }
                Some(_) => anyhow::bail!("线程字段 virtual 必须是布尔值"),
            };
            *states.entry(state.to_owned()).or_default() += 1;
            let frames = match thread.get("stack") {
                None => Vec::new(),
                Some(Value::Array(frames)) => frames
                    .iter()
                    .map(|frame| {
                        frame
                            .as_str()
                            .context("线程 stack 只能包含字符串")
                            .map(str::to_owned)
                    })
                    .collect::<Result<Vec<_>>>()?,
                Some(_) => anyhow::bail!("线程字段 stack 必须是数组"),
            };
            let thread_type = match virtual_thread {
                Some(true) => "virtual",
                Some(false) => "platform",
                None => "unknown",
            };
            let time = match thread.get("time") {
                None => None,
                Some(Value::String(time)) => Some(time.clone()),
                Some(_) => anyhow::bail!("线程字段 time 必须是字符串"),
            };
            rows.push(json!({
                "index": rows.len(),
                "tid": tid,
                "name": name,
                "state": state,
                "virtual": virtual_thread,
                "threadType": thread_type,
                "containerIndex": container_index,
                "container": container_name,
                "containerParent": parent.clone(),
                "containerOwner": owner.clone(),
                "time": time,
                "lockDataAvailable": false,
                "held": [],
                "wait": [],
                "releasedForWait": [],
                "frames": frames
            }));
        }
    }
    ensure!(!rows.is_empty(), "JDK JSON 转储中没有线程条目");

    let metadata = json!({
        "processId": json_scalar_text(dump.get("processId"), "processId")?,
        "time": json_optional_string(dump.get("time"), "time")?,
        "runtimeVersion": json_optional_string(dump.get("runtimeVersion"), "runtimeVersion")?
    });
    super::report_limited(
        json!({
            "sourceFormat": "jcmd-json",
            "threadCount": rows.len(),
            "containerCount": containers.len(),
            "virtualThreadCount": virtual_thread_count,
            "virtualTypeUnknownCount": virtual_type_unknown_count,
            "states": states,
            "explicitJvmDeadlockReport": false,
            "cycleThreadIndices": [],
            "waitEdges": [],
            "metadata": metadata,
            "threads": rows,
            "notes": "已解析 jcmd Thread.dump_to_file JSON 的线程容器、线程状态与栈帧。仅在输入显式提供 virtual 字段时分类线程；缺少该字段的 JDK 版本标记为未提供。JSON 转储不报告 JVM 死锁检测结果，本工具不从线程状态或容器关系推断死锁；锁等待关系仅覆盖支持的文本格式。"
        }),
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
fn thread_state(line: &str) -> Option<&str> {
    // Preserve the old regex's wildcard separators as well as normal dots.
    for (at, _) in line.match_indices("java") {
        let mut rest = &line[at + 4..];
        let mut matched = true;
        for part in ["lang", "Thread", "State:"] {
            let Some(c) = rest.chars().next() else {
                matched = false;
                break;
            };
            if c == '\n' {
                matched = false;
                break;
            }
            rest = &rest[c.len_utf8()..];
            let Some(next) = rest.strip_prefix(part) else {
                matched = false;
                break;
            };
            rest = next;
        }
        if !matched || !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        let end = rest
            .find(|c: char| !(c.is_ascii_uppercase() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}
fn thread_lock(line: &str) -> Option<(&str, &str)> {
    for (at, _) in line.match_indices("- ") {
        let rest = &line[at + 2..];
        for kind in [
            "locked",
            "waiting on",
            "waiting to lock",
            "parking to wait for",
        ] {
            let Some(tail) = rest.strip_prefix(kind) else {
                continue;
            };
            if !tail.starts_with(char::is_whitespace) {
                continue;
            }
            let Some(lock) = tail.trim_start().strip_prefix('<') else {
                continue;
            };
            let Some(digits) = lock.strip_prefix("0x") else {
                continue;
            };
            let length = digits
                .find(|c: char| !c.is_ascii_hexdigit())
                .unwrap_or(digits.len());
            if length > 0 && digits[length..].starts_with('>') {
                return Some((kind, &lock[..length + 2]));
            }
        }
    }
    None
}
fn omitted_version(tail: &str) -> Option<&str> {
    for (at, _) in tail.match_indices("omitted for conflict with ") {
        let rest = &tail[at + 26..];
        let end = rest.find([' ', ')']).unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}

pub fn dependencies(input: &str) -> Result<String> {
    dependencies_limited(input, 8 * 1024 * 1024)
}
pub(crate) fn dependencies_limited(input: &str, report_limit: usize) -> Result<String> {
    bounded(input)?;
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
        } else if let Some(version) = omitted_version(tail) {
            version
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
    let mut report = json!({"entries":entries,"multipleSelectedVersions":conflicts,"unparsedLines":skipped,"notes":"支持 Maven 标准坐标/树和 Gradle module:artifact:version、-> 版本选择；Gradle dependencyInsight 带 Selection reasons 和树连接符时附带可识别的反向路径。不同 configuration/scope 中的版本差异不等于运行时冲突；约束/重复子树保留标记，不执行 wrapper。"});
    if let Some(insight) = gradle_dependency_insight(input) {
        report["dependencyInsight"] = insight;
    }
    super::report_limited(report, report_limit)
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

struct InsightNode {
    depth: usize,
    parent: Option<usize>,
    label: String,
    has_child: bool,
}

struct InsightBlock {
    target: String,
    requested: String,
    selected: String,
    line: usize,
    nodes: Vec<InsightNode>,
    stack: Vec<usize>,
}

impl InsightBlock {
    fn new(coordinate: &str, tail: &str, line: usize, label: String) -> Option<Self> {
        let pieces: Vec<_> = coordinate.split(':').collect();
        if pieces.len() < 3 {
            return None;
        }
        let target = format!("{}:{}", pieces[0], pieces[1]);
        let requested = pieces[2].to_owned();
        let selected = tail
            .split_once(" -> ")
            .and_then(|(_, version)| version.split_whitespace().next())
            .unwrap_or(&requested)
            .to_owned();
        Some(Self {
            target: target.clone(),
            requested,
            selected,
            line,
            nodes: vec![InsightNode {
                depth: 0,
                parent: None,
                label,
                has_child: false,
            }],
            stack: vec![0],
        })
    }

    fn add_node(&mut self, depth: usize, label: String) -> bool {
        if depth == 0 || depth > 64 || self.nodes.len() >= 2048 {
            return false;
        }
        while self
            .stack
            .last()
            .is_some_and(|index| self.nodes[*index].depth >= depth)
        {
            self.stack.pop();
        }
        let Some(parent) = self.stack.last().copied() else {
            return false;
        };
        if self.nodes[parent].depth + 1 < depth {
            return false;
        }
        self.nodes[parent].has_child = true;
        let index = self.nodes.len();
        self.nodes.push(InsightNode {
            depth,
            parent: Some(parent),
            label,
            has_child: false,
        });
        self.stack.push(index);
        true
    }

    fn into_paths(
        self,
        output: &mut Vec<Value>,
        seen: &mut BTreeSet<String>,
        truncated: &mut bool,
    ) {
        if self.nodes.len() < 2 {
            return;
        }
        for (index, node) in self.nodes.iter().enumerate().skip(1) {
            if node.has_child {
                continue;
            }
            let mut path = Vec::new();
            let mut cursor = Some(index);
            while let Some(current) = cursor {
                let node = &self.nodes[current];
                path.push(node.label.clone());
                cursor = node.parent;
            }
            let key = format!(
                "{}\0{}\0{}\0{}",
                self.target,
                self.requested,
                self.selected,
                path.join("\0")
            );
            if !seen.insert(key) {
                continue;
            }
            if output.len() >= 128 {
                *truncated = true;
                break;
            }
            output.push(json!({"target":self.target,"requested":self.requested,"selected":self.selected,"rootToTarget":path,"line":self.line}));
        }
    }
}

fn gradle_dependency_insight(input: &str) -> Option<Value> {
    let mut saw_selection_reasons = false;
    let mut awaiting_target = false;
    let mut target_count = 0usize;
    let mut truncated = false;
    let mut current: Option<InsightBlock> = None;
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();

    for (index, original) in input.lines().enumerate() {
        let line = original
            .strip_prefix("[INFO] ")
            .unwrap_or(original)
            .trim_end();
        if line.trim() == "Selection reasons:" {
            saw_selection_reasons = true;
            awaiting_target = true;
            if let Some(block) = current.take() {
                block.into_paths(&mut paths, &mut seen, &mut truncated);
            }
            continue;
        }
        if !saw_selection_reasons {
            continue;
        }

        if awaiting_target {
            if let Some((prefix, coordinate, tail)) = dependency_row(line)
                && prefix.trim().is_empty()
            {
                if target_count >= 1000 {
                    truncated = true;
                    break;
                }
                let label = bounded_insight_label(coordinate, &mut truncated);
                current = InsightBlock::new(coordinate, tail, index + 1, label);
                if current.is_some() {
                    target_count += 1;
                    awaiting_target = false;
                }
            }
            continue;
        }

        if let Some((prefix, coordinate, tail)) = dependency_row(line) {
            let depth = prefix.len() / 5;
            if depth == 0 {
                if let Some(block) = current.take() {
                    block.into_paths(&mut paths, &mut seen, &mut truncated);
                }
                if target_count >= 1000 {
                    truncated = true;
                    break;
                }
                let label = bounded_insight_label(coordinate, &mut truncated);
                current = InsightBlock::new(coordinate, tail, index + 1, label);
                if current.is_some() {
                    target_count += 1;
                }
            } else if let Some(block) = current.as_mut() {
                let label = bounded_insight_label(coordinate, &mut truncated);
                if !block.add_node(depth, label) {
                    truncated = true;
                }
            }
            continue;
        }

        if let Some((depth, label)) = gradle_tree_line(line)
            && let Some(block) = current.as_mut()
        {
            let label = bounded_insight_label(label, &mut truncated);
            if !block.add_node(depth, label) {
                truncated = true;
            }
        }
    }
    if !saw_selection_reasons {
        return None;
    }
    if let Some(block) = current {
        block.into_paths(&mut paths, &mut seen, &mut truncated);
    }
    Some(
        json!({"targetCount":target_count,"paths":paths,"truncated":truncated,"notes":"仅从带 Selection reasons 标记及可识别 Gradle 树连接符的区段提取；rootToTarget 按报告中的反向依赖方向重排，不推断未打印的依赖。"}),
    )
}

fn gradle_tree_line(line: &str) -> Option<(usize, &str)> {
    let marker = line.find("+---").or_else(|| line.find("\\---"))?;
    let depth = line[..marker].len() / 5 + 1;
    let label = line[marker + 4..].trim();
    (!label.is_empty()).then_some((depth, label))
}

fn bounded_insight_label(value: &str, truncated: &mut bool) -> String {
    let mut chars = value.chars();
    let label: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        *truncated = true;
    }
    label
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
    const MAX_CALL_TREE_NODES: usize = 256;
    const MAX_STACK_DEPTH: usize = 128;
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
    let mut call_nodes = Vec::<(Option<usize>, String, usize)>::new();
    let mut call_node_ids = HashMap::<(Option<usize>, String), usize>::new();
    let mut call_tree_samples = 0_usize;
    let mut call_tree_truncated = false;
    let mut timeline = vec![];
    for (index, e) in events.iter().enumerate() {
        let kind = e["type"].as_str().context("事件缺少 type")?;
        *counts.entry(kind.into()).or_default() += 1;
        if kind == "jdk.ExecutionSample" {
            if let Some(frames) = e
                .pointer("/values/stackTrace/frames")
                .and_then(Value::as_array)
            {
                if frames.is_empty() {
                    call_tree_truncated = true;
                }
                if frames.len() > MAX_STACK_DEPTH {
                    call_tree_truncated = true;
                }
                let mut parent = None;
                let mut included_frame = false;
                for frame in frames.iter().take(MAX_STACK_DEPTH).rev() {
                    let Some(method) = frame.get("method") else {
                        call_tree_truncated = true;
                        break;
                    };
                    let class = method
                        .pointer("/type/name")
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    let name = method["name"].as_str().unwrap_or("?");
                    let full_name = format!("{class}.{name}");
                    let key = (parent, full_name.clone());
                    let node = if let Some(node) = call_node_ids.get(&key).copied() {
                        node
                    } else if call_nodes.len() < MAX_CALL_TREE_NODES {
                        let node = call_nodes.len();
                        call_nodes.push((parent, jfr_display_method(&full_name), 0));
                        call_node_ids.insert(key, node);
                        node
                    } else {
                        call_tree_truncated = true;
                        break;
                    };
                    call_nodes[node].2 += 1;
                    parent = Some(node);
                    included_frame = true;
                }
                if included_frame {
                    call_tree_samples += 1;
                }
                if let Some(method) = e.pointer("/values/stackTrace/frames/0/method") {
                    let class = method
                        .pointer("/type/name")
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    let name = method["name"].as_str().unwrap_or("?");
                    *hotspots.entry(format!("{class}.{name}")).or_default() += 1;
                }
            } else {
                call_tree_truncated = true;
            }
        }
        timeline.push(json!({"index":index,"type":kind,"startTime":e.pointer("/values/startTime"),"durationSeconds":e.pointer("/values/duration").and_then(Value::as_str).and_then(duration_seconds)}));
    }
    let mut hot: Vec<_> = hotspots.into_iter().collect();
    hot.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let call_tree_nodes: Vec<_> = call_nodes
        .into_iter()
        .enumerate()
        .map(|(index, (parent, method, samples))| {
            json!({"index":index,"parentIndex":parent,"method":method,"inclusiveSamples":samples})
        })
        .collect();
    super::report_limited(
        json!({"eventCount":events.len(),"counts":counts,"sampleTopFrames":hot.into_iter().take(30).map(|(name,count)|json!({"method":name,"samples":count})).collect::<Vec<_>>(),"sampleCallTree":{"rootToLeaf":true,"sampleSemantics":"inclusiveSamples counts sampled stacks passing through this node","samples":call_tree_samples,"truncated":call_tree_truncated,"maxNodes":MAX_CALL_TREE_NODES,"maxStackDepth":MAX_STACK_DEPTH,"nodes":call_tree_nodes},"timeline":timeline,"notes":"支持 JDK jfr print --json 事件结构；热点为 ExecutionSample 顶层帧样本数，不是 CPU 百分比。采样调用树按调用路径聚合，inclusiveSamples 表示经过该节点的样本数；truncated=true 表示采样栈缺失/为空，或部分栈帧/新路径因限额省略。不是 CPU 百分比或完整火焰图，不替代 JMC。"}),
        report_limit,
    )
}

fn jfr_display_method(method: &str) -> String {
    let mut chars = method.chars();
    let prefix: String = chars.by_ref().take(159).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::*;
    #[test]
    fn scanners_preserve_previous_regex_capture_semantics() {
        let state = regex::Regex::new(r"java.lang.Thread.State:\s+([A-Z_]+)").unwrap();
        let lock = regex::Regex::new(
            r"- (locked|waiting on|waiting to lock|parking to wait for)\s+<(0x[0-9a-fA-F]+)>",
        )
        .unwrap();
        let omitted = regex::Regex::new(r"omitted for conflict with ([^ )]+)").unwrap();
        for whitespace in [" ", "\t", "\u{2003}", "", "  "] {
            for prefix in ["", "prefix invalid java.lang.Thread.State: x "] {
                for separator in [".", "X", "中", ""] {
                    let line = format!(
                        "{prefix}java{separator}lang{separator}Thread{separator}State:{whitespace}WAITING extra"
                    );
                    assert_eq!(
                        thread_state(&line),
                        state.captures(&line).map(|c| c.get(1).unwrap().as_str()),
                        "{line:?}"
                    );
                }
            }
            for kind in [
                "locked",
                "waiting on",
                "waiting to lock",
                "parking to wait for",
                "lockedd",
            ] {
                for id in ["0xAb01", "0x", "0xg1", "0X123", "0x12", "0x中"] {
                    let line = format!("- bad <0xff> then - {kind}{whitespace}<{id}> trailing");
                    assert_eq!(
                        thread_lock(&line),
                        lock.captures(&line)
                            .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str())),
                        "{line:?}"
                    );
                }
            }
        }
        for text in [
            "prefix omitted for conflict with 2.0)",
            "omitted for conflict with ) omitted for conflict with 3.0",
            "omitted for conflict with \t中)",
            "omitted for conflict with ",
        ] {
            assert_eq!(
                omitted_version(text),
                omitted.captures(text).map(|c| c.get(1).unwrap().as_str())
            );
        }
    }
}
