//! Read-only parsers for pasted, standard text traces. No code or SQL execution.
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn lines(input: &str) -> Result<Vec<&str>> {
    ensure!(
        input.len() <= 1024 * 1024,
        "输入超过 1 MiB，请先缩小内容范围"
    );
    let lines: Vec<_> = input.lines().collect();
    ensure!(lines.len() <= 20_000, "最多处理 20000 行日志");
    Ok(lines)
}
#[derive(Serialize)]
struct JavaException {
    relation: String,
    parent: Option<usize>,
    class: String,
    message: String,
    suppressed_branch: bool,
    source_line: usize,
    frames: Vec<Value>,
    omitted_frames: usize,
}
pub fn java_trace(input: &str) -> Result<String> {
    let header = Regex::new(
        r#"^(?:Exception in thread "[^"]+" )?([A-Za-z_$][\w$]*(?:[./][A-Za-z_$][\w$]*)*)(?::\s?(.*))?$"#,
    )?;
    let frame = Regex::new(r"^at\s+(.+)\(([^()]*)\)$")?;
    let omitted = Regex::new(r"^\.\.\. (\d+) (?:more|common frames omitted)$")?;
    let mut nodes: Vec<JavaException> = Vec::new();
    let mut active: BTreeMap<usize, usize> = BTreeMap::new();
    let mut ends = Vec::new();
    let mut root = None;
    let mut frames = 0;
    for (line_no, line) in lines(input)?.iter().enumerate() {
        let trimmed = line.trim();
        let indent = line
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum();
        let (relation, text) = if let Some(s) = trimmed.strip_prefix("Caused by: ") {
            ("cause", s)
        } else if let Some(s) = trimmed.strip_prefix("Suppressed: ") {
            ("suppressed", s)
        } else {
            ("root", trimmed)
        };
        if let Some(c) = header.captures(text) {
            let class = c[1].to_owned();
            if relation == "root"
                && !(class.contains('.')
                    || class.ends_with("Exception")
                    || class.ends_with("Error")
                    || class == "Throwable")
            {
                continue;
            }
            ensure!(nodes.len() < 512, "异常节点超过 512 项，请缩小日志范围");
            let parent = if relation == "root" {
                active.clear();
                None
            } else if relation == "suppressed" {
                active
                    .range(..indent)
                    .next_back()
                    .or_else(|| active.iter().next_back())
                    .map(|(_, i)| *i)
            } else {
                active.range(..=indent).next_back().map(|(_, i)| *i)
            };
            let suppressed_branch =
                relation == "suppressed" || parent.is_some_and(|i| nodes[i].suppressed_branch);
            let index = nodes.len();
            nodes.push(JavaException {
                relation: relation.into(),
                parent,
                class,
                message: c.get(2).map(|m| m.as_str()).unwrap_or("").into(),
                suppressed_branch,
                source_line: line_no + 1,
                frames: Vec::new(),
                omitted_frames: 0,
            });
            active.retain(|level, _| *level <= indent);
            active.insert(indent, index);
            if relation == "root" || (root.is_none() && !suppressed_branch) {
                root = Some(ends.len());
                ends.push(index);
            }
            if !suppressed_branch && let Some(root) = root {
                ends[root] = index;
            }
        } else if let Some(c) = frame.captures(trimmed) {
            if let Some(node) = nodes.last_mut() {
                frames += 1;
                ensure!(frames <= 10_000, "堆栈帧超过 10000 项");
                node.frames
                    .push(json!({"call":&c[1],"location":&c[2],"sourceLine":line_no+1}));
            }
        } else if let Some(c) = omitted.captures(trimmed)
            && let Some(node) = nodes.last_mut()
        {
            node.omitted_frames = node
                .omitted_frames
                .checked_add(c[1].parse::<usize>().context("省略帧数无效")?)
                .context("省略帧数过大")?;
        }
    }
    ensure!(
        !nodes.is_empty(),
        "未识别到 Java 异常；请粘贴标准 printStackTrace 文本，去除日志时间戳前缀"
    );
    Ok(serde_json::to_string_pretty(
        &json!({"exceptionCount":nodes.len(),"frameCount":frames,"lastVisibleMainExceptions":ends,"exceptions":nodes,"notes":"节点索引从 0 开始；source_line 为输入行号。Suppressed 分支不会替代主异常链终点。省略帧只计数，不还原；最后可见异常不等于已确认的业务根因。"}),
    )?)
}

