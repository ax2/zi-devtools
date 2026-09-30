//! Synthetic knowledge index for manual, isolated MCP client compatibility checks.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{path::PathBuf, sync::atomic::AtomicBool};
use zi_devtools::{knowledge_index, knowledge_sources};

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("fixture directory required")?,
    );
    ensure!(
        root.is_absolute() && root.is_dir(),
        "fixture directory must exist"
    );
    ensure!(
        std::fs::read_dir(&root)?.next().is_none(),
        "fixture directory must be empty"
    );
    let notes = root.join("notes");
    std::fs::create_dir(&notes)?;
    std::fs::write(
        notes.join("guide.md"),
        "# Synthetic MCP guide\n\nRustMcpProbe confirms this synthetic local document.",
    )?;
    let mut sources = Vec::new();
    let mut source = knowledge_sources::add_source(&mut sources, &notes, "Fixture notes", "")?;
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false))?);
    let registry = root.join("sources.json");
    std::fs::write(
        &registry,
        serde_json::to_vec(&json!({
            "schema": "zi-devtools-knowledge-sources",
            "version": 1,
            "sources": [source]
        }))?,
    )?;
    knowledge_index::sync_all(
        &root.join("index.sqlite3"),
        &knowledge_sources::read_sources(&registry)?,
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )?;
    println!("Synthetic MCP fixture ready");
    Ok(())
}
