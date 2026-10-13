//! Classic-menu plans, inspections and user-confirmed current-user maintenance.
pub mod audit;
pub mod lifecycle;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::quote;
const ROOTS: [&str; 6] = [
    r"Software\Classes\*\shell\ZiCode.DevTools",
    r"Software\Classes\ZiCode.DevTools.FileActions",
    r"Software\Classes\Directory\shell\ZiCode.DevTools",
    r"Software\Classes\Directory\Background\shell\ZiCode.DevTools",
    r"Software\Classes\ZiCode.DevTools.DirectoryActions",
    r"Software\Classes\ZiCode.DevTools.BackgroundActions",
];
const OWNER: &str = "com.zicode.devtools";
const MENU_PROTOCOL: &str = "1";

#[derive(Clone, Debug, Serialize)]
pub struct Value {
    pub key: String,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub schema_version: u32,
    pub preview_only: bool,
    pub scope: &'static str,
    pub shell_max_files: usize,
    pub executable: PathBuf,
    pub config: PathBuf,
    pub values: Vec<Value>,
    pub limitations: [&'static str; 3],
}

fn template_path(path: &Path) -> Result<String> {
    let path = std::path::absolute(path)?;
    let value = path.to_str().context("菜单模板路径不是有效Unicode")?;
    // Canonical filesystem paths may carry \\?\, which ShellExecute rejects.
    // Preserve canonical identity in Plan; only the Shell command uses Win32 syntax.
    #[cfg(windows)]
    let shell_path = if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = value.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        ensure!(
            bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'\\',
            "此扩展设备路径不能可靠地用于Shell命令，未执行安装"
        );
        drive.to_owned()
    } else {
        value.to_owned()
    };
    #[cfg(windows)]
    let value = shell_path.as_str();
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '%')),
        "菜单模板路径含引号、控制字符或百分号，不能可靠地用于Shell占位符模板；未执行任何安装操作"
    );
    Ok(quote(value))
}

pub fn plan(exe: &Path, config: &Path) -> Result<Plan> {
    plan_declared(
        exe,
        config,
        &crate::capability_manifest::Manifest::current(),
    )
}

