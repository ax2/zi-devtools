//! Windows tool launchers, with explicit identity and no implicit service restore.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.' | b'/')),
        "无效的工具编号"
    );
    Ok(())
}

pub fn app_id(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("ZiCode.DevTools.Tool.{:x}", Sha256::digest(id.as_bytes()))
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            '\\',
            slashes * if c == '"' { 2 } else { 1 },
        ));
        slashes = 0;
        if c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

pub fn arguments(id: &str, config: &Path) -> Result<String> {
    validate_id(id)?;
    let config = std::path::absolute(config).context("无法确定配置的绝对位置")?;
    let config = config.to_str().context("配置路径不是有效 Unicode")?;
    ensure!(!config.contains('\0'), "配置路径包含无效字符");
    Ok(format!(
        "--tool {} --config {} --no-restore",
        quote(id),
        quote(config)
    ))
}

#[cfg(windows)]
pub fn set_process_identity(id: &str) -> Result<()> {
    validate_id(id)?;
    unsafe {
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            &windows::core::HSTRING::from(app_id(id)),
        )?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn create(
    directory: &Path,
    exe: &Path,
    config: &Path,
    id: &str,
    title: &str,
) -> Result<PathBuf> {
    use windows::{
        Win32::{
            Foundation::PROPERTYKEY,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile, StructuredStorage::PROPVARIANT,
            },
            UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
        },
        core::{HSTRING, Interface},
    };
    let args = arguments(id, config)?;
    ensure!(
        exe.is_absolute() && exe.is_file(),
        "程序位置不存在，请重新创建快捷方式"
    );
    std::fs::create_dir_all(directory)?;
    let label: String = title
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(60)
        .collect();
    let label = label.trim_end_matches([' ', '.']);
    let label = if label.is_empty() { "工具" } else { label };
    // Reserve the destination without replacing an existing user shortcut.
    let mut destination = None;
    for n in 0..1000 {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!(" ({n})")
        };
        let path = directory.join(format!("Zi DevTools - {label}{suffix}.lnk"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {
                destination = Some(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let destination = destination.context("同名快捷方式过多，请清理后重试")?;
    let path = destination.clone();
    let exe = exe.to_owned();
    let title = title.to_owned();
    let identity = app_id(id);
    // A fresh STA avoids inheriting a different COM apartment from the UI.
    let result = std::thread::spawn(move || -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            struct Apartment;
            impl Drop for Apartment {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _apartment = Apartment;
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let exe_text = HSTRING::from(exe.as_os_str());
            link.SetPath(&exe_text)?;
            link.SetArguments(&HSTRING::from(args))?;
            link.SetDescription(&HSTRING::from(format!("直接打开 {title}")))?;
            link.SetIconLocation(&exe_text, 0)?;
            link.SetWorkingDirectory(&HSTRING::from(
                exe.parent().context("程序没有父目录")?.as_os_str(),
            ))?;
            let store: IPropertyStore = link.cast()?;
            let key = PROPERTYKEY {
                fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
                pid: 5,
            };
            store.SetValue(&key, &PROPVARIANT::from(identity.as_str()))?;
            store.Commit()?;
            let file: IPersistFile = link.cast()?;
            file.Save(&HSTRING::from(path.as_os_str()), true)?;
        }
        Ok(())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("创建快捷方式失败"))?;
    if let Err(error) = result {
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_preserve_paths_and_reject_invalid_ids() {
        assert_eq!(
            arguments("json", Path::new("C:\\资料 folder\\config.yml")).unwrap(),
            "--tool \"json\" --config \"C:\\资料 folder\\config.yml\" --no-restore"
        );
        assert!(arguments("json\" --bad", Path::new("x")).is_err());
        let relative = Path::new("local fixtures/services.yml");
        assert!(arguments("json", relative).unwrap().contains(&quote(
            std::path::absolute(relative).unwrap().to_str().unwrap()
        )));
        assert_ne!(app_id("json"), app_id("base64"));
        assert_eq!(quote("x\\"), "\"x\\\\\"");
    }

    #[cfg(windows)]
    #[test]
    fn native_links_preserve_arguments_identity_and_existing_files() -> Result<()> {
        use windows::{
            Win32::{
                Foundation::PROPERTYKEY,
                System::Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoUninitialize, IPersistFile, STGM_READ,
                },
                UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
            },
            core::{HSTRING, Interface},
        };
        let directory = std::env::temp_dir().join(format!("zi-shortcuts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let exe = std::env::current_exe()?;
        let config = directory.join("中文 配置.yml");
        let path = create(
            &directory,
            &exe,
            &config,
            "plugin:local-text/uppercase",
            "文本工具",
        )?;
        let original = std::fs::read(&path)?;
        let second = create(&directory, &exe, &config, "json", "文本工具")?;
        assert_ne!(path, second);
        assert_eq!(std::fs::read(&path)?, original);
        std::thread::spawn(move || -> Result<()> {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                }
                let _apartment = Apartment;
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                let file: IPersistFile = link.cast()?;
                file.Load(&HSTRING::from(path.as_os_str()), STGM_READ)?;
                let mut buffer = [0u16; 4096];
                link.GetArguments(&mut buffer)?;
                let decoded = |buffer: &[u16]| {
                    String::from_utf16_lossy(
                        &buffer[..buffer.iter().position(|c| *c == 0).unwrap()],
                    )
                };
                assert_eq!(
                    decoded(&buffer),
                    arguments("plugin:local-text/uppercase", &config)?
                );
                buffer.fill(0);
                link.GetPath(&mut buffer, std::ptr::null_mut(), 4)?;
                assert_eq!(PathBuf::from(decoded(&buffer)), exe);
                let store: IPropertyStore = link.cast()?;
                let key = PROPERTYKEY {
                    fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
                    pid: 5,
                };
                assert_eq!(
                    store.GetValue(&key)?.to_string(),
                    app_id("plugin:local-text/uppercase")
                );
            }
            Ok(())
        })
        .join()
        .expect("COM fixture panicked")?;
        Ok(())
    }
}
