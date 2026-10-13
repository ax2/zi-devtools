//! Native Shell discovery of the actually installed MSI menu.
//! Optional invocation is limited to a disposable hosted runner and a synthetic file.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::windows::fs::OpenOptionsExt, path::PathBuf};
    use windows::{
        Win32::{
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            UI::Shell::{
                BHID_SFUIObject, CMINVOKECOMMANDINFO, CommandLineToArgvW, IContextMenu,
                IContextMenu3, IShellItem, SHCreateItemFromParsingName,
            },
        },
        core::{Interface, PCSTR, PCWSTR},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use zi_shell_core::{
        explorer::lifecycle::{Identity, store},
        file_actions::Target,
    };
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("CI").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "Native installed-menu probe requires a disposable hosted runner"
    );
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--invoke-generic"),
        "Expected fixture file, installed executable, and optional --invoke-generic"
    );
    let invoke_generic = args.len() == 3;
    let file = std::fs::canonicalize(PathBuf::from(&args[0]))?;
    let executable = std::fs::canonicalize(PathBuf::from(&args[1]))?;
    let temporary =
        std::fs::canonicalize(std::env::var_os("RUNNER_TEMP").context("Runner temp missing")?)?;
    ensure!(
        file.starts_with(&temporary) && executable.starts_with(&temporary),
        "Outside disposable scope"
    );
    ensure!(file.is_file(), "Expected an existing fixture file");
    let before = store::load()?;
    ensure!(
        before.pending_msi_operation().is_none(),
        "Installation has not finished"
    );
    let active = before
        .active()
        .context("Active menu installation missing")?;
    ensure!(
        active.identity == Identity::Msi && active.executable == executable,
        "Wrong active installation"
    );
    let mut file_lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .open(&executable)?;
    let length = file_lock.metadata()?.len();
    ensure!(
        (2..=256 * 1024 * 1024).contains(&length),
        "Executable outside fixture bounds"
    );
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut read = 0u64;
    loop {
        let count = file_lock.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read += count as u64;
        ensure!(read <= length, "Executable size changed");
        digest.update(&buffer[..count]);
    }
    ensure!(
        read == length
            && active.executable_sha256.as_deref()
                == Some(format!("{:x}", digest.finalize()).as_str()),
        "Executable differs from installed ledger"
    );
    let declaration = zi_shell_core::capability_image::read(&mut file_lock, length)?
        .context("Installed desktop capability declaration missing")?;
    ensure!(
        declaration.targets() == Target::ALL,
        "Installed desktop declaration differs from receivers"
    );
    let mut mcp_lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .open(
            executable
                .parent()
                .context("Executable parent missing")?
                .join("ZiDevToolsMcp.exe"),
        )?;
    let mcp_length = mcp_lock.metadata()?.len();
    ensure!(
        zi_shell_core::capability_image::read(&mut mcp_lock, mcp_length)?.is_none(),
        "MCP must not declare desktop UI receivers"
    );
    let owner =
        zi_shell_core::msi::owner(executable.parent().context("Executable parent missing")?)?
            .context("Native MSI ownership missing")?;
    ensure!(
        std::fs::canonicalize(owner.executable)? == executable && owner.version == active.version,
        "Native owner mismatch"
    );

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    struct Menu(HMENU);
    impl Drop for Menu {
        fn drop(&mut self) {
            unsafe {
                DestroyMenu(self.0);
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let _com = Com;
    // Keep canonical paths for identity/scope checks. Shell parsing uses the
    // ordinary Win32 namespace, as in the existing native invocation probe.
    let canonical = file.to_str().context("Fixture path is not Unicode")?;
    let shell_path = if let Some(unc) = canonical.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        canonical
            .strip_prefix(r"\\?\")
            .unwrap_or(canonical)
            .to_owned()
    };
    ensure!(
        std::fs::canonicalize(&shell_path)? == file,
        "Shell path differs from verified fixture"
    );
    let wide: Vec<_> = shell_path.encode_utf16().chain(Some(0)).collect();
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
        .context("SHCreateItemFromParsingName for installed-menu fixture")?;
    let context: IContextMenu = unsafe { item.BindToHandler(None, &BHID_SFUIObject) }
        .context("IShellItem::BindToHandler for installed-menu fixture")?;
    let menu = Menu(unsafe { CreatePopupMenu() });
    ensure!(!menu.0.is_null(), "Cannot create native menu");
    unsafe {
        context
            .QueryContextMenu(
                windows::Win32::UI::WindowsAndMessaging::HMENU(menu.0),
                0,
                1,
                32767,
                4096,
            )
            .ok()
            .context("IContextMenu::QueryContextMenu for installed menu")?;
    }
    fn labels(menu: HMENU) -> anyhow::Result<Vec<(String, u32)>> {
        let count = unsafe { GetMenuItemCount(menu) };
        ensure!((0..=256).contains(&count), "Native menu count is invalid");
        let mut values = Vec::new();
        for index in 0..count {
            let mut text = [0u16; 512];
            let length = unsafe {
                GetMenuStringW(
                    menu,
                    index as u32,
                    text.as_mut_ptr(),
                    text.len() as i32,
                    MF_BYPOSITION,
                )
            };
            ensure!(
                (0..511).contains(&length),
                "Native menu text invalid or truncated"
            );
            values.push((String::from_utf16(&text[..length as usize])?, unsafe {
                GetMenuItemID(menu, index)
            }));
        }
        Ok(values)
    }
    let top = labels(menu.0)?;
    let positions: Vec<_> = top
        .iter()
        .enumerate()
        .filter(|(_, (label, _))| label == "Zi DevTools")
        .map(|(position, _)| position)
        .collect();
    ensure!(
        positions.len() == 1,
        "Expected exactly one installed product cascade"
    );
    let position = positions[0];
    let submenu = unsafe { GetSubMenu(menu.0, position as i32) };
    ensure!(!submenu.is_null(), "Installed cascade cannot expand");
    let messages: IContextMenu3 = context.cast().context("Installed menu IContextMenu3")?;
    unsafe {
        messages
            .HandleMenuMsg2(
                WM_INITMENUPOPUP,
                windows::Win32::Foundation::WPARAM(submenu as usize),
                windows::Win32::Foundation::LPARAM(position as isize),
                None,
            )
            .context("WM_INITMENUPOPUP for installed cascade")?;
    }
    let actions = labels(submenu)?;
    let declared_plan =
        zi_shell_core::explorer::plan_declared(&executable, &active.config, &declaration)?;
    let expected: Vec<_> = declared_plan
        .values
        .iter()
        .filter(|value| {
            value.name == "MUIVerb"
                && value
                    .key
                    .starts_with(r"Software\Classes\ZiCode.DevTools.FileActions\shell\")
        })
        .map(|value| value.value.clone())
        .collect();
    ensure!(
        actions.len() == expected.len(),
        "Installed cascade count differs from shared receivers"
    );
    let mut command_ids = std::collections::BTreeSet::new();
    for label in &expected {
        let matches: Vec<_> = actions
            .iter()
            .filter(|(actual, _)| actual == label)
            .collect();
        ensure!(
            matches.len() == 1,
            "Missing or duplicate installed receiver"
        );
        let command = matches[0].1;
        ensure!(
            (1..32767).contains(&command) && command_ids.insert(command),
            "Invalid or duplicate native command ID"
        );
    }
    let mut invocation = serde_json::json!({
        "performed": false,
        "receivedArgumentsMatch": false,
        "explorerWindowVerified": false,
    });
    if invoke_generic {
        ensure!(
            file.file_name().and_then(|name| name.to_str()) == Some("真实文件 %1 & 空格.json"),
            "Invocation is limited to the named synthetic fixture"
        );
        ensure!(
            std::fs::read(&file)? == b"synthetic installed Shell invocation fixture",
            "Refusing to invoke a non-synthetic file"
        );
        let receipt = file.with_file_name(".zidevtools-shell-receipt.json");
        ensure!(!receipt.exists(), "Synthetic receipt already exists");
        let matches: Vec<_> = actions
            .iter()
            .filter(|(label, _)| label == "选择工具处理…")
            .collect();
        ensure!(matches.len() == 1, "Generic receiver missing or duplicated");
        let command = matches[0].1;
        ensure!((1..32767).contains(&command), "Invalid generic command ID");
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            fMask: 0x100 | 0x400, // NOASYNC | FLAG_NO_UI; the receiver is a headless fixture.
            lpVerb: PCSTR((command - 1) as usize as *const u8),
            nShow: 0,
            ..Default::default()
        };
        unsafe {
            context
                .InvokeCommand(&info)
                .context("IContextMenu::InvokeCommand for installed generic receiver")?;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !receipt.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let received: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&receipt)
                .context("Installed Shell receiver did not write its synthetic receipt")?,
        )?;
        let generic_command = declared_plan
            .values
            .iter()
            .find(|value| value.key.ends_with(r"\shell\choose\command"))
            .context("Generic command is absent from the installed declaration plan")?;
        let command_line = generic_command.value.replace("%1", &shell_path);
        let command_wide: Vec<_> = command_line.encode_utf16().chain(Some(0)).collect();
        let mut argument_count = 0;
        let command_arguments =
            unsafe { CommandLineToArgvW(PCWSTR(command_wide.as_ptr()), &mut argument_count) };
        ensure!(
            !command_arguments.is_null(),
            "Cannot parse planned Shell command"
        );
        let wanted: Vec<String> = unsafe {
            std::slice::from_raw_parts(command_arguments, argument_count as usize)
                .iter()
                .skip(1)
                .map(|value| {
                    value
                        .to_string()
                        .context("Planned Shell argument is invalid Unicode")
                })
                .collect::<anyhow::Result<_>>()?
        };
        unsafe { windows_sys::Win32::Foundation::LocalFree(command_arguments.cast()) };
        ensure!(
            received["fixtureOnly"] == true
                && received["contentRead"] == false
                && received["executed"] == false
                && received["arguments"] == serde_json::json!(wanted)
                && std::fs::canonicalize(std::path::PathBuf::from(
                    received["received"]
                        .as_str()
                        .context("Receiver omitted source path")?
                ))? == file,
            "Installed Shell invocation arguments differ from the registered command"
        );
        invocation = serde_json::json!({
            "performed": true,
            "receivedArgumentsMatch": true,
            "receiver": "generic file chooser",
            "syntheticSourceUnchanged": std::fs::read(&file)? == b"synthetic installed Shell invocation fixture",
            "explorerWindowVerified": false,
        });
        std::fs::remove_file(&receipt)?;
    }
    ensure!(
        store::load()? == before,
        "Menu ledger changed during discovery or invocation"
    );
    println!(
        "{}",
        serde_json::json!({
            "installedMsiMenuResolved": true,
            "receiverCount": declaration.targets().len(),
            "desktopDeclarationMatches": true,
            "declaredPlanMatchesShell": true,
            "mcpDeclaresDesktopReceivers": false,
            "nativeCommandCount": actions.len(),
            "labels": expected,
            "associationWrites": false,
            "commandInvocation": invocation,
        })
    );
    // Keep the native context and executable consistency lock until discovery,
    // optional invocation and the final ledger recheck have completed.
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
