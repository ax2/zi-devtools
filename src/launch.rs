//! Shared literal startup parsing for desktop launches and native Shell receivers.
use crate::config::default_config_path;
use anyhow::{Context, Result};

pub struct Arguments {
    pub tool: Option<String>,
    pub open_files: Vec<std::path::PathBuf>,
    pub file_action: Option<crate::intake::Target>,
    pub config_path: std::path::PathBuf,
    pub restore_services: bool,
    pub fixture_port: Option<u16>,
    pub fixture_shutdown_port: Option<u16>,
    pub import_config: Option<std::path::PathBuf>,
    pub force_import: bool,
}

pub fn parse_args() -> Result<Arguments> {
    parse_args_from(std::env::args_os().skip(1))
}

pub fn parse_args_from(input: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Arguments> {
    let mut tool = None;
    let mut open_files = Vec::new();
    let mut file_action = None;
    let mut config_path = default_config_path();
    let mut restore_services = true;
    let mut fixture_port = None;
    let mut fixture_shutdown_port = None;
    let mut import_config = None;
    let mut force_import = false;
    let mut args = input.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str().context("参数不是有效 Unicode")? {
            "--file-action" => {
                anyhow::ensure!(file_action.is_none(), "--file-action cannot be repeated");
                let id = args.next().context("--file-action requires an action ID")?;
                file_action = Some(
                    crate::intake::Target::from_action_id(
                        id.to_str().context("文件动作编号不是有效 Unicode")?,
                    )
                    .context("未知文件动作")?,
                );
            }
            "--open-files" => {
                // Terminal argument: everything after it is a path, never another option.
                for value in args.by_ref().take(65) {
                    anyhow::ensure!(
                        !value.is_empty() && !value.as_encoded_bytes().contains(&0),
                        "文件路径为空或包含无效字符"
                    );
                    open_files.push(std::path::absolute(std::path::PathBuf::from(value))?);
                }
                anyhow::ensure!(
                    (1..=64).contains(&open_files.len()),
                    "--open-files requires 1–64 file paths"
                );
                break;
            }
            "--tool" => {
                let id = args
                    .next()
                    .context("--tool requires a tool ID")?
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("工具编号不是有效 Unicode"))?;
                crate::tool_shortcuts::validate_id(&id)?;
                anyhow::ensure!(tool.is_none(), "--tool cannot be repeated");
                tool = Some(id);
            }
            "--fixture-server" => {
                let value = args.next().context("--fixture-server requires a port")?;
                fixture_port = Some(
                    value
                        .to_str()
                        .context("invalid port")?
                        .parse()
                        .context("invalid fixture port")?,
                );
            }
            "--fixture-shutdown" => {
                let value = args.next().context("--fixture-shutdown requires a port")?;
                fixture_shutdown_port = Some(
                    value
                        .to_str()
                        .context("invalid port")?
                        .parse()
                        .context("invalid fixture port")?,
                );
            }
            "--config" => {
                config_path = args
                    .next()
                    .map(std::path::PathBuf::from)
                    .context("--config requires a path")?;
            }
            "--no-restore" => restore_services = false,
            "--import-config" => {
                import_config = Some(
                    args.next()
                        .map(std::path::PathBuf::from)
                        .context("--import-config requires a path")?,
                );
            }
            "--force-import" => force_import = true,
            _ => {}
        }
    }
    anyhow::ensure!(
        open_files.is_empty()
            || (tool.is_none()
                && fixture_port.is_none()
                && fixture_shutdown_port.is_none()
                && import_config.is_none()
                && !force_import),
        "--open-files cannot be combined with tool, fixture or config-import actions"
    );
    if let Some(target) = file_action {
        target
            .validate_selection(&open_files)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(Arguments {
        restore_services: restore_services && tool.is_none() && open_files.is_empty(),
        tool,
        open_files,
        file_action,
        config_path,
        fixture_port,
        fixture_shutdown_port,
        import_config,
        force_import,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_actions_select_exact_receivers_and_reject_invalid_or_conflicting_requests() {
        let parse = |args: &[&str]| parse_args_from(args.iter().map(std::ffi::OsString::from));
        for target in crate::intake::Target::ALL {
            let args = parse(&[
                "--file-action",
                target.action_id(),
                "--open-files",
                "missing.input",
            ])
            .unwrap();
            assert_eq!(args.file_action, Some(target));
            assert!(!args.restore_services);
        }
        for args in [
            vec!["--file-action"],
            vec!["--file-action", "unknown", "--open-files", "a"],
            vec!["--file-action", "import:csv"],
            vec!["--file-action", "import:csv", "--open-files", "a", "b"],
            vec![
                "--file-action",
                "import:files",
                "--file-action",
                "import:csv",
                "--open-files",
                "a",
            ],
            vec![
                "--tool",
                "json",
                "--file-action",
                "import:json",
                "--open-files",
                "a",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        assert_eq!(
            parse(&["--file-action", "import:files", "--open-files", "a", "b"])
                .unwrap()
                .open_files
                .len(),
            2
        );
    }
    #[cfg(windows)]
    #[test]
    fn generated_shell_templates_roundtrip_through_windows_argument_parser() {
        use windows::{Win32::UI::Shell::CommandLineToArgvW, core::PCWSTR};
        let dir = std::env::current_dir().unwrap().join("资料 folder");
        let exe = dir.join("ZiDevTools.exe");
        let config = dir.join("配置.yml");
        let file = dir.join("file & --tool $(literal).json");
        let plan = crate::explorer::plan(&exe, &config).unwrap();
        for entry in plan
            .values
            .iter()
            .filter(|entry| entry.key.ends_with("\\command"))
        {
            let command = entry.value.replace("%1", file.to_str().unwrap());
            let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
            let mut count = 0;
            unsafe {
                let argv = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut count);
                assert!(!argv.is_null());
                let values: Vec<_> = std::slice::from_raw_parts(argv, count as usize)
                    .iter()
                    .map(|p| std::ffi::OsString::from(p.to_string().unwrap()))
                    .collect();
                windows_sys::Win32::Foundation::LocalFree(argv.cast());
                assert_eq!(std::path::PathBuf::from(&values[0]), exe);
                let args = parse_args_from(values.into_iter().skip(1)).unwrap();
                assert_eq!(args.config_path, config);
                assert_eq!(args.open_files, vec![file.clone()]);
                assert!(!args.restore_services);
                if let Some(id) = entry
                    .key
                    .strip_suffix("\\command")
                    .unwrap()
                    .rsplit('\\')
                    .next()
                    .filter(|id| *id != "choose")
                {
                    assert_eq!(args.file_action.unwrap().action_id(), id);
                } else {
                    assert!(args.file_action.is_none());
                }
            }
        }
    }
    #[test]
    fn external_files_are_literal_bounded_and_do_not_restore_services() {
        let parse = |args: &[&str]| parse_args_from(args.iter().map(std::ffi::OsString::from));
        let parsed = parse(&[
            "--config",
            "local.yml",
            "--open-files",
            "资料 folder.csv",
            "--tool",
            "json",
        ])
        .unwrap();
        assert!(!parsed.restore_services);
        assert!(parsed.tool.is_none());
        assert_eq!(parsed.open_files.len(), 3);
        assert_eq!(
            parsed.open_files[0],
            std::path::absolute("资料 folder.csv").unwrap()
        );
        assert_eq!(parsed.open_files[1], std::path::absolute("--tool").unwrap());
        for args in [
            vec!["--open-files"],
            vec!["--open-files", ""],
            vec!["--open-files", "bad\0path"],
            vec!["--tool", "json", "--open-files", "a.json"],
            vec!["--fixture-server", "12345", "--open-files", "a.json"],
            vec!["--fixture-shutdown", "12345", "--open-files", "a.json"],
            vec!["--import-config", "config.yml", "--open-files", "a.json"],
            vec!["--force-import", "--open-files", "a.json"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        let mut many = vec!["--open-files"];
        many.extend(std::iter::repeat_n("not-required-to-exist.txt", 64));
        assert_eq!(parse(&many).unwrap().open_files.len(), 64);
        many.push("extra.txt");
        assert!(parse(&many).is_err());
    }
    #[test]
    fn tool_launch_is_explicit_and_does_not_restore_services() {
        let parse = |args: &[&str]| parse_args_from(args.iter().map(std::ffi::OsString::from));
        let args = parse(&[
            "--tool",
            "json",
            "--config",
            "C:\\资料 folder\\services.yml",
        ])
        .unwrap();
        assert_eq!(args.tool.as_deref(), Some("json"));
        assert!(!args.restore_services);
        assert!(parse(&["--tool"]).is_err());
        assert!(parse(&["--tool", "json", "--tool", "base64"]).is_err());
        assert!(parse(&[]).unwrap().restore_services);
    }
}
