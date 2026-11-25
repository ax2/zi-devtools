//! Reviews only recorded retrieval results, independently of any index or model.
use crate::{
    CaseResult, ComparisonCase, MAX_CASES, MAX_REPORT_BYTES, comparison_stats, first_expected_rank,
    reciprocal_rank, valid_text, validate_case_content,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt::Write as _};

const SINGLE: &str = "zi-devtools-rag-eval";
const COMPARE: &str = "zi-devtools-rag-compare";
type Failure = &'static str;
const INVALID: Failure = "INVALID_REPORT";

#[derive(Deserialize)]
struct Identity {
    schema: String,
    version: u32,
    mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SingleReport {
    #[serde(rename = "schema")]
    _schema: String,
    #[serde(rename = "version")]
    _version: u32,
    created_at: String,
    #[serde(rename = "mode")]
    _mode: String,
    retrieval: String,
    #[serde(deserialize_with = "crate::present_option")]
    embedding_model: Option<String>,
    #[serde(deserialize_with = "crate::present_option")]
    keyword_weight: Option<u8>,
    #[serde(deserialize_with = "crate::present_option")]
    model: Option<String>,
    top_k: usize,
    total: usize,
    completed: usize,
    retrieval_hits: usize,
    cited_expected: usize,
    literal_expected_cases: usize,
    literal_answer_matches: usize,
    cancelled: bool,
    results: Vec<CaseResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareReport {
    #[serde(rename = "schema")]
    _schema: String,
    #[serde(rename = "version")]
    _version: u32,
    created_at: String,
    top_k: usize,
    embedding_model: String,
    keyword_weight: u8,
    total: usize,
    completed: usize,
    paired: usize,
    keyword_hits: usize,
    hybrid_hits: usize,
    hybrid_only: usize,
    keyword_only: usize,
    both: usize,
    neither: usize,
    keyword_mrr: f64,
    hybrid_mrr: f64,
    cancelled: bool,
    results: Vec<ComparisonCase>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    format: &'static str,
    top_k: usize,
    total: usize,
    completed: usize,
    cancelled: bool,
    matches_report: bool,
    differences: Vec<&'static str>,
    single: Option<SingleMetrics>,
    comparison: Option<CompareMetrics>,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SingleMetrics {
    retrieval: String,
    failed: usize,
    hits: usize,
    mrr: Option<f64>,
    rows: Vec<SingleRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SingleRow {
    case_number: usize,
    first_rank: Option<usize>,
    recorded_rank: Option<usize>,
    failed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompareMetrics {
    paired: usize,
    excluded: usize,
    keyword_hits: usize,
    hybrid_hits: usize,
    keyword_only: usize,
    hybrid_only: usize,
    both: usize,
    neither: usize,
    keyword_mrr: Option<f64>,
    hybrid_mrr: Option<f64>,
    rows: Vec<CompareRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompareRow {
    case_number: usize,
    keyword_rank: Option<usize>,
    hybrid_rank: Option<usize>,
    keyword_failed: bool,
    hybrid_failed: bool,
    included: bool,
}

fn snapshot_valid(
    top_k: usize,
    total: usize,
    completed: usize,
    rows: usize,
    cancelled: bool,
) -> bool {
    [3, 5, 8].contains(&top_k)
        && (1..=MAX_CASES).contains(&total)
        && completed == rows
        && completed <= total
        && (cancelled || completed == total)
}

fn valid_rank(rank: Option<usize>, top_k: usize) -> bool {
    rank.is_none_or(|rank| (1..=top_k).contains(&rank))
}

fn mrr_text(mrr: Option<f64>) -> String {
    mrr.map_or_else(|| "—".to_owned(), |mrr| format!("{mrr:.6}"))
}

fn rank_text(rank: Option<usize>, failed: bool) -> String {
    if failed {
        "失败".to_owned()
    } else {
        rank.map_or_else(|| "未命中".to_owned(), |rank| rank.to_string())
    }
}

fn head(top_k: usize, completed: usize, total: usize, cancelled: bool) -> String {
    let state = if cancelled { " · 已取消" } else { "" };
    format!("Top K {top_k} · 已完成 {completed}/{total} 题{state}")
}

fn add_review_summary(text: &mut String, differences: &[&str]) {
    if differences.is_empty() {
        text.push_str("\n复算与报告记录一致。");
    } else {
        let _ = write!(text, "\n发现 {} 项不一致", differences.len());
        for field in differences {
            let label = match *field {
                "expected_rank" => "单路记录名次",
                "retrieval_hits" => "单路命中数",
                "literal_expected_cases" => "预期片段题数",
                "model_only_counters" => "仅模型模式使用的计数",
                "paired" => "配对题数",
                "keyword_hits" => "关键词命中数",
                "hybrid_hits" => "混合命中数",
                "keyword_only" => "仅关键词命中数",
                "hybrid_only" => "仅混合命中数",
                "both" => "两路都命中数",
                "neither" => "两路均未命中数",
                "keyword_mrr" => "关键词 MRR",
                "hybrid_mrr" => "混合 MRR",
                _ => field,
            };
            let _ = write!(text, " · {label}");
        }
    }
}

pub fn read(bytes: &[u8]) -> Result<Review, Failure> {
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let identity: Identity = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    match (identity.schema.as_str(), identity.version) {
        (SINGLE, 2) if identity.mode.as_deref() == Some("retrieval-only") => {
            single(serde_json::from_slice(bytes).map_err(|_| INVALID)?)
        }
        (SINGLE, 2) => Err("UNSUPPORTED_MODE"),
        (COMPARE, 1) => compare(serde_json::from_slice(bytes).map_err(|_| INVALID)?),
        _ => Err("UNSUPPORTED_VERSION"),
    }
}

fn single(report: SingleReport) -> Result<Review, Failure> {
    if !snapshot_valid(
        report.top_k,
        report.total,
        report.completed,
        report.results.len(),
        report.cancelled,
    ) || !valid_text(&report.created_at, 64, 64)
        || report.model.is_some()
    {
        return Err(INVALID);
    }
    let label = match report.retrieval.as_str() {
        "keyword" if report.embedding_model.is_none() && report.keyword_weight.is_none() => {
            "关键词"
        }
        "hybrid"
            if report
                .embedding_model
                .as_ref()
                .is_some_and(|model| !model.trim().is_empty())
                && matches!(report.keyword_weight, Some(25 | 50 | 75)) =>
        {
            "混合"
        }
        _ => return Err(INVALID),
    };
    let mut differences = Vec::new();
    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    let mut hits = 0;
    let mut failed = 0;
    let mut rr = 0.0;
    let mut literal_cases = 0;
    let mut rank_mismatch = false;
    for (index, row) in report.results.iter().enumerate() {
        if !valid_text(&row.id, 64, 128)
            || !ids.insert(&row.id)
            || row.evidence_count != row.evidence.len()
            || row.evidence_count > report.top_k
            || !valid_rank(row.expected_rank, report.top_k)
            || row.answer.is_some()
            || row.expected_cited.is_some()
            || row.answer_contains_expected.is_some()
            || !row.cited_ids.is_empty()
            || (row.error.is_some() && !row.evidence.is_empty())
        {
            return Err(INVALID);
        }
        validate_case_content(
            &row.id,
            &row.question,
            &row.terms,
            &row.expected_source,
            &row.expected_relative,
            row.expected_answer_contains.as_deref(),
        )
        .map_err(|_| INVALID)?;
        if !row.evidence.iter().enumerate().all(|(index, evidence)| {
            evidence.id == index + 1
                && evidence.file_sha256.len() == 64
                && evidence
                    .file_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(INVALID);
        }
        let rank = first_expected_rank(
            row.evidence
                .iter()
                .map(|item| (item.id, item.source.as_str(), item.relative.as_str())),
            &row.expected_source,
            &row.expected_relative,
        );
        hits += usize::from(rank.is_some());
        failed += usize::from(row.error.is_some());
        literal_cases += usize::from(row.expected_answer_contains.is_some());
        rr += reciprocal_rank(rank);
        rank_mismatch |= rank != row.expected_rank;
        rows.push(SingleRow {
            case_number: index + 1,
            first_rank: rank,
            recorded_rank: row.expected_rank,
            failed: row.error.is_some(),
        });
    }
    if rank_mismatch {
        differences.push("expected_rank");
    }
    if hits != report.retrieval_hits {
        differences.push("retrieval_hits");
    }
    if literal_cases != report.literal_expected_cases {
        differences.push("literal_expected_cases");
    }
    if report.cited_expected != 0 || report.literal_answer_matches != 0 {
        differences.push("model_only_counters");
    }
    let mrr = (report.completed > 0).then(|| rr / report.completed as f64);
    let mut text = head(
        report.top_k,
        report.completed,
        report.total,
        report.cancelled,
    );
    let _ = write!(
        text,
        "\n{label}单路 · 命中 {hits}/{} · 失败 {failed} · 推导 MRR {}",
        report.completed,
        mrr_text(mrr)
    );
    for row in &rows {
        let _ = write!(
            text,
            "\n第 {} 题 · 首个正确名次 {}",
            row.case_number,
            rank_text(row.first_rank, row.failed)
        );
        if row.first_rank != row.recorded_rank {
            let _ = write!(text, "（报告记录 {}）", rank_text(row.recorded_rank, false));
        }
    }
    text.push_str("\n首名按已记录证据复算。所有完成题进入分母，失败和未命中按 0 计。");
    add_review_summary(&mut text, &differences);
    Ok(Review {
        format: "retrieval-only",
        top_k: report.top_k,
        total: report.total,
        completed: report.completed,
        cancelled: report.cancelled,
        matches_report: differences.is_empty(),
        differences,
        single: Some(SingleMetrics {
            retrieval: report.retrieval,
            failed,
            hits,
            mrr,
            rows,
        }),
        comparison: None,
        text,
    })
}

fn compare(report: CompareReport) -> Result<Review, Failure> {
    if !snapshot_valid(
        report.top_k,
        report.total,
        report.completed,
        report.results.len(),
        report.cancelled,
    ) || !valid_text(&report.created_at, 64, 64)
        || report.embedding_model.trim().is_empty()
        || ![25, 50, 75].contains(&report.keyword_weight)
    {
        return Err(INVALID);
    }
    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    for (index, row) in report.results.iter().enumerate() {
        if !valid_text(&row.id, 64, 128)
            || !ids.insert(&row.id)
            || !valid_text(&row.expected_source, 80, 320)
            || !valid_text(&row.expected_relative, 512, 2048)
            || !valid_rank(row.keyword_rank, report.top_k)
            || !valid_rank(row.hybrid_rank, report.top_k)
            || (row.keyword_rank.is_some() && row.keyword_error.is_some())
            || (row.hybrid_rank.is_some() && row.hybrid_error.is_some())
        {
            return Err(INVALID);
        }
        rows.push(CompareRow {
            case_number: index + 1,
            keyword_rank: row.keyword_rank,
            hybrid_rank: row.hybrid_rank,
            keyword_failed: row.keyword_error.is_some(),
            hybrid_failed: row.hybrid_error.is_some(),
            included: row.keyword_error.is_none() && row.hybrid_error.is_none(),
        });
    }
    let stats = comparison_stats(&report.results);
    let excluded = report.completed - stats.paired;
    let keyword_mrr = (stats.paired > 0).then_some(stats.keyword_mrr);
    let hybrid_mrr = (stats.paired > 0).then_some(stats.hybrid_mrr);
    let mut differences: Vec<_> = [
        ("paired", report.paired, stats.paired),
        ("keyword_hits", report.keyword_hits, stats.keyword_hits),
        ("hybrid_hits", report.hybrid_hits, stats.hybrid_hits),
        ("keyword_only", report.keyword_only, stats.keyword_only),
        ("hybrid_only", report.hybrid_only, stats.hybrid_only),
        ("both", report.both, stats.both),
        ("neither", report.neither, stats.neither),
    ]
    .into_iter()
    .filter_map(|(name, actual, expected)| (actual != expected).then_some(name))
    .collect();
    for (name, actual, expected) in [
        ("keyword_mrr", report.keyword_mrr, stats.keyword_mrr),
        ("hybrid_mrr", report.hybrid_mrr, stats.hybrid_mrr),
    ] {
        if (actual - expected).abs() > 1e-12 {
            differences.push(name);
        }
    }
    let mut text = head(
        report.top_k,
        report.completed,
        report.total,
        report.cancelled,
    );
    let _ = write!(text, "\n配对 {} 题 · 排除失败 {excluded} 题", stats.paired);
    let _ = write!(
        text,
        "\n关键词命中 {}/{} · MRR {}",
        stats.keyword_hits,
        stats.paired,
        mrr_text(keyword_mrr)
    );
    let _ = write!(
        text,
        "\n混合命中 {}/{} · MRR {}",
        stats.hybrid_hits,
        stats.paired,
        mrr_text(hybrid_mrr)
    );
    let _ = write!(
        text,
        "\n两路都命中 {} · 仅关键词 {} · 仅混合 {} · 均未命中 {}",
        stats.both, stats.keyword_only, stats.hybrid_only, stats.neither
    );
    for row in &rows {
        let _ = write!(
            text,
            "\n第 {} 题 · 关键词 {} · 混合 {}",
            row.case_number,
            rank_text(row.keyword_rank, row.keyword_failed),
            rank_text(row.hybrid_rank, row.hybrid_failed)
        );
    }
    text.push_str("\n双路报告仅保存名次。复算汇总只统计两路均成功的题。");
    add_review_summary(&mut text, &differences);
    Ok(Review {
        format: "comparison",
        top_k: report.top_k,
        total: report.total,
        completed: report.completed,
        cancelled: report.cancelled,
        matches_report: differences.is_empty(),
        differences,
        single: None,
        comparison: Some(CompareMetrics {
            paired: stats.paired,
            excluded,
            keyword_hits: stats.keyword_hits,
            hybrid_hits: stats.hybrid_hits,
            keyword_only: stats.keyword_only,
            hybrid_only: stats.hybrid_only,
            both: stats.both,
            neither: stats.neither,
            keyword_mrr,
            hybrid_mrr,
            rows,
        }),
        text,
    })
}
