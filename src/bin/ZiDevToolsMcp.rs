use anyhow::{Context, Result, ensure};
use std::{io, path::PathBuf};
use zi_devtools::knowledge_mcp::{self, Config};

fn config() -> Result<Config> {
    let mut config = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sources" => {
                config.sources = PathBuf::from(args.next().context("--sources needs a path")?)
            }
            "--index" => config.index = PathBuf::from(args.next().context("--index needs a path")?),
            _ => anyhow::bail!("unknown MCP server argument"),
        }
    }
    ensure!(
        config.sources.is_absolute() && config.index.is_absolute(),
        "MCP file paths must be absolute"
    );
    Ok(config)
}

fn main() -> Result<()> {
    let config = config()?;
    knowledge_mcp::serve(
        io::BufReader::new(io::stdin().lock()),
        io::stdout().lock(),
        &config,
    )
}
