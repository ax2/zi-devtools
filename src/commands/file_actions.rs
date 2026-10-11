//! Shared file receivers for import UI, external launch and Shell plans.
use std::path::PathBuf;

pub const MAX_FILES: usize = 64;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Csv,
    Tsv,
    JsonData,
    Json,
    Java,
    Python,
    Threads,
    Gc,
    Sql,
    Celery,
    Files,
}
impl Target {
    pub fn action_id(self) -> &'static str {
        match self {
            Self::Csv => "import:csv",
            Self::Tsv => "import:tsv",
            Self::JsonData => "import:json-data",
            Self::Json => "import:json",
            Self::Java => "import:java-trace",
            Self::Python => "import:django-trace",
            Self::Threads => "import:java-threads",
            Self::Gc => "import:java-gc",
            Self::Sql => "import:django-sql",
            Self::Celery => "import:celery",
            Self::Files => "import:files",
        }
    }
    pub fn from_action_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|target| target.action_id() == id)
    }
    pub fn tool_id(self) -> &'static str {
        match self {
            Self::Csv | Self::Tsv | Self::JsonData => "data",
            Self::Json => "json",
            Self::Java => "java-trace",
            Self::Python => "django-trace",
            Self::Threads => "java-thread-dump",
            Self::Gc => "java-gc-log",
            Self::Sql => "django-sql",
            Self::Celery => "celery-diagnostics",
            Self::Files => "files",
        }
    }
    pub fn max_files(self) -> usize {
        if self == Self::Files { MAX_FILES } else { 1 }
    }
    pub fn validate_selection(self, paths: &[PathBuf]) -> Result<(), String> {
        if paths.is_empty() || paths.len() > self.max_files() {
            return Err(format!(
                "{}每次接收1–{}个文件；多个文件可选择文件校验",
                self.label(),
                self.max_files()
            ));
        }
        Ok(())
    }
    pub const ALL: [Self; 11] = [
        Self::Csv,
        Self::Tsv,
        Self::JsonData,
        Self::Json,
        Self::Java,
        Self::Python,
        Self::Threads,
        Self::Gc,
        Self::Sql,
        Self::Celery,
        Self::Files,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV 数据表",
            Self::Tsv => "TSV 数据表",
            Self::JsonData => "JSON 对象数组",
            Self::Json => "JSON 格式化",
            Self::Java => "Java 异常链",
            Self::Python => "Python / Django 异常",
            Self::Threads => "JVM 线程转储",
            Self::Gc => "GC 日志",
            Self::Sql => "Django SQL 分析",
            Self::Celery => "Celery 日志",
            Self::Files => "文件校验",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_receivers_are_unique_implemented_and_bounded() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/tools.json")).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for target in Target::ALL {
            assert!(ids.insert(target.action_id()));
            assert_eq!(Target::from_action_id(target.action_id()), Some(target));
            assert!(
                catalog["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["id"] == target.tool_id() && tool["status"] == "implemented")
            );
            assert!(target.validate_selection(&[]).is_err());
            assert!(
                target
                    .validate_selection(&vec![PathBuf::from("file"); target.max_files()])
                    .is_ok()
            );
            assert!(
                target
                    .validate_selection(&vec![PathBuf::from("file"); target.max_files() + 1])
                    .is_err()
            );
        }
        assert!(Target::from_action_id("import:unknown").is_none());
        assert!(Target::from_action_id("IMPORT:CSV").is_none());
    }
}
