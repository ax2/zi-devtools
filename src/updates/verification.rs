//! Offline release integrity checks. A checksum is not publisher authentication.
use super::delta_files;
use anyhow::{Context, Result, ensure};
use eframe::egui;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const MAX_SUMS: usize = 8192;
const MAX_ASSET: usize = 128 * 1024 * 1024;

fn names(version: &semver::Version) -> [String; 3] {
    [
        format!("ZiDevTools-{version}-windows-x64.exe"),
        format!("ZiDevTools-{version}-windows-x64.msi"),
        format!("ZiDevToolsMcp-{version}-windows-x64.exe"),
    ]
}

fn checksums(bytes: &[u8], version: &semver::Version) -> Result<BTreeMap<String, String>> {
    ensure!(version.to_string().len() <= 128, "版本字段超过限制");
    ensure!(bytes.len() <= MAX_SUMS, "摘要清单超过8 KiB限制");
    let text = std::str::from_utf8(bytes)
        .context("摘要清单必须是UTF-8文本")?
        .trim_start_matches('\u{feff}');
    let expected = names(version);
    let mut result = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // Match the release workflow's sha256sum format, never paths or shell arguments.
        let (hash, name) = line.split_once("  ").context("摘要清单行格式无效")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "SHA-256格式无效"
        );
        ensure!(
            expected.iter().any(|n| n == name),
            "摘要清单包含其它版本或未知附件"
        );
        ensure!(
            result
                .insert(name.into(), hash.to_ascii_lowercase())
                .is_none(),
            "摘要清单有重复附件"
        );
    }
    ensure!(result.len() == 3, "摘要清单必须包含该版本的三个发布附件");
    Ok(result)
}

fn hash_reader(mut reader: impl Read, size: u64, cancel: &AtomicBool) -> Result<String> {
    ensure!(
        size > 0 && size <= MAX_ASSET as u64,
        "附件大小必须在1字节至128 MiB内"
    );
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 65536];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "校验已取消");
        let n = reader.read(&mut buffer).context("无法完整读取附件")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(total <= size, "附件读取大小超过发布大小");
        hash.update(&buffer[..n]);
    }
    ensure!(total == size, "附件读取大小与发布大小不一致");
    ensure!(!cancel.load(Ordering::Relaxed), "校验已取消");
    Ok(format!("{:x}", hash.finalize()))
}

#[derive(Clone)]
pub struct Verified {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub version: semver::Version,
}

pub fn verify(
    manifest: &Path,
    asset: &Path,
    version: &semver::Version,
    cancel: &AtomicBool,
) -> Result<Verified> {
    ensure!(!cancel.load(Ordering::Relaxed), "校验已取消");
    let mut bytes = Vec::new();
    delta_files::open_regular(manifest, MAX_SUMS)?
        .take((MAX_SUMS + 1) as u64)
        .read_to_end(&mut bytes)?;
    let sums = checksums(&bytes, version)?;
    let name = asset
        .file_name()
        .and_then(|s| s.to_str())
        .context("附件文件名无效")?;
    let expected = sums
        .get(name)
        .context("所选附件不属于清单中的版本，请保留发布文件原名")?;
    let file = delta_files::open_regular(asset, MAX_ASSET)?;
    let size = file.metadata()?.len();
    let sha256 = hash_reader(file, size, cancel)?;
    ensure!(
        &sha256 == expected,
        "SHA-256不匹配，请重新下载附件和同版本摘要清单"
    );
    Ok(Verified {
        name: name.into(),
        size,
        sha256,
        version: version.clone(),
    })
}