#[derive(Serialize, Default)]
struct PythonException {
    relation: String,
    frames: Vec<Value>,
    exception: Option<String>,
    message: Option<String>,
    hint: Option<&'static str>,
}
fn django_hint(class: &str) -> Option<&'static str> {
    match class.rsplit('.').next().unwrap_or(class) {
        "NoReverseMatch" => {
            Some("检查 URL 名称、命名空间以及 reverse/url 标签传入的参数；提示不代表已定位根因。")
        }
        "DisallowedHost" => Some("核对实际请求 Host 与 ALLOWED_HOSTS 配置，避免直接放开所有主机。"),
        "TemplateDoesNotExist" => Some("检查模板名称、TEMPLATES 搜索目录、APP_DIRS 与应用注册。"),
        "IntegrityError" => {
            Some("结合数据库约束检查唯一键、外键及事务；不要直接删除数据或迁移记录。")
        }
        "OperationalError" => {
            Some("结合数据库原始错误检查连接、权限或表结构；仅凭异常类型无法确定原因。")
        }
        "NodeNotFoundError" | "InconsistentMigrationHistory" => {
            Some("检查迁移依赖图与已应用记录；不要盲目使用 --fake。")
        }
        "ImproperlyConfigured" => {
            Some("检查 settings、应用配置和运行环境；不要将密钥复制到诊断报告。")
        }
        "ModuleNotFoundError" | "ImportError" => {
            Some("确认当前解释器、虚拟环境、依赖安装和模块名称。")
        }
        _ => None,
    }
}
pub fn django_trace(input: &str) -> Result<String> {
    let frame = Regex::new(r#"^\s*File "(.+)", line (\d+)(?:, in (.+))?$"#)?;
    let error = Regex::new(
        r"^([A-Za-z_][\w.]*(?:Error|Exception|Warning|Exit|Interrupt|Match|Host|Exist|History|Configured)?)(?::\s?(.*))?$",
    )?;
    let mut blocks: Vec<PythonException> = Vec::new();
    let mut relation = "initial";
    let mut count = 0;
    for (line_no, line) in lines(input)?.iter().enumerate() {
        if *line == "During handling of the above exception, another exception occurred:" {
            relation = "during_handling";
            continue;
        }
        if *line == "The above exception was the direct cause of the following exception:" {
            relation = "direct_cause";
            continue;
        }
        if *line == "Traceback (most recent call last):" {
            ensure!(blocks.len() < 256, "异常链超过 256 段");
            blocks.push(PythonException {
                relation: relation.into(),
                ..Default::default()
            });
            relation = "independent";
            continue;
        }
        if let Some(c) = frame.captures(line) {
            if blocks.is_empty() {
                blocks.push(PythonException {
                    relation: "header_missing".into(),
                    ..Default::default()
                });
            }
            count += 1;
            ensure!(count <= 10_000, "堆栈帧超过 10000 项");
            blocks.last_mut().unwrap().frames.push(json!({"file":&c[1],"line":c[2].parse::<u64>().context("文件行号过大")?,"function":c.get(3).map(|m|m.as_str()),"sourceLine":line_no+1}));
        } else if !line.starts_with(char::is_whitespace)
            && let Some(c) = error.captures(line)
            && let Some(block) = blocks.last_mut()
            && block.exception.is_none()
            && !block.frames.is_empty()
        {
            block.exception = Some(c[1].into());
            block.message = Some(c.get(2).map(|m| m.as_str()).unwrap_or("").into());
            block.hint = django_hint(&c[1]);
        }
    }
    ensure!(
        count > 0,
        "未识别到 Python 堆栈帧；请粘贴文本 Traceback（不支持 Django HTML 调试页或 ExceptionGroup 树）"
    );
    let incomplete = blocks.iter().any(|b| b.exception.is_none());
    let last_frame = blocks.last().and_then(|b| b.frames.last());
    Ok(serde_json::to_string_pretty(
        &json!({"exceptionCount":blocks.len(),"frameCount":count,"incomplete":incomplete,"lastVisibleFrame":last_frame,"exceptions":blocks,"notes":"保留输入中的路径，不读取文件、不执行 Python。只提取异常首行消息；不支持 ExceptionGroup 树。最后可见帧与规则提示是排查线索，不是根因结论。"}),
    )?)
}
