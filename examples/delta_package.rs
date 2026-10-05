//! Local release-engineering helper; never downloads, installs or executes output.
use anyhow::{Context, Result, ensure};
use std::{path::Path, sync::atomic::AtomicBool};
use zi_devtools::updates::{delta, delta_files};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let cancel = AtomicBool::new(false);
    match args.first().map(String::as_str) {
        Some("compare") if args.len() == 5 => {
            use flate2::{Compression, write::DeflateEncoder};
            use sha2::{Digest, Sha256};
            use std::io::Write;
            let source = delta_files::read(Path::new(&args[1]), false, &cancel)?;
            let target = delta_files::read(Path::new(&args[2]), false, &cancel)?;
            let (header, patch) = delta::create(&source, &target, &args[3], &args[4], &cancel)?;
            ensure!(
                delta::reconstruct(Some(&source), &patch, &cancel)?.1 == target,
                "比较自检失败"
            );
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            for chunk in target.chunks(65536) {
                encoder.write_all(chunk)?;
            }
            let full = encoder.finish()?;
            let mut full_header = header.clone();
            full_header.kind = delta::Kind::Full;
            full_header.copied_bytes = 0;
            full_header.literal_bytes = target.len() as u64;
            full_header.payload_size = full.len() as u64;
            full_header.payload_sha256 = format!("{:x}", Sha256::digest(&full));
            let full_size = 12 + serde_json::to_vec(&full_header)?.len() + full.len();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"header":header,"raw_target_bytes":target.len(),"selected_package_bytes":patch.len(),"compressed_full_package_bytes":full_size,"saving_against_compressed_full_bytes":full_size-patch.len()})
                )?
            );
        }
        Some("create") if args.len() == 6 => {
            let source = delta_files::read(Path::new(&args[1]), false, &cancel)?;
            let target = delta_files::read(Path::new(&args[2]), false, &cancel)?;
            let (header, patch) = delta::create(&source, &target, &args[3], &args[4], &cancel)?;
            ensure!(
                delta::reconstruct(Some(&source), &patch, &cancel)?.1 == target,
                "生成自检失败"
            );
            let path = delta_files::save_new(Path::new(&args[5]), &patch, &cancel)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"header":header,"package_size":patch.len(),"path":path})
                )?
            );
        }
        Some("rebuild") if args.len() == 4 => {
            let source = if args[1] == "-" {
                None
            } else {
                Some(delta_files::read(Path::new(&args[1]), false, &cancel)?)
            };
            let patch = delta_files::read(Path::new(&args[2]), true, &cancel)?;
            let (header, target) = delta::reconstruct(source.as_deref(), &patch, &cancel)?;
            let path = delta_files::save_new(Path::new(&args[3]), &target, &cancel)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"header":header,"path":path}))?
            );
        }
        Some("inspect") if args.len() == 2 => {
            let patch = delta_files::read(Path::new(&args[1]), true, &cancel)?;
            let (header, _) = delta::inspect(&patch, &cancel)?;
            println!("{}", serde_json::to_string_pretty(&header)?);
        }
        _ => anyhow::bail!(
            "用法: create SOURCE TARGET SOURCE_VERSION TARGET_VERSION OUTPUT | compare SOURCE TARGET SOURCE_VERSION TARGET_VERSION | rebuild SOURCE_OR_- PATCH OUTPUT | inspect PATCH"
        ),
    }
    std::io::Write::flush(&mut std::io::stdout()).context("输出失败")?;
    Ok(())
}
