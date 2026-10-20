use anyhow::{Result, anyhow};
use sqlformat::{Dialect as FormatterDialect, FormatOptions, Indent, QueryParams};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndentStyle {
    TwoSpaces,
    #[default]
    FourSpaces,
    Tabs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeywordCase {
    #[default]
    Uppercase,
    Lowercase,
    Preserve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    let dialect = Dialect::parse(dialect)?;
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
    let output = sqlformat::format(&protected_input, &QueryParams::None, &options);
    Ok(restore_protected_literals(&output, &literals))
}

fn protect_dialect_regions(
    input: &str,
    dialect: Dialect,
) -> Result<(String, Vec<(String, String)>)> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut literals = Vec::new();
    let mut scan = 0;
    let mut copy_from = 0;

    while scan < bytes.len() {
        match bytes[scan] {
            b'\'' | b'"' => scan = quoted_end(bytes, scan, bytes[scan]),
            b'-' if bytes.get(scan + 1) == Some(&b'-') => {
                scan = line_comment_end(bytes, scan + 2);
            }
            b'/' if bytes.get(scan + 1) == Some(&b'*') => {
                scan = block_comment_end(bytes, scan + 2);
            }
            b'$' if dialect == Dialect::PostgreSql => {
                let Some((delimiter, body_start)) = dollar_delimiter(bytes, scan) else {
                    scan += 1;
                    continue;
                };
                let body = &bytes[body_start..];
                let Some(close_offset) = body
                    .windows(delimiter.len())
                    .position(|window| window == delimiter)
                else {
                    return Err(anyhow!("PostgreSQL dollar 引号缺少结束标记"));
                };
                let end = body_start + close_offset + delimiter.len();
                output.push_str(&input[copy_from..scan]);

                let mut placeholder = format!("__ZICODE_SQLFMT_LITERAL_{}__", literals.len());
                while input.contains(&placeholder) {
                    placeholder.insert(0, '_');
                }
                output.push_str(&placeholder);
                literals.push((placeholder, input[scan..end].to_owned()));
                scan = end;
                copy_from = end;
            }
            b'[' if dialect == Dialect::Sqlite => {
                if let Some(end) = bracket_identifier_end(bytes, scan) {
                    output.push_str(&input[copy_from..scan]);
                    let mut placeholder = format!("__ZICODE_SQLFMT_LITERAL_{}__", literals.len());
                    while input.contains(&placeholder) {
                        placeholder.insert(0, '_');
                    }
                    output.push_str(&placeholder);
                    literals.push((placeholder, input[scan..end].to_owned()));
                    scan = end;
                    copy_from = end;
                } else {
                    scan += 1;
                }
            }
            _ => scan += 1,
        };
    }

    if literals.is_empty() {
        Ok((input.to_owned(), literals))
    } else {
        output.push_str(&input[copy_from..]);
        Ok((output, literals))
    }
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

fn restore_protected_literals(output: &str, literals: &[(String, String)]) -> String {
    let mut restored = String::with_capacity(output.len());
    let mut cursor = 0;

    loop {
        let next = literals
            .iter()
            .filter_map(|(placeholder, literal)| {
                output[cursor..]
                    .find(placeholder)
                    .map(|offset| (cursor + offset, placeholder, literal))
            })
            .min_by_key(|(position, _, _)| *position);
        let Some((position, placeholder, literal)) = next else {
            restored.push_str(&output[cursor..]);
            return restored;
        };
        restored.push_str(&output[cursor..position]);
        restored.push_str(literal);
        cursor = position + placeholder.len();
    }
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
