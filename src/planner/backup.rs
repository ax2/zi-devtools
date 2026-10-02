use super::*;
use anyhow::Context;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const LIMIT: usize = 40 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    format: String,
    version: u32,
    pub created_at: i64,
    #[serde(deserialize_with = "read_records")]
    pub records: Vec<Item>,
}
fn read_records<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Item>, D::Error> {
    struct Records;
    impl<'de> serde::de::Visitor<'de> for Records {
        type Value = Vec<Item>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("最多 2000 条备忘或日程")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut records = Vec::new();
            while let Some(item) = seq.next_element::<Item>()? {
                if records.len() == MAX_ITEMS {
                    return Err(serde::de::Error::custom("备份超过 2000 条"));
                }
                item.validate().map_err(serde::de::Error::custom)?;
                records.push(item);
            }
            Ok(records)
        }
    }
    deserializer.deserialize_seq(Records)
}
pub(super) fn validate_records(records: &[Item]) -> Result<()> {
    ensure!(records.len() <= MAX_ITEMS, "记录超过 2000 条");
    let mut ids = HashSet::new();
    let mut bytes = 0;
    for item in records {
        item.validate()?;
        ensure!(
            item.revision > 0 && ids.insert(&item.id),
            "备份含未保存或重复标识的记录"
        );
        bytes += serde_json::to_vec(item)?.len();
        ensure!(bytes <= 32 * 1024 * 1024, "记录超过 32 MiB 保存容量");
    }
    Ok(())
}
impl Document {
    pub fn new(records: Vec<Item>) -> Result<Self> {
        validate_records(&records)?;
        Ok(Self {
            format: "zi-devtools-planner".into(),
            version: 1,
            created_at: Local::now().timestamp(),
            records,
        })
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.format == "zi-devtools-planner" && self.version == 1,
            "不支持此备份格式或版本"
        );
        ensure!(valid_timestamp(self.created_at), "备份时间无效");
        validate_records(&self.records)
    }
}
fn json_path(path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json")),
        "请选择 .json 备份文件"
    );
    Ok(())
}
pub(super) fn read(path: &Path) -> Result<Document> {
    json_path(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= LIMIT as u64,
        "备份需要是 40 MiB 以内的普通文件"
    );
    let file = File::open(path)?;
    ensure!(file.metadata()?.is_file(), "备份路径不是普通文件");
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "备份超过 40 MiB");
    let document: Document = serde_json::from_slice(&bytes).context("备份 JSON 无效")?;
    document.validate()?;
    Ok(document)
}
pub(super) fn write(path: &Path, document: &Document) -> Result<()> {
    json_path(path)?;
    document.validate()?;
    let bytes = serde_json::to_vec(document)?;
    ensure!(bytes.len() <= LIMIT, "备份超过 40 MiB");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法创建备份，请选择新的文件名；已有文件不会覆盖")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .context("备份写入未完成，目标可能不完整；请检查后使用新文件名重试")?;
    Ok(())
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    KeepCurrent,
    BackupWins,
    ReplaceAll,
}
pub(super) struct Change {
    pub id: String,
    pub label: &'static str,
}
pub(super) struct Plan {
    pub records: Vec<Item>,
    pub changes: Vec<Change>,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub kept: usize,
}
fn same_content(a: &Item, b: &Item) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.revision = 0;
    b.revision = 0;
    a == b
}
pub(super) fn plan(current: &[Item], doc: &Document, mode: Mode, reminders: bool) -> Result<Plan> {
    doc.validate()?;
    validate_records(current)?;
    let existing: HashMap<_, _> = current.iter().map(|i| (i.id.as_str(), i)).collect();
    let incoming: HashSet<_> = doc.records.iter().map(|i| i.id.as_str()).collect();
    let mut result = Plan {
        records: Vec::new(),
        changes: Vec::new(),
        added: 0,
        updated: 0,
        removed: 0,
        kept: 0,
    };
    for old in current {
        if !incoming.contains(old.id.as_str()) {
            if mode == Mode::ReplaceAll {
                result.removed += 1;
                result.changes.push(Change {
                    id: old.id.clone(),
                    label: "移除",
                });
            } else {
                result.records.push(old.clone());
                result.kept += 1;
            }
        }
    }
    for source in &doc.records {
        let old = existing.get(source.id.as_str()).copied();
        if mode == Mode::KeepCurrent
            && let Some(old) = old
        {
            result.records.push(old.clone());
            result.kept += 1;
            continue;
        }
        let mut item = source.clone();
        if !reminders && let Some(schedule) = &mut item.schedule {
            schedule.remind = false;
        }
        if let Some(old) = old {
            if same_content(&item, old) {
                result.records.push(old.clone());
                result.kept += 1;
                continue;
            }
            result.updated += 1;
            result.changes.push(Change {
                id: item.id.clone(),
                label: "覆盖",
            });
        } else {
            result.added += 1;
            result.changes.push(Change {
                id: item.id.clone(),
                label: "新增",
            });
        }
        result.records.push(item);
    }
    // Changed records receive 19-digit version tokens at commit; include that
    // overhead in the preview's capacity check rather than failing after consent.
    let changed: HashSet<_> = result
        .changes
        .iter()
        .filter(|c| c.label != "移除")
        .map(|c| c.id.as_str())
        .collect();
    let mut projected = result.records.clone();
    for item in &mut projected {
        if changed.contains(item.id.as_str()) {
            item.revision = 1i64 << 62;
        }
    }
    validate_records(&projected)?;
    Ok(result)
}
pub(super) struct RestoreReview {
    pub document: Document,
    pub current: Vec<Item>,
    pub mode: Mode,
    pub reminders: bool,
    pub plan: std::result::Result<Plan, String>,
    pub confirmed: bool,
}
impl RestoreReview {
    pub fn new(document: Document, current: Vec<Item>) -> Self {
        let plan = plan(&current, &document, Mode::KeepCurrent, false).map_err(|e| e.to_string());
        Self {
            document,
            current,
            mode: Mode::KeepCurrent,
            reminders: false,
            plan,
            confirmed: false,
        }
    }
    pub fn rebuild(&mut self) {
        self.plan = plan(&self.current, &self.document, self.mode, self.reminders)
            .map_err(|e| e.to_string());
        self.confirmed = false;
    }
}
pub(super) enum Review {
    Export(Document),
    Restore(RestoreReview),
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_backup(&mut self, scene: usize) {
        self.preview(false, false);
        self.items[1].trash = true;
        if scene < 222 {
            self.backup_review = Some(Review::Export(Document::new(self.items.clone()).unwrap()));
        } else {
            let mut records = vec![self.items[0].clone(), self.items[2].clone()];
            records[0].title = "来自备份的网站检查清单".into();
            records[0].body = "备份中的修改：补充移动端 WebP 检查。".into();
            let mut extra = records[0].clone();
            extra.id = uuid::Uuid::new_v4().to_string();
            extra.title = "从备份恢复的新备忘".into();
            records.push(extra);
            let mut review =
                RestoreReview::new(Document::new(records).unwrap(), self.items.clone());
            if scene >= 224 {
                review.mode = Mode::ReplaceAll;
                review.rebuild();
            }
            self.backup_review = Some(Review::Restore(review));
        }
        self.focus_editor = false;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_backup_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview(false, false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-backup-ui-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut shared = Item::new(None);
                shared.title = "本机原记录".into();
                store::save(&self.path, shared).unwrap();
                let mut local = Item::new(None);
                local.title = "仅在本机的记录".into();
                store::save(&self.path, local).unwrap();
                let current = store::load(&self.path).unwrap();
                let mut incoming = current
                    .iter()
                    .find(|i| i.title == "本机原记录")
                    .unwrap()
                    .clone();
                incoming.title = "备份里的内容".into();
                let mut extra = incoming.clone();
                extra.id = uuid::Uuid::new_v4().to_string();
                extra.title = "备份新增记录".into();
                write(
                    &self.path.with_file_name("backup.json"),
                    &Document::new(vec![incoming, extra]).unwrap(),
                )
                .unwrap();
                self.draft = None;
                self.original = None;
                self.items = current;
                self.read_backup(self.path.with_file_name("backup.json"))
                    .unwrap();
            }
            1 => {
                assert!(self.backup_review.is_none() && self.pending.is_none());
                assert!(
                    store::load(&self.path)
                        .unwrap()
                        .iter()
                        .any(|i| i.title == "本机原记录")
                );
                self.read_backup(self.path.with_file_name("backup.json"))
                    .unwrap();
            }
            2 => {
                assert!(
                    self.restore_backup().is_err(),
                    "replacement requires explicit acknowledgment"
                );
            }
            3 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && self.backup_review.is_none() && !self.has_unsaved());
                let stored = store::load(&self.path).unwrap();
                let document = read(&self.path.with_file_name("backup.json")).unwrap();
                assert_eq!(stored.len(), 2);
                assert!(self.draft.is_none());
                for item in &document.records {
                    assert!(
                        stored
                            .iter()
                            .any(|i| same_content(i, item) && i.revision != item.revision)
                    );
                }
                std::fs::remove_file(self.path.with_file_name("backup.json")).unwrap();
                std::fs::remove_file(&self.path).unwrap();
                std::fs::remove_dir(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS backup UI: actual cancel, replace-mode selection, overwrite acknowledgment and restore click; transactional result matches preview with fresh versions"
                );
            }
            _ => unreachable!(),
        }
        true
    }
    fn backup_available(&self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none() && !self.has_unsaved(),
            "请先保存或放弃当前编辑，并等待读写完成"
        );
        ensure!(
            self.purge_review.is_none()
                && self.export_review.is_none()
                && self.backup_review.is_none(),
            "请先关闭其他确认窗口"
        );
        Ok(())
    }
    pub(super) fn prepare_backup(&mut self) -> Result<()> {
        self.backup_available()?;
        let path = self.path.clone();
        self.start_file(move || {
            Ok(Reply::BackupReady(Box::new(Review::Export(Document::new(
                store::load(&path)?,
            )?))))
        });
        Ok(())
    }
    pub(super) fn read_backup(&mut self, path: PathBuf) -> Result<()> {
        self.backup_available()?;
        let database = self.path.clone();
        self.start_file(move || {
            let doc = read(&path)?;
            let current = store::load(&database)?;
            Ok(Reply::BackupReady(Box::new(Review::Restore(
                RestoreReview::new(doc, current),
            ))))
        });
        Ok(())
    }
    pub(super) fn save_backup(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            self.pending.is_none() && !self.has_unsaved(),
            "请等待读写完成并保存当前编辑"
        );
        let Some(Review::Export(document)) = self.backup_review.as_ref() else {
            anyhow::bail!("请先预览备份");
        };
        let document = document.clone();
        self.backup_review = None;
        self.start_file(move || {
            write(&path, &document)?;
            Ok(Reply::BackupSaved(path, document.records.len()))
        });
        Ok(())
    }
    pub(super) fn restore_backup(&mut self) -> Result<()> {
        ensure!(
            self.pending.is_none() && !self.has_unsaved(),
            "请等待读写完成并保存当前编辑"
        );
        let Some(Review::Restore(review)) = self.backup_review.as_ref() else {
            anyhow::bail!("请先预览恢复影响");
        };
        let plan = review
            .plan
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?;
        ensure!(!plan.changes.is_empty(), "无需恢复，当前记录没有变化");
        ensure!(
            (plan.updated == 0 && plan.removed == 0) || review.confirmed,
            "请明确确认覆盖或移除现有记录"
        );
        let records = plan.records.clone();
        let expected = review.current.clone();
        let path = self.path.clone();
        self.backup_review = None;
        self.start_file(move || store::restore(&path, &expected, &records).map(Reply::Restored));
        Ok(())
    }
}
