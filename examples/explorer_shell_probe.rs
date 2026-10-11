//! Real Windows Shell COM resolution and invocation, with disposable associations.
//! This does not automate or certify the Explorer window or the product installer.
#[cfg(windows)]
#[path = "explorer_shell_probe/ui.rs"]
mod application_ui;

#[cfg(windows)]
fn fixture_text(target: Option<zi_devtools::intake::Target>) -> &'static str {
    use zi_devtools::intake::Target;
    match target {
        Some(Target::JsonData) => "[{\"name\":\"fixture\",\"count\":3}]",
        Some(Target::Json) => "{\"fixture\":true}",
        Some(Target::Csv) => "name,count\nfixture,3\n",
        Some(Target::Tsv) => "name\tcount\nfixture\t3\n",
        _ => "synthetic tutorial fixture; no diagnostic automatically executed",
    }
}
#[cfg(windows)]
mod native {
    use anyhow::{Context, Result, ensure};
    use std::{
        fs,
        path::Path,
        ptr,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            UI::Shell::{
                BHID_SFUIObject, CMINVOKECOMMANDINFO, IContextMenu, IContextMenu3, IShellItem,
                SHCreateItemFromParsingName,
            },
        },
        core::{Interface, PCSTR, PCWSTR},
    };
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS, System::Registry::*, UI::WindowsAndMessaging::*,
    };
    use zi_devtools::commands::file_actions::Target;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    fn check(code: u32) -> Result<()> {
        ensure!(code == ERROR_SUCCESS, "registry operation failed: {code}");
        Ok(())
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn classes() -> Result<Key> {
        let mut key = ptr::null_mut();
        check(unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(r"Software\Classes").as_ptr(),
                0,
                KEY_READ | KEY_WRITE,
                &mut key,
            )
        })?;
        Ok(Key(key))
    }
    fn create(base: HKEY, path: &str) -> Result<Key> {
        let mut key = ptr::null_mut();
        check(unsafe {
            RegCreateKeyExW(
                base,
                wide(path).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        })?;
        Ok(Key(key))
    }
    fn set(key: HKEY, name: &str, value: &str) -> Result<()> {
        let data = wide(value);
        check(unsafe {
            RegSetValueExW(
                key,
                wide(name).as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        })
    }
    fn exists(base: HKEY, path: &str) -> Result<bool> {
        let mut key = ptr::null_mut();
        let code = unsafe {
            RegOpenKeyExW(
                base,
                wide(path).as_ptr(),
                REG_OPTION_OPEN_LINK,
                KEY_READ,
                &mut key,
            )
        };
        if code == 2 {
            return Ok(false);
        }
        check(code)?;
        drop(Key(key));
        Ok(true)
    }
    struct Associations {
        roots: Vec<String>,
    }
    impl Associations {
        fn cleanup(&mut self) -> Result<()> {
            let base = classes()?;
            for root in &self.roots {
                ensure!(
                    root.starts_with(".zidt-probe-")
                        || root.starts_with("ZiCode.DevTools.ShellProbe-"),
                    "invalid fixture cleanup scope"
                );
                if exists(base.0, root)? {
                    check(unsafe { RegDeleteTreeW(base.0, wide(root).as_ptr()) })?;
                }
                ensure!(
                    !exists(base.0, root)?,
                    "fixture association survived cleanup"
                );
            }
            self.roots.clear();
            Ok(())
        }
    }
    impl Drop for Associations {
        fn drop(&mut self) {
            if let Err(e) = self.cleanup() {
                eprintln!("fixture cleanup failed: {e:#}");
            }
        }
    }
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
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
    fn menu_text(menu: HMENU, position: i32) -> String {
        let mut text = [0u16; 512];
        let size = unsafe {
            GetMenuStringW(
                menu,
                position as u32,
                text.as_mut_ptr(),
                text.len() as i32,
                MF_BYPOSITION,
            )
        };
        String::from_utf16_lossy(&text[..size.max(0) as usize])
    }
    fn children(menu: HMENU) -> Vec<(String, u32)> {
        let count = unsafe { GetMenuItemCount(menu) };
        (0..count)
            .map(|i| (menu_text(menu, i), unsafe { GetMenuItemID(menu, i) }))
            .collect()
    }
    fn probe(file: &Path, receipt: &Path, command_config: &Path) -> Result<Vec<String>> {
        let file_wide = wide(file.to_str().context("fixture path must be Unicode")?);
        let item: IShellItem =
            unsafe { SHCreateItemFromParsingName(PCWSTR(file_wide.as_ptr()), None) }
                .context("SHCreateItemFromParsingName")?;
        let context: IContextMenu = unsafe { item.BindToHandler(None, &BHID_SFUIObject) }
            .context("IShellItem::BindToHandler")?;
        let menu = Menu(unsafe { CreatePopupMenu() });
        ensure!(!menu.0.is_null(), "CreatePopupMenu failed");
        const FIRST: u32 = 1;
        unsafe {
            context
                .QueryContextMenu(
                    windows::Win32::UI::WindowsAndMessaging::HMENU(menu.0),
                    0,
                    FIRST,
                    32767,
                    4096, // CMF_SYNCCASCADEMENU: populate cascades synchronously.
                )
                .ok()
                .context("IContextMenu::QueryContextMenu")?;
        }
        let top = children(menu.0);
        let position = top
            .iter()
            .position(|(text, _)| text == "Zi DevTools")
            .context("Shell did not resolve Zi DevTools cascade")?;
        let submenu = unsafe { GetSubMenu(menu.0, position as i32) };
        ensure!(!submenu.is_null(), "cascade has no submenu");
        let messages: IContextMenu3 = context.cast().context("IContextMenu3")?;
        unsafe {
            messages
                .HandleMenuMsg2(
                    WM_INITMENUPOPUP,
                    windows::Win32::Foundation::WPARAM(submenu as usize),
                    windows::Win32::Foundation::LPARAM(position as isize),
                    None,
                )
                .context("initialize cascade")?;
        }
        let actions = children(submenu);
        let expected: Vec<_> = std::iter::once(("选择工具处理…".to_owned(), None))
            .chain(Target::ALL.into_iter().map(|target| {
                (
                    format!("导入到{}…", target.label()),
                    Some(target.action_id()),
                )
            }))
            .collect();
        ensure!(
            actions.len() == expected.len(),
            "Shell cascade count: {} expected {}",
            actions.len(),
            expected.len()
        );
        let mut invoked = Vec::new();
        for (label, action) in expected {
            fs::write(
                file,
                super::fixture_text(action.and_then(Target::from_action_id)),
            )?;
            let id = actions
                .iter()
                .find(|(text, _)| text == &label)
                .context("missing Shell action")?
                .1;
            ensure!((FIRST..32767).contains(&id), "invalid command ID");
            if receipt.exists() {
                fs::remove_file(receipt)?;
            }
            let info = CMINVOKECOMMANDINFO {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                fMask: 0x100 | 0x400, // NOASYNC | FLAG_NO_UI: deterministic fixture, no prompts.
                lpVerb: PCSTR((id - FIRST) as usize as *const u8),
                nShow: 0,
                ..Default::default()
            };
            unsafe {
                context
                    .InvokeCommand(&info)
                    .with_context(|| format!("IContextMenu::InvokeCommand: {label}"))?;
            }
            let deadline = Instant::now() + Duration::from_secs(60);
            while !receipt.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            let actual: Vec<String> = serde_json::from_slice(
                &fs::read(receipt).context("Shell receiver did not report arguments")?,
            )?;
            let mut wanted = vec![
                "--config".to_owned(),
                command_config.display().to_string(),
                "--no-restore".into(),
            ];
            if let Some(action) = action {
                wanted.extend(["--file-action".into(), action.into()]);
            }
            wanted.extend(["--open-files".into(), file.display().to_string()]);
            ensure!(
                actual == wanted,
                "Shell argument expansion differed for {label}"
            );
            ensure!(
                fs::read(file)?
                    == super::fixture_text(action.and_then(Target::from_action_id)).as_bytes(),
                "application changed the selected source file"
            );
            invoked.push(label);
        }
        Ok(invoked)
    }
    pub fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|a| a == "--config") {
            let parsed =
                zi_devtools::launch::parse_args_from(args.iter().map(std::ffi::OsString::from))?;
            ensure!(
                !parsed.restore_services && parsed.open_files.len() == 1 && parsed.tool.is_none(),
                "Shell launch did not pass the product parser"
            );
            // Receiver is this test executable, never the user's desktop process.
            let config = &parsed.config_path;
            ensure!(
                config
                    .file_name()
                    .is_some_and(|n| n == "shell-services.yml"),
                "not a fixture config"
            );
            let path = config.with_file_name("shell-receipt.json");
            let control = config.with_file_name("shell-ui-control.json");
            if control.exists() {
                let light: bool = serde_json::from_slice(&fs::read(control)?)?;
                super::application_ui::run(parsed, light)?;
            }
            let pending = path.with_extension("pending");
            fs::write(&pending, serde_json::to_vec(&args)?)?;
            fs::rename(pending, &path)?;
            return Ok(());
        }
        ensure!(
            !args.is_empty()
                && args.iter().skip(1).all(|a| matches!(
                    a.as_str(),
                    "--extended-paths" | "--application-ui" | "--light"
                )),
            "usage: explorer_shell_probe ARCHIVE_DIRECTORY [--extended-paths] [--application-ui] [--light]"
        );
        let extended = args.iter().any(|a| a == "--extended-paths");
        let application_ui = args.iter().any(|a| a == "--application-ui");
        let light = args.iter().any(|a| a == "--light");
        ensure!(
            !light || application_ui,
            "--light requires --application-ui"
        );
        let absolute = std::path::absolute(&args[0])?;
        for ancestor in absolute.ancestors() {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                fs::symlink_metadata(ancestor)?.file_attributes() & 0x400 == 0,
                "archive path contains a reparse point"
            );
        }
        let canonical = fs::canonicalize(&args[0])?;
        // The Shell parsing namespace rejects the filesystem-only extended prefix.
        let canonical = canonical.to_str().context("archive must be Unicode")?;
        let folder =
            std::path::PathBuf::from(if let Some(unc) = canonical.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{unc}")
            } else {
                canonical
                    .strip_prefix(r"\\?\")
                    .unwrap_or(canonical)
                    .to_owned()
            });
        ensure!(
            !folder.symlink_metadata()?.file_type().is_symlink(),
            "linked archive"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let extension = format!(".zidt-probe-{id}");
        let progid = format!("ZiCode.DevTools.ShellProbe-{id}");
        let actions = format!("{progid}.Actions");
        let receipt = folder.join("shell-receipt.json");
        let config = folder.join("shell-services.yml");
        let control = folder.join("shell-ui-control.json");
        ensure!(
            !config.exists() && !control.exists(),
            "fixture config already exists"
        );
        fs::write(
            &config,
            format!(
                "state_dir: '{}'\nservices: {{}}\n",
                folder.join("shell-state").display()
            ),
        )?;
        if application_ui {
            fs::write(&control, serde_json::to_vec(&light)?)?;
        }
        ensure!(
            !receipt.exists() && !receipt.with_extension("pending").exists(),
            "fixture receipt already exists"
        );
        let file = folder.join(format!("教程 空格 & %1 字面--file-action{extension}"));
        fs::write(&file, b"synthetic Shell fixture; never user content")?;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        let _com = Com;
        let base = classes()?;
        let mut associations = Associations { roots: Vec::new() };
        for root in [&extension, &progid, &actions] {
            ensure!(!exists(base.0, root)?, "fixture association already exists");
            drop(create(base.0, root)?);
            associations.roots.push(root.clone());
        }
        set(create(base.0, &extension)?.0, "", &progid)?;
        let parent = format!(r"{progid}\shell\ZiCode.DevTools");
        let executable = if extended {
            fs::canonicalize(std::env::current_exe()?)?
        } else {
            std::env::current_exe()?
        };
        let command_config = if extended {
            fs::canonicalize(&folder)?.join("shell-services.yml")
        } else {
            config.clone()
        };
        let plan = zi_devtools::explorer::plan(&executable, &command_config)?;
        for value in plan.values {
            let path = if let Some(tail) = value
                .key
                .strip_prefix(r"Software\Classes\*\shell\ZiCode.DevTools")
            {
                format!("{parent}{tail}")
            } else {
                format!(
                    "{actions}{}",
                    value
                        .key
                        .strip_prefix(r"Software\Classes\ZiCode.DevTools.FileActions")
                        .context("unexpected plan root")?
                )
            };
            set(
                create(base.0, &path)?.0,
                &value.name,
                if value.name == "ExtendedSubCommandsKey" {
                    &actions
                } else {
                    &value.value
                },
            )?;
        }
        // Independently expect ordinary Win32 config syntax, even when the plan
        // input carried the extended filesystem prefix used by installation records.
        let result = probe(&file, &receipt, &config);
        associations.cleanup()?;
        fs::remove_file(&file)?;
        if receipt.exists() {
            fs::remove_file(&receipt)?;
        }
        fs::remove_file(&config)?;
        if control.exists() {
            fs::remove_file(&control)?;
        }
        let invoked = result?;
        let proof = serde_json::json!({"scope":"actual Windows Shell COM; isolated association; not Explorer-window acceptance", "productParserAccepted":true,"applicationUiConfirmed":application_ui,"light":light,"extendedCommandPaths":extended,"invoked":invoked,"count":invoked.len(),"registryCleanupVerified":true,"syntheticFileRemoved":true,"productMenuRegistered":false});
        fs::write(
            folder.join(if extended {
                "shell-proof-extended.json"
            } else {
                "shell-proof.json"
            }),
            serde_json::to_vec_pretty(&proof)?,
        )?;
        println!(
            "PASS Shell COM cascade and {} actual command invocations; isolated associations removed",
            invoked.len()
        );
        Ok(())
    }
}
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    native::run()
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows Shell integration requires Windows");
}
