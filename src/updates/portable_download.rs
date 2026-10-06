//! Online preparation selects only a signed exact baseline; patch failures never silently fall back.
use super::{Report, delta_files, download, portable, signed_delta};
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
pub(super) struct Outcome {
    pub prepared: portable::Prepared,
    pub transfer: String,
}
pub(super) fn prepare(
    report: Report,
    cancel: &AtomicBool,
    progress: [&AtomicU64; 2],
    total: &AtomicU64,
) -> Result<Outcome> {
    let directory = portable::current_directory()?;
    let current = env!("CARGO_PKG_VERSION");
    ensure!(
        report
            .version
            .cmp_precedence(&semver::Version::parse(current)?)
            .is_gt(),
        "没有更高版本可安装"
    );
    let (target, raw, sig) = download::signed_release(&report, cancel)?;
    let sources = [
        delta_files::read(&directory.join("ZiDevTools.exe"), false, cancel)?,
        delta_files::read(&directory.join("ZiDevToolsMcp.exe"), false, cancel)?,
    ];
    let present: Vec<_> = ["update-delta.json", "update-delta.sig"]
        .iter()
        .map(|name| report.assets.iter().any(|a| a.name == *name))
        .collect();
    ensure!(present[0] == present[1], "增量签名文件不完整，拒绝回退");
    let mut selected = None;
    let reason = if present[0] {
        let delta_raw = download::metadata(
            &report,
            "update-delta.json",
            super::signed::MAX_MANIFEST,
            cancel,
        )?;
        let delta_sig = download::metadata(
            &report,
            "update-delta.sig",
            super::signed::MAX_SIGNATURE,
            cancel,
        )?;
        let delta = signed_delta::verify_official(&delta_raw, &delta_sig, &target, &raw)?;
        if signed_delta::baseline(&delta, current, [&sources[0], &sources[1]]) {
            total.store(
                delta.files.iter().map(|e| e.patch.size).sum(),
                Ordering::Relaxed,
            );
            selected = Some(delta);
            ""
        } else {
            "发布增量基线与当前版本或文件不同"
        }
    } else {
        "发布没有增量包"
    };
    let mut binaries = [Vec::new(), Vec::new()];
    let transfer = if let Some(delta) = selected {
        for i in 0..2 {
            let patch =
                download::authenticated_bytes(&report, &delta.files[i].patch, cancel, progress[i])?;
            binaries[i] = signed_delta::reconstruct(&delta, i, &sources[i], &patch, cancel)?;
        }
        let bytes = delta.files.iter().map(|e| e.patch.size).sum::<u64>();
        let full = delta.files.iter().map(|e| e.target.size).sum::<u64>();
        format!(
            "增量准备 · 程序内容下载 {}，完整EXE为 {}，节省 {:.1}%（不含元数据）",
            super::size(bytes),
            super::size(full),
            100.0 * (1.0 - bytes as f64 / full as f64)
        )
    } else {
        let files: Vec<_> = ["ZiDevTools", "ZiDevToolsMcp"]
            .iter()
            .map(|prefix| {
                target
                    .manifest
                    .files
                    .iter()
                    .find(|f| f.name == format!("{prefix}-{}-windows-x64.exe", report.version))
                    .expect("validated complete manifest")
            })
            .collect();
        total.store(files.iter().map(|f| f.size).sum(), Ordering::Relaxed);
        for i in 0..2 {
            binaries[i] = download::authenticated_bytes(&report, files[i], cancel, progress[i])?;
        }
        format!(
            "完整包准备 · {reason}；程序内容下载 {}（不含元数据）",
            super::size(total.load(Ordering::Relaxed))
        )
    };
    // Both baselines must still be the exact bytes used to select/reconstruct.
    for (i, name) in ["ZiDevTools.exe", "ZiDevToolsMcp.exe"].iter().enumerate() {
        ensure!(
            portable::hash_file(&directory.join(name))? == portable::digest(&sources[i]),
            "准备期间本机程序改变，请重新准备"
        );
    }
    let prepared = portable::prepare(
        &directory,
        current,
        &raw,
        &sig,
        [&binaries[0], &binaries[1]],
        &sources[0],
        cancel,
    )?;
    Ok(Outcome { prepared, transfer })
}