struct Job {
    receiver: mpsc::Receiver<Result<Verified, String>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct State {
    version: String,
    manifest: String,
    asset: String,
    pending: Option<Job>,
    result: Option<Result<Verified, String>>,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.pending {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl State {
    pub fn poll(&mut self) {
        let Some(job) = &self.pending else {
            return;
        };
        match job.receiver.try_recv() {
            Ok(result) => {
                self.result = Some(if job.cancel.load(Ordering::Relaxed) {
                    Err("校验已取消".into())
                } else {
                    result
                });
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.result = Some(Err("校验任务未完成，请重试".into()));
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    fn begin(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let version = match semver::Version::parse(self.version.trim().trim_start_matches('v')) {
            Ok(version) if self.version.len() <= 128 => version,
            _ => {
                self.result = Some(Err("请输入正确的发布版本，例如0.81.0".into()));
                return;
            }
        };
        let manifest = std::path::PathBuf::from(&self.manifest);
        let asset = std::path::PathBuf::from(&self.asset);
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let ctx = ctx.clone();
        self.result = None;
        match std::thread::Builder::new()
            .name("release-integrity".into())
            .spawn(move || {
                let result =
                    verify(&manifest, &asset, &version, &worker).map_err(|e| e.to_string());
                let _ = tx.send(result);
                ctx.request_repaint();
            }) {
            Ok(_) => self.pending = Some(Job { receiver, cancel }),
            Err(_) => self.result = Some(Err("无法启动校验任务".into())),
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("校验已下载的发布附件").strong());
        ui.weak(
            "从同一个官方发布页下载附件与SHA256SUMS.txt，保留附件原名。只读取文件，不安装或运行。",
        );
        let mut changed = false;
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label("发布版本");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.version)
                            .desired_width(180.0)
                            .hint_text("例如0.81.0"),
                    )
                    .changed();
            });
            for (label, value) in [
                ("摘要清单", &mut self.manifest),
                ("EXE / MSI", &mut self.asset),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    changed |= ui
                        .add(egui::TextEdit::singleline(value).desired_width(450.0))
                        .changed();
                    #[cfg(windows)]
                    if ui.button("选择…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            *value = path.to_string_lossy().into_owned();
                            changed = true;
                        }
                    }
                });
            }
        });
        if changed {
            self.result = None;
        }
        if let Some(job) = &self.pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("后台读取并校验…");
                if ui.button("取消校验").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        } else if ui.button("开始完整性校验").clicked() {
            self.begin(ui.ctx());
        }
        if let Some(result) = &self.result {
            match result {
                Ok(verified) => {
                    ui.label(egui::RichText::new("SHA-256校验通过").strong());
                    ui.label(format!(
                        "{} · {} · {:.2} MiB",
                        verified.version,
                        verified.name,
                        verified.size as f64 / 1_048_576.0
                    ));
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(&verified.sha256);
                        if ui.small_button("复制摘要").clicked() {
                            ui.ctx().copy_text(verified.sha256.clone());
                        }
                    });
                    ui.weak("只证明本次读取内容与所选清单一致。文件后续修改需重新校验；尚未验证发布者签名。");
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, failure: bool) {
        self.version = "0.81.0".into();
        self.manifest = "C:\\Example\\SHA256SUMS.txt".into();
        self.asset = "C:\\Example\\ZiDevTools-0.81.0-windows-x64.exe".into();
        self.result = Some(if failure {
            Err("SHA-256不匹配，请重新下载附件和同版本摘要清单（合成演示）".into())
        } else {
            Ok(Verified {
                version: semver::Version::new(0, 81, 0),
                name: "ZiDevTools-0.81.0-windows-x64.exe（合成演示）".into(),
                size: 27_000_000,
                sha256: "a".repeat(64),
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(version: &semver::Version, hash: &str) -> Vec<u8> {
        names(version)
            .iter()
            .map(|name| format!("{hash}  {name}\r\n"))
            .collect::<String>()
            .into_bytes()
    }
    #[test]
    fn manifest_rejects_ambiguous_or_foreign_names() {
        let version = semver::Version::new(1, 2, 3);
        let valid = manifest(&version, &"a".repeat(64));
        assert_eq!(checksums(&valid, &version).unwrap().len(), 3);
        let bom = [vec![0xef, 0xbb, 0xbf], manifest(&version, &"A".repeat(64))].concat();
        assert!(
            checksums(&bom, &version)
                .unwrap()
                .values()
                .all(|v| v == &"a".repeat(64))
        );
        for invalid in [
            Vec::new(),
            vec![b'a'; MAX_SUMS + 1],
            [valid.clone(), valid.clone()].concat(),
            String::from_utf8(valid.clone())
                .unwrap()
                .replace("1.2.3", "1.2.4")
                .into_bytes(),
            String::from_utf8(valid.clone())
                .unwrap()
                .replace("  Zi", "  ../Zi")
                .into_bytes(),
            manifest(&version, &"g".repeat(64)),
            vec![255],
        ] {
            assert!(checksums(&invalid, &version).is_err());
        }
    }
    #[test]
    fn stream_checks_length_io_and_cancellation() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            hash_reader(&b"abc"[..], 3, &cancel).unwrap(),
            format!("{:x}", Sha256::digest(b"abc"))
        );
        assert!(hash_reader(&b"abc"[..], 2, &cancel).is_err());
        assert!(hash_reader(&b"abc"[..], 4, &cancel).is_err());
        assert!(hash_reader(&b""[..], 0, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(hash_reader(&b"abc"[..], 3, &cancel).is_err());
    }
    #[test]
    fn reading_errors_and_midstream_cancel_never_succeed() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture read failure"))
            }
        }
        let cancel = AtomicBool::new(false);
        assert!(hash_reader(Broken, 1, &cancel).is_err());
        struct Cancelling<'a>(&'a AtomicBool);
        impl Read for Cancelling<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer[0] = 0;
                self.0.store(true, Ordering::Relaxed);
                Ok(1)
            }
        }
        assert!(hash_reader(Cancelling(&cancel), 1, &cancel).is_err());
    }
    #[test]
    fn verifies_real_files_without_modifying_them_and_rejects_tampering() {
        let root = std::env::temp_dir().join(format!("zi-release-check-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let version = semver::Version::new(1, 2, 3);
        let sums = root.join("SHA256SUMS.txt");
        let asset = root.join(&names(&version)[0]);
        let payload = b"synthetic executable, never executed";
        let original = manifest(&version, &format!("{:x}", Sha256::digest(payload)));
        std::fs::write(&sums, &original).unwrap();
        std::fs::write(&asset, payload).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            verify(&sums, &asset, &version, &cancel).unwrap().size,
            payload.len() as u64
        );
        assert_eq!(std::fs::read(&sums).unwrap(), original);
        assert_eq!(std::fs::read(&asset).unwrap(), payload);
        std::fs::write(&asset, b"tampered").unwrap();
        assert!(verify(&sums, &asset, &version, &cancel).is_err());
        assert!(verify(&sums, &root, &version, &cancel).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