/// Plans only the declared receivers, without reading files or writing registry.
/// The caller must bind this declaration to the target executable before use.
pub fn plan_declared(
    exe: &Path,
    config: &Path,
    declaration: &crate::capability_manifest::Manifest,
) -> Result<Plan> {
    let executable = std::path::absolute(exe)?;
    let configuration = std::path::absolute(config)?;
    let exe = template_path(&executable)?;
    let config = template_path(&configuration)?;
    let parent = ROOTS[0];
    let actions = ROOTS[1];
    let mut values = Vec::new();
    let mut add = |key: &str, name: &str, value: String| {
        values.push(Value {
            key: key.into(),
            name: name.into(),
            value,
        });
    };
    add(parent, "MUIVerb", "Zi DevTools".into());
    add(parent, "Icon", exe.clone());
    add(
        parent,
        "ExtendedSubCommandsKey",
        "ZiCode.DevTools.FileActions".into(),
    );
    // Ownership markers are a proposed layout, not evidence of an installed owner.
    for key in [parent, actions] {
        add(key, "ZiCodeOwner", OWNER.into());
        add(key, "ZiCodeMenuProtocol", MENU_PROTOCOL.into());
    }
    let generic = format!(r"{actions}\shell\choose");
    add(&generic, "MUIVerb", "选择工具处理…".into());
    add(&generic, "MultiSelectModel", "Single".into());
    add(
        &format!(r"{generic}\command"),
        "",
        format!("{exe} --config {config} --no-restore --open-files \"%1\""),
    );
    for target in declaration.targets() {
        let key = format!(r"{actions}\shell\{}", target.action_id());
        add(&key, "MUIVerb", format!("导入到{}…", target.label()));
        add(&key, "MultiSelectModel", "Single".into());
        add(
            &format!(r"{key}\command"),
            "",
            format!(
                "{exe} --config {config} --no-restore --file-action {} --open-files \"%1\"",
                target.action_id()
            ),
        );
    }
    if declaration.supports_folder_contexts() {
        for (root, action_root) in [
            (
                ROOTS[2],
                (ROOTS[4], "ZiCode.DevTools.DirectoryActions", "%1"),
            ),
            (
                ROOTS[3],
                (ROOTS[5], "ZiCode.DevTools.BackgroundActions", "%V"),
            ),
        ] {
            add(root, "MUIVerb", "Zi DevTools".into());
            add(root, "Icon", exe.clone());
            add(root, "ZiCodeOwner", OWNER.into());
            add(root, "ZiCodeMenuProtocol", MENU_PROTOCOL.into());
            add(root, "ExtendedSubCommandsKey", action_root.1.into());
            add(action_root.0, "ZiCodeOwner", OWNER.into());
            add(action_root.0, "ZiCodeMenuProtocol", MENU_PROTOCOL.into());
            let action = format!(r"{}\shell\disk-inspector", action_root.0);
            add(&action, "MUIVerb", "分析目录空间…".into());
            add(&action, "Icon", exe.clone());
            add(
                &format!(r"{action}\command"),
                "",
                format!(
                    "{exe} --config {config} --no-restore --tool disk-inspector --open-folder \"{}\"",
                    action_root.2
                ),
            );
        }
    }
    ensure!(
        values
            .iter()
            .filter(|value| value.key.ends_with("\\command"))
            .all(|value| value.value.encode_utf16().count() < 32767),
        "完整菜单命令超过Windows命令行上限，未执行安装"
    );
    Ok(Plan {
        schema_version: 1,
        preview_only: true,
        scope: "HKEY_CURRENT_USER",
        shell_max_files: 1,
        executable,
        config: configuration,
        values,
        limitations: [
            "此JSON仅是单选经典菜单候选布局，生成计划不读取或写入注册表；维护流程需另行预览确认",
            "文件夹入口使用独立有界只读扫描；文件多选和Windows11现代菜单仍需验收",
            "MSI自动维护、Windows Installer回滚及真实Explorer显示/调用仍需独立验收",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_actions::Target;
    #[test]
    fn declared_subset_never_adds_actions_from_the_maintaining_program() {
        use crate::capability_manifest::Manifest;
        let dir = std::env::current_dir().unwrap().join("capability-subset");
        let exe = dir.join("ZiDevTools.exe");
        let config = dir.join("config.yml");
        for (body, expected) in [
            (
                "import:csv\nimport:json\n",
                vec!["import:csv", "import:json"],
            ),
            ("", vec![]),
        ] {
            let declaration = Manifest::parse(
                format!("ZiDevTools/ShellCapabilities/1\nchoose-files\n{body}").as_bytes(),
            )
            .unwrap();
            let planned = plan_declared(&exe, &config, &declaration).unwrap();
            let commands: Vec<_> = planned
                .values
                .iter()
                .filter(|v| v.key.ends_with("\\command"))
                .collect();
            assert_eq!(commands.len(), expected.len() + 1);
            for target in Target::ALL {
                assert_eq!(
                    commands.iter().any(|v| v.key.contains(target.action_id())),
                    expected.contains(&target.action_id())
                );
            }
            assert!(
                commands
                    .iter()
                    .all(|v| v.value.ends_with("--open-files \"%1\""))
            );
            assert!(planned.preview_only);
        }
        assert_eq!(
            serde_json::to_value(plan(&exe, &config).unwrap()).unwrap(),
            serde_json::to_value(plan_declared(&exe, &config, &Manifest::current()).unwrap())
                .unwrap()
        );
    }
    #[cfg(windows)]
    #[test]
    fn canonical_file_identity_uses_shell_compatible_command_paths() {
        let exe = Path::new(r"\\?\C:\资料 folder\ZiDevTools.exe");
        let config = Path::new(r"\\?\UNC\server\share\服务.yml");
        let plan = plan(exe, config).unwrap();
        assert_eq!(plan.executable, exe);
        assert_eq!(plan.config, config);
        let command = &plan
            .values
            .iter()
            .find(|v| v.key.ends_with(r"\choose\command"))
            .unwrap()
            .value;
        assert!(
            command.starts_with(
                r#""C:\资料 folder\ZiDevTools.exe" --config "\\server\share\服务.yml""#
            )
        );
        assert!(!command.contains(r"\\?\"));
        assert!(
            template_path(Path::new(
                r"\\?\Volume{00000000-0000-0000-0000-000000000000}\ZiDevTools.exe"
            ))
            .is_err()
        );
    }
    #[test]
    fn plans_only_current_user_single_file_receivers_and_literal_process_commands() {
        let dir = std::env::current_dir().unwrap().join("资料 folder");
        let p = plan(&dir.join("ZiDevTools.exe"), &dir.join("服务.yml")).unwrap();
        assert!(p.preview_only);
        assert_eq!(p.scope, "HKEY_CURRENT_USER");
        assert_eq!(p.shell_max_files, 1);
        let commands: Vec<_> = p
            .values
            .iter()
            .filter(|v| v.key.ends_with("\\command"))
            .collect();
        assert_eq!(commands.len(), Target::ALL.len() + 3);
        assert!(
            commands
                .iter()
                .filter(|c| c.key.contains("FileActions"))
                .all(|c| c.value.ends_with("--open-files \"%1\""))
        );
        let mut keys = std::collections::BTreeSet::new();
        for value in &p.values {
            assert!(value.key.starts_with("Software\\Classes\\"));
            assert!(keys.insert((&value.key, &value.name)));
            if value.name == "MultiSelectModel" {
                assert_eq!(value.value, "Single");
            }
        }
        for target in Target::ALL {
            assert_eq!(
                commands
                    .iter()
                    .filter(|v| v
                        .value
                        .contains(&format!("--file-action {} --", target.action_id())))
                    .count(),
                1
            );
        }
        for bad in ["bad%1.exe", "bad\"path.exe", "bad\0path", "bad\npath"] {
            assert!(plan(&dir.join(bad), &dir.join("config.yml")).is_err());
            assert!(plan(&dir.join("ZiDevTools.exe"), &dir.join(bad)).is_err());
        }
        let long = dir.join("a".repeat(18000));
        assert!(
            plan(&long, &long).is_err(),
            "two individually bounded paths must not form an oversized Windows command line"
        );
    }

    #[test]
    fn folder_contexts_have_distinct_bounded_directory_receivers() {
        use crate::capability_manifest::Manifest;
        let dir = std::env::current_dir().unwrap().join("资料 folder");
        let exe = dir.join("ZiDevTools.exe");
        let config = dir.join("服务.yml");
        let current = plan(&exe, &config).unwrap();
        let folder_commands: Vec<_> = current
            .values
            .iter()
            .filter(|value| {
                value.key.ends_with("\\command") && value.value.contains("--open-folder")
            })
            .collect();
        assert_eq!(folder_commands.len(), 2);
        assert!(folder_commands.iter().any(|command| {
            command.key.starts_with(ROOTS[4]) && command.value.ends_with("--open-folder \"%1\"")
        }));
        assert!(folder_commands.iter().any(|command| {
            command.key.starts_with(ROOTS[5]) && command.value.ends_with("--open-folder \"%V\"")
        }));
        for command in &folder_commands {
            assert!(command.value.contains("--tool disk-inspector"));
            assert!(!command.value.contains("--open-files"));
        }

        let legacy = plan_declared(
            &exe,
            &config,
            &Manifest::parse(b"ZiDevTools/ShellCapabilities/1\nchoose-files\n").unwrap(),
        )
        .unwrap();
        assert!(
            !legacy
                .values
                .iter()
                .any(|value| value.key.starts_with(ROOTS[2])
                    || value.key.starts_with(ROOTS[3])
                    || value.key.starts_with(ROOTS[4])
                    || value.key.starts_with(ROOTS[5]))
        );
    }
}
