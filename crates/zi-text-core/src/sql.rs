use std::{borrow::Cow, collections::BTreeMap, fmt::Write};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Deserializer};
use sqlformat::{Dialect as FormatterDialect, FormatOptions, Indent, QueryParams};
#[cfg(target_os = "wasi")]
use sqlformat_wasi as sqlformat;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IndentStyle {
    TwoSpaces,
    #[default]
    FourSpaces,
    Tabs,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeywordCase {
    #[default]
    Uppercase,
    Lowercase,
    Preserve,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Dialect {
    Generic,
    PostgreSql,
    MySql,
    Sqlite,
    SqlServer,
}

impl Dialect {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" | "通用" => Ok(Self::Generic),
            "postgresql" | "postgres" | "pg" => Ok(Self::PostgreSql),
            "mysql" => Ok(Self::MySql),
            "sqlite" => Ok(Self::Sqlite),
            "sqlserver" | "sql server" | "tsql" => Ok(Self::SqlServer),
            _ => Err(anyhow!("请选择支持的 SQL 方言")),
        }
    }

    fn formatter(self) -> FormatterDialect {
        match self {
            Self::PostgreSql => FormatterDialect::PostgreSql,
            Self::SqlServer => FormatterDialect::SQLServer,
            Self::Generic | Self::MySql | Self::Sqlite => FormatterDialect::Generic,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TextInput {
    pub(crate) text: String,
    #[serde(default, deserialize_with = "present")]
    dialect: Option<Dialect>,
    #[serde(default, deserialize_with = "present")]
    indent: Option<IndentStyle>,
    #[serde(default, deserialize_with = "present")]
    keyword_case: Option<KeywordCase>,
}

impl TextInput {
    pub(crate) fn has_sql_options(&self) -> bool {
        self.dialect.is_some() || self.indent.is_some() || self.keyword_case.is_some()
    }

    pub(crate) fn format(&self) -> Result<String> {
        #[cfg(target_os = "wasi")]
        let wire_overhead = serde_json::to_vec(&crate::ResultEnvelope::success(String::new()))
            .expect("empty success serializes")
            .len();
        format_sql(
            &self.text,
            self.dialect.unwrap_or(Dialect::Generic),
            self.indent.unwrap_or_default(),
            self.keyword_case.unwrap_or_default(),
            #[cfg(target_os = "wasi")]
            Some(crate::REQUEST_LIMIT - wire_overhead),
        )
    }
}

// An absent property uses its default, but explicit null is outside the string enum.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

pub fn format(input: &str, dialect: &str) -> Result<String> {
    format_with_options(
        input,
        dialect,
        IndentStyle::FourSpaces,
        KeywordCase::Uppercase,
    )
}

pub fn format_with_options(
    input: &str,
    dialect: &str,
    indent_style: IndentStyle,
    keyword_case: KeywordCase,
) -> Result<String> {
    format_sql(
        input,
        Dialect::parse(dialect)?,
        indent_style,
        keyword_case,
        #[cfg(target_os = "wasi")]
        None,
    )
}

fn format_sql(
    input: &str,
    dialect: Dialect,
    indent_style: IndentStyle,
    keyword_case: KeywordCase,
    #[cfg(target_os = "wasi")] response_content_budget: Option<usize>,
) -> Result<String> {
    if input.trim().is_empty() {
        return Err(anyhow!("请输入 SQL 语句"));
    }

    let (protected_input, literals) = protect_dialect_regions(input, dialect)?;

    let options = FormatOptions {
        dialect: dialect.formatter(),
        indent: match indent_style {
            IndentStyle::TwoSpaces => Indent::Spaces(2),
            IndentStyle::FourSpaces => Indent::Spaces(4),
            IndentStyle::Tabs => Indent::Tabs,
        },
        uppercase: match keyword_case {
            KeywordCase::Uppercase => Some(true),
            KeywordCase::Lowercase => Some(false),
            KeywordCase::Preserve => None,
        },
        max_inline_arguments: Some(40),
        max_inline_top_level: Some(60),
        joins_as_top_level: true,
        ..FormatOptions::default()
    };

    // Keep placeholders verbatim. Formatting must never substitute user parameters.
    #[cfg(target_os = "wasi")]
    if let Some(mut remaining) = response_content_budget.filter(|_| literals.is_empty()) {
        return sqlformat::format_checked(
            &protected_input,
            &QueryParams::None,
            &options,
            |chunk| {
                let Some(rest) = remaining.checked_sub(json_string_content_size(chunk)) else {
                    return false;
                };
                remaining = rest;
                true
            },
        )
        .ok_or_else(|| OutputLimit.into());
    }

    let output = sqlformat::format(&protected_input, &QueryParams::None, &options);
    Ok(restore_protected_literals(output, &literals))
}

#[cfg(target_os = "wasi")]
#[derive(Debug)]
pub(crate) struct OutputLimit;

#[cfg(target_os = "wasi")]
impl std::fmt::Display for OutputLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SQL output exceeds the JSON envelope limit")
    }
}

#[cfg(target_os = "wasi")]
impl std::error::Error for OutputLimit {}

#[cfg(any(test, target_os = "wasi"))]
fn json_string_content_size(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\x08' | b'\t' | b'\n' | b'\x0c' | b'\r' => 2,
            0..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
#[test]
fn output_budget_counts_the_actual_json_escapes() {
    let mut text: String = (0..=127).map(char::from).collect();
    text.push_str("中文😀é\\\"");
    assert_eq!(
        json_string_content_size(&text),
        serde_json::to_vec(&text).unwrap().len() - 2
    );
}

const LITERAL_MARKER: &str = "__ZICODE_SQLFMT_LITERAL_";

struct ProtectedLiteral<'a> {
    text: &'a str,
    leading_underscores: usize,
}

fn protect_dialect_regions(
    input: &str,
    dialect: Dialect,
) -> Result<(Cow<'_, str>, Vec<ProtectedLiteral<'_>>)> {
    if !matches!(dialect, Dialect::PostgreSql | Dialect::Sqlite) {
        return Ok((Cow::Borrowed(input), Vec::new()));
    }

    let bytes = input.as_bytes();
    let mut output = String::new();
    let collisions = literal_marker_collisions(input);
    let mut literals = Vec::new();
    let mut scan = 0;
    let mut copy_from = 0;

    while scan < bytes.len() {
        let protected_end = match bytes[scan] {
            b'\'' | b'"' => {
                scan = quoted_end(bytes, scan, bytes[scan]);
                continue;
            }
            b'-' if bytes.get(scan + 1) == Some(&b'-') => {
                scan = line_comment_end(bytes, scan + 2);
                continue;
            }
            b'/' if bytes.get(scan + 1) == Some(&b'*') => {
                scan = block_comment_end(bytes, scan + 2);
                continue;
            }
            b'$' if dialect == Dialect::PostgreSql => match dollar_delimiter(bytes, scan) {
                Some((delimiter, body_start)) => {
                    let close_offset = bytes[body_start..]
                        .windows(delimiter.len())
                        .position(|window| window == delimiter)
                        .ok_or_else(|| anyhow!("PostgreSQL dollar 引号缺少结束标记"))?;
                    Some(body_start + close_offset + delimiter.len())
                }
                None => None,
            },
            b'[' if dialect == Dialect::Sqlite => bracket_identifier_end(bytes, scan),
            _ => None,
        };

        if let Some(end) = protected_end {
            let index = literals.len();
            let leading_underscores = collisions.get(&index).copied().unwrap_or(0);
            output.push_str(&input[copy_from..scan]);
            output.extend(std::iter::repeat_n('_', leading_underscores));
            write!(output, "{LITERAL_MARKER}{index}__").expect("writing to a string");
            literals.push(ProtectedLiteral {
                text: &input[scan..end],
                leading_underscores,
            });
            scan = end;
            copy_from = end;
        } else {
            scan += 1;
        }
    }

    if literals.is_empty() {
        Ok((Cow::Borrowed(input), literals))
    } else {
        output.push_str(&input[copy_from..]);
        Ok((Cow::Owned(output), literals))
    }
}

fn literal_marker_index(tail: &str) -> Option<(usize, &str)> {
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    let index = &tail[..digits];
    if digits > 1 && index.starts_with('0') {
        return None;
    }
    let suffix = tail[digits..].strip_prefix("__")?;
    Some((index.parse().ok()?, suffix))
}

// Compute the shortest collision-free version of each original marker in one scan.
// Keeping the old marker widths preserves the formatter's line-wrap decisions.
fn literal_marker_collisions(input: &str) -> BTreeMap<usize, usize> {
    let mut collisions = BTreeMap::new();
    for (start, marker) in input.match_indices(LITERAL_MARKER) {
        let Some((index, _)) = literal_marker_index(&input[start + marker.len()..]) else {
            continue;
        };
        let leading_underscores = input.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'_')
            .count()
            + 1;
        let longest = collisions.entry(index).or_insert(0);
        *longest = (*longest).max(leading_underscores);
    }
    collisions
}

