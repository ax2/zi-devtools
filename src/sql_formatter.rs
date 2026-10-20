pub use zi_text_core::sql::{IndentStyle, KeywordCase};

pub fn format(input: &str, dialect: &str) -> anyhow::Result<String> {
    zi_text_core::sql::format(input, dialect)
}

pub fn format_with_options(
    input: &str,
    dialect: &str,
    indent_style: IndentStyle,
    keyword_case: KeywordCase,
) -> anyhow::Result<String> {
    zi_text_core::sql::format_with_options(input, dialect, indent_style, keyword_case)
}
