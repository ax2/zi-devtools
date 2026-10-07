//! Disposable real-process concurrency fixture; never reads normal user preferences.
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};
use zi_devtools::preferences::Preferences;

struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "writer") {
        ensure!(args.len() == 4, "writer needs directory worker tool");
        let directory = PathBuf::from(&args[1]);
        let worker = args[2].to_str().context("worker id")?;
        let tool = args[3].to_str().context("tool id")?;
        let path = directory.join("preferences.json");
        let mut prefs = Preferences::load(&path);
        std::fs::write(directory.join(format!("{worker}.ready")), b"ready")?;
        let began = Instant::now();
        while !directory.join("go").is_file() {
            ensure!(
                began.elapsed() < Duration::from_secs(30),
                "fixture barrier timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        prefs.toggle(tool);
        if worker == "a" {
            prefs.recorder_auto_stop_minutes = 15;
        }
        for _ in 0..40 {
            prefs.visit(tool);
            prefs.save(&path)?;
        }
        return Ok(());
    }
    ensure!(args.len() == 1, "provide disposable fixture root");
    let directory = PathBuf::from(&args[0]).join(format!("preferences-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("preferences.json");
    let mut seed = Preferences::load(&path);
    seed.light = true;
    seed.save(&path)?;
    let mut children = Children(Vec::new());
    for (worker, tool) in [("a", "json"), ("b", "json"), ("c", "base64")] {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("writer").arg(&directory).arg(worker).arg(tool);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        children.0.push(command.spawn()?);
    }
    let began = Instant::now();
    while ["a", "b", "c"]
        .iter()
        .any(|id| !directory.join(format!("{id}.ready")).is_file())
    {
        ensure!(
            began.elapsed() < Duration::from_secs(30),
            "fixture readiness timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(directory.join("go"), b"go")?;
    for child in &mut children.0 {
        ensure!(child.wait()?.success(), "writer failed");
    }
    let merged = Preferences::load(&path);
    ensure!(
        merged.usage.get("json") == Some(&80) && merged.usage.get("base64") == Some(&40),
        "lost or duplicated visits"
    );
    ensure!(
        merged.favorites.len() == 2
            && merged.favorites.contains(&"json".to_owned())
            && merged.favorites.contains(&"base64".to_owned()),
        "lost favorites"
    );
    ensure!(
        merged.recent.len() == 2 && merged.light && merged.recorder_auto_stop_minutes == 15,
        "lost recent or independent settings"
    );
    ensure!(
        !std::fs::read_dir(&directory)?.any(|file| file
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")),
        "temporary file leaked"
    );
    println!(
        "PASS three real Windows writers: 120 visits, same-tool count 80, both favorites, bounded unique recent, independent settings, no temporary files"
    );
    Ok(())
}