fn bracket_identifier_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut scan = start + 1;
    while scan < bytes.len() {
        if bytes[scan] == b']' {
            if bytes.get(scan + 1) == Some(&b']') {
                scan += 2;
            } else {
                return Some(scan + 1);
            }
        } else {
            scan += 1;
        }
    }
    None
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut scan = start + 1;
    while scan < bytes.len() {
        if quote == b'\'' && bytes[scan] == b'\\' {
            scan = (scan + 2).min(bytes.len());
        } else if bytes[scan] == quote {
            if bytes.get(scan + 1) == Some(&quote) {
                scan += 2;
            } else {
                return scan + 1;
            }
        } else {
            scan += 1;
        }
    }
    bytes.len()
}

fn line_comment_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| matches!(byte, b'\n' | b'\r'))
        .map_or(bytes.len(), |offset| start + offset)
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut scan = start;
    let mut depth = 1usize;
    while scan < bytes.len() {
        if bytes[scan..].starts_with(b"/*") {
            depth += 1;
            scan += 2;
        } else if bytes[scan..].starts_with(b"*/") {
            depth -= 1;
            scan += 2;
            if depth == 0 {
                return scan;
            }
        } else {
            scan += 1;
        }
    }
    bytes.len()
}

fn restore_protected_literals(output: String, literals: &[ProtectedLiteral<'_>]) -> String {
    if literals.is_empty() {
        return output;
    }
    let mut restored = String::with_capacity(output.len());
    let mut cursor = 0;
    for (marker_start, marker) in output.match_indices(LITERAL_MARKER) {
        let Some((index, tail)) = literal_marker_index(&output[marker_start + marker.len()..])
        else {
            continue;
        };
        let Some(literal) = literals.get(index) else {
            continue;
        };
        let Some(start) = marker_start.checked_sub(literal.leading_underscores) else {
            continue;
        };
        if start < cursor
            || !output.as_bytes()[start..marker_start]
                .iter()
                .all(|&byte| byte == b'_')
        {
            continue;
        }
        restored.push_str(&output[cursor..start]);
        restored.push_str(literal.text);
        cursor = output.len() - tail.len();
    }
    restored.push_str(&output[cursor..]);
    restored
}

fn dollar_delimiter(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let end = bytes[start + 1..]
        .iter()
        .position(|byte| *byte == b'$')
        .map(|offset| start + 1 + offset)?;
    let tag = &bytes[start + 1..end];
    if tag.is_empty()
        || (matches!(tag[0], b'_' | b'a'..=b'z' | b'A'..=b'Z')
            && tag
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_'))
    {
        Some((&bytes[start..=end], end + 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Dialect, FormatterDialect, format};

    #[test]
    fn formats_nested_cte_case_joins_and_window_functions() {
        let output = format(
            "with active as (select id, user_id from orders where status = 'open') select u.id, case when count(o.id) > 1 then 'many' else 'one' end as label, row_number() over (partition by u.team_id order by u.name) as rank from users u left join active o on o.user_id = u.id group by u.id, u.team_id, u.name",
            "postgresql",
        )
        .unwrap();

        assert!(output.contains("WITH\nactive AS ("));
        assert!(output.contains("LEFT JOIN active o"));
        assert!(output.contains("CASE"));
        assert!(output.contains("row_number() over ("));
        assert!(output.contains("PARTITION BY u.team_id"));
        assert!(output.contains("'open'"));
        assert!(output.contains("'many'"));
    }

    #[test]
    fn preserves_comments_quoted_identifiers_and_parameter_placeholders() {
        let output = format(
            "select `order`, '$1 -- literal' as note, @user, :tenant, ? from `table` -- keep this comment\nwhere id = $1",
            "mysql",
        )
        .unwrap();

        assert!(output.contains("`order`"));
        assert!(output.contains("'$1 -- literal'"));
        assert!(output.contains("@user"));
        assert!(output.contains(":tenant"));
        assert!(output.contains("?"));
        assert!(output.contains("$1"));
        assert!(output.contains("-- keep this comment"));

        let hash_comment = format("select id # keep from, where\nfrom users", "mysql").unwrap();
        assert!(hash_comment.contains("# keep from, where"));
        assert!(hash_comment.contains("FROM users"));
    }

    #[test]
    fn preserves_postgresql_dollar_strings_and_sql_server_bracket_identifiers() {
        let postgres =
            format("select $$select, from$$ as body from records", "postgresql").unwrap();
        assert!(postgres.contains("$$select, from$$"));
        let tagged = format("select $body$from, where$body$ as body", "postgresql").unwrap();
        assert!(tagged.contains("$body$from, where$body$"));
        let embedded = format(
            "select '$tag$inside string' as text, __ZICODE_SQLFMT_LITERAL_0__, $$actual body$$ as body -- $unfinished",
            "postgresql",
        )
        .unwrap();
        assert!(embedded.contains("'$tag$inside string'"));
        assert!(embedded.contains("__ZICODE_SQLFMT_LITERAL_0__"));
        assert!(embedded.contains("$$actual body$$"));
        assert!(format("select $tag$unterminated", "postgresql").is_err());

        let sql_server = format(
            "select [order], [a]]b] from [table] where [user_id] = @id",
            "sqlserver",
        )
        .unwrap();
        assert!(sql_server.contains("[order]"));
        assert!(sql_server.contains("[a]]b]"));
        assert!(sql_server.contains("[table]"));
        assert!(sql_server.contains("[user_id]"));
        assert!(sql_server.contains("@id"));
    }

    #[test]
    fn preserves_sqlite_bracket_quoted_identifiers() {
        let output = format("select [order], [a]]b] from [table]", "sqlite").unwrap();
        assert!(output.contains("[order]"));
        assert!(output.contains("[a]]b]"));
        assert!(output.contains("[table]"));
    }

    #[test]
    fn maps_visible_dialects_to_supported_lexer_modes() {
        assert_eq!(
            Dialect::parse("postgres").unwrap().formatter(),
            FormatterDialect::PostgreSql
        );
        assert_eq!(
            Dialect::parse("tsql").unwrap().formatter(),
            FormatterDialect::SQLServer
        );
        assert_eq!(
            Dialect::parse("mysql").unwrap().formatter(),
            FormatterDialect::Generic
        );
        assert_eq!(
            Dialect::parse("sqlite").unwrap().formatter(),
            FormatterDialect::Generic
        );
        assert!(format("select 1", "oracle").is_err());
    }

    #[test]
    fn rejects_empty_input_and_formats_incomplete_sql_without_claiming_validation() {
        assert!(format("  \n", "generic").is_err());
        let output = format("select from", "generic").unwrap();
        assert!(output.contains("SELECT"));
        assert!(output.contains("FROM"));
    }

    #[test]
    fn applies_selected_indent_and_keyword_case_without_touching_literals() {
        let input = "SeLeCt a_very_long_column_name_that_pushes_the_line_wrap_threshold, another_very_long_column_name_that_pushes_the_line_wrap_threshold, 'MiXeD' FrOm users Where active = true";
        let two_spaces = super::format_with_options(
            input,
            "generic",
            super::IndentStyle::TwoSpaces,
            super::KeywordCase::Lowercase,
        )
        .unwrap();
        assert!(
            two_spaces.contains("select\n  a_very_long"),
            "{two_spaces:?}"
        );
        assert!(two_spaces.contains("from users"));
        assert!(two_spaces.contains("'MiXeD'"));

        let tabs = super::format_with_options(
            input,
            "generic",
            super::IndentStyle::Tabs,
            super::KeywordCase::Preserve,
        )
        .unwrap();
        assert!(tabs.contains("SeLeCt\n\ta_very_long"), "{tabs:?}");
        assert!(tabs.contains("FrOm users"));
        assert!(tabs.contains("'MiXeD'"));
    }
}
