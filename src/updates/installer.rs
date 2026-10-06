//! Read-only Windows Installer identity and component ownership.
//! No registry guessing, install, repair or uninstall operations in production.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::*;

pub const UPGRADE_CODE: &str = "{61AF00B6-E92A-4519-A0A3-382347FA4051}";
const MAX_TEXT: usize = 32768;
struct Handle(u32);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            MsiCloseHandle(self.0);
        }
    }
}
fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut value: Vec<_> = value.encode_wide().collect();
    ensure!(
        value.len() < MAX_TEXT && !value.contains(&0),
        "安装器字符串无效"
    );
    value.push(0);
    Ok(value)
}
fn text(value: &str) -> Result<Vec<u16>> {
    wide(std::ffi::OsStr::new(value))
}
/// Called only after authenticated preparation and exact process exit.
/// Windows Installer owns replacement and rollback; no manual EXE copying.
pub(super) fn install(package: &Path, directory: &Path, visible: bool) -> Result<u32> {
    let package = text(&installer_path(package)?)?;
    let directory = installer_path(directory)?;
    ensure!(
        !directory.chars().any(|c| c.is_control() || c == '"'),
        "安装路径包含保留字符"
    );
    let properties = text(&format!(
        "INSTALLFOLDER=\"{directory}\" REBOOT=ReallySuppress MSIRESTARTMANAGERCONTROL=Disable"
    ))?;
    let old_ui = unsafe {
        MsiSetInternalUI(
            if visible {
                INSTALLUILEVEL_BASIC
            } else {
                INSTALLUILEVEL_NONE
            },
            std::ptr::null_mut(),
        )
    };
    struct Ui(i32);
    impl Drop for Ui {
        fn drop(&mut self) {
            unsafe {
                MsiSetInternalUI(self.0, std::ptr::null_mut());
            }
        }
    }
    let _ui = Ui(old_ui);
    Ok(unsafe { MsiInstallProductW(package.as_ptr(), properties.as_ptr()) })
}
fn installer_path(path: &Path) -> Result<String> {
    let value = path.to_str().context("安装路径编码无效")?;
    Ok(if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(value).to_owned()
    })
}
fn check(code: u32, operation: &str) -> Result<()> {
    ensure!(code == 0, "Windows Installer {operation}失败（{code}）");
    Ok(())
}
fn guid(value: &str) -> Result<String> {
    let id = uuid::Uuid::parse_str(value).context("安装器GUID无效")?;
    ensure!(!id.is_nil(), "安装器GUID不能为空");
    Ok(format!(
        "{{{}}}",
        id.hyphenated().to_string().to_uppercase()
    ))
}
pub fn numeric_version(value: &str) -> Result<semver::Version> {
    let v = semver::Version::parse(value)?;
    ensure!(
        v.pre.is_empty()
            && v.build.is_empty()
            && v.major <= 255
            && v.minor <= 255
            && v.patch <= 65535,
        "MSI要求三段数值版本，范围255.255.65535"
    );
    Ok(v)
}
fn decoded(buffer: &[u16], length: u32) -> Result<String> {
    ensure!((length as usize) < buffer.len(), "安装器返回字符串超过限制");
    String::from_utf16(&buffer[..length as usize]).context("安装器字符串编码无效")
}
fn query(db: &Handle, sql: &str) -> Result<String> {
    let sql = text(sql)?;
    let mut view = 0;
    check(
        unsafe { MsiDatabaseOpenViewW(db.0, sql.as_ptr(), &mut view) },
        "打开只读查询",
    )?;
    let view = Handle(view);
    check(unsafe { MsiViewExecute(view.0, 0) }, "执行只读查询")?;
    let mut record = 0;
    let status = unsafe { MsiViewFetch(view.0, &mut record) };
    if status == 259 {
        return Ok(String::new());
    }
    check(status, "读取查询结果")?;
    let record = Handle(record);
    let mut buffer = vec![0; MAX_TEXT];
    let mut length = (buffer.len() - 1) as u32;
    check(
        unsafe { MsiRecordGetStringW(record.0, 1, buffer.as_mut_ptr(), &mut length) },
        "读取字段",
    )?;
    let mut second = 0;
    let status = unsafe { MsiViewFetch(view.0, &mut second) };
    if second != 0 {
        drop(Handle(second));
    }
    ensure!(status == 259, "安装器身份字段不唯一（{status}）");
    decoded(&buffer, length)
}
#[derive(Clone, Debug, Serialize)]
pub struct Package {
    pub product_code: String,
    pub version: String,
    pub desktop_component: String,
    pub architecture: String,
    pub transactional_upgrade: bool,
    pub records_install_location: bool,
}
/// Reading an MSI does not execute custom actions or install its files.
pub fn inspect(path: &Path) -> Result<Package> {
    let metadata = std::fs::symlink_metadata(path)?;
    use std::os::windows::fs::MetadataExt;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.file_attributes() & 0x400 == 0,
        "MSI必须为普通文件"
    );
    let path = text(&installer_path(path)?)?;
    let mut db = 0;
    check(
        unsafe { MsiOpenDatabaseW(path.as_ptr(), MSIDBOPEN_READONLY, &mut db) },
        "打开MSI",
    )?;
    let db = Handle(db);
    let property = |name: &str| {
        query(
            &db,
            &format!("SELECT `Value` FROM `Property` WHERE `Property`='{name}'"),
        )
    };
    ensure!(
        property("ProductName")? == "Zi DevTools" && property("Manufacturer")? == "ZiCode",
        "不是Zi DevTools安装器"
    );
    ensure!(
        guid(&property("UpgradeCode")?)? == UPGRADE_CODE,
        "安装器产品系列不匹配"
    );
    ensure!(property("ALLUSERS")?.is_empty(), "当前只支持每用户安装器");
    let version = property("ProductVersion")?;
    numeric_version(&version)?;
    let product_code = guid(&property("ProductCode")?)?;
    let desktop_component = guid(&query(
        &db,
        "SELECT `ComponentId` FROM `Component` WHERE `Component`='Application'",
    )?)?;
    let mut summary = 0;
    check(
        unsafe { MsiGetSummaryInformationW(db.0, std::ptr::null(), 0, &mut summary) },
        "读取包摘要",
    )?;
    let summary = Handle(summary);
    let mut kind = 0;
    let mut number = 0;
    let mut time = windows_sys::Win32::Foundation::FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut buffer = vec![0; MAX_TEXT];
    let mut length = (buffer.len() - 1) as u32;
    check(
        unsafe {
            MsiSummaryInfoGetPropertyW(
                summary.0,
                7,
                &mut kind,
                &mut number,
                &mut time,
                buffer.as_mut_ptr(),
                &mut length,
            )
        },
        "读取架构",
    )?;
    let architecture = decoded(&buffer, length)?;
    ensure!(
        kind == 30 && architecture.split(';').next() == Some("x64"),
        "安装器不是Windows x64包"
    );
    let sequence = |action: &str| -> Result<i32> {
        query(
            &db,
            &format!("SELECT `Sequence` FROM `InstallExecuteSequence` WHERE `Action`='{action}'"),
        )?
        .parse()
        .context("安装器升级顺序缺失")
    };
    let transactional_upgrade = sequence("RemoveExistingProducts")?
        > sequence("InstallInitialize")?
        && sequence("RemoveExistingProducts")? < sequence("InstallFinalize")?;
    // Legacy packages can omit the optional CustomAction table altogether.
    let records_install_location = !query(
        &db,
        "SELECT `Name` FROM `_Tables` WHERE `Name`='CustomAction'",
    )?
    .is_empty()
        && query(
            &db,
            "SELECT `Target` FROM `CustomAction` WHERE `Source`='ARPINSTALLLOCATION'",
        )? == "[INSTALLFOLDER]";
    Ok(Package {
        product_code,
        version,
        desktop_component,
        architecture,
        transactional_upgrade,
        records_install_location,
    })
}
fn product_info(code: &str, name: &str) -> Result<String> {
    let code = text(code)?;
    let name = text(name)?;
    let mut buffer = vec![0; MAX_TEXT];
    let mut length = (buffer.len() - 1) as u32;
    check(
        unsafe {
            MsiGetProductInfoW(
                code.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr(),
                &mut length,
            )
        },
        "读取已注册产品",
    )?;
    decoded(&buffer, length)
}
#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub product_code: String,
    pub version: String,
    pub executable: PathBuf,
    pub install_location: Option<PathBuf>,
}
pub fn registered_products() -> Result<Vec<Installed>> {
    let upgrade = text(UPGRADE_CODE)?;
    let mut products = Vec::new();
    for index in 0..64 {
        let mut code = [0u16; 39];
        let status =
            unsafe { MsiEnumRelatedProductsW(upgrade.as_ptr(), 0, index, code.as_mut_ptr()) };
        if status == 259 {
            return Ok(products);
        }
        check(status, "枚举已安装产品")?;
        let code = guid(&decoded(&code, 38)?)?;
        let cached = product_info(&code, "LocalPackage")?;
        ensure!(!cached.is_empty(), "已安装产品的缓存MSI缺失");
        let package = inspect(Path::new(&cached))?;
        ensure!(
            package.product_code == code
                && package.version == product_info(&code, "VersionString")?,
            "注册产品与缓存身份不一致"
        );
        let product = text(&code)?;
        let component = text(&package.desktop_component)?;
        let mut buffer = vec![0; MAX_TEXT];
        let mut length = (buffer.len() - 1) as u32;
        let state = unsafe {
            MsiGetComponentPathW(
                product.as_ptr(),
                component.as_ptr(),
                buffer.as_mut_ptr(),
                &mut length,
            )
        };
        ensure!(
            state == INSTALLSTATE_LOCAL,
            "无法确认安装器主程序组件（{state}）"
        );
        let executable = PathBuf::from(decoded(&buffer, length)?);
        ensure!(
            executable.is_absolute()
                && executable
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("ZiDevTools.exe")),
            "已安装程序路径无效"
        );
        let location = product_info(&code, "InstallLocation")?;
        let install_location = if location.is_empty() {
            None
        } else {
            let location = PathBuf::from(location);
            ensure!(
                location.is_absolute() && same_directory(&executable, &location)?,
                "安装器位置与组件归属不一致"
            );
            Some(location)
        };
        products.push(Installed {
            product_code: code,
            version: package.version,
            executable,
            install_location,
        });
    }
    anyhow::bail!("安装器产品数量超过检查上限")
}
fn same_directory(executable: &Path, directory: &Path) -> Result<bool> {
    let parent = executable.parent().context("安装器组件目录缺失")?;
    // canonicalize catches missing/broken ownership rather than treating it as portable.
    Ok(parent.canonicalize()? == directory.canonicalize()?)
}
pub fn owner(directory: &Path) -> Result<Option<Installed>> {
    let mut matching = None;
    for installed in registered_products()? {
        if same_directory(&installed.executable, directory)? {
            ensure!(matching.is_none(), "多个安装器产品声明相同程序目录");
            matching = Some(installed);
        }
    }
    Ok(matching)
}
pub(super) fn ensure_portable_directory(directory: &Path, products: &[Installed]) -> Result<()> {
    for installed in products {
        ensure!(
            !same_directory(&installed.executable, directory)?,
            "该目录由MSI {}管理，请通过Windows Installer升级，不能直接替换EXE",
            installed.version
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_versions_are_bounded_and_not_prereleases() {
        for value in ["0.81.0", "255.255.65535"] {
            assert!(numeric_version(value).is_ok());
        }
        for value in [
            "0.82.0-dev.20",
            "0.81.0+meta",
            "256.0.0",
            "1.256.0",
            "1.1.65536",
            "01.1.0",
            "1.0",
            "1.0.0\0",
        ] {
            assert!(numeric_version(value).is_err(), "{value}");
        }
    }
    #[test]
    fn identity_and_text_reject_invalid_values() {
        assert_eq!(
            guid("61af00b6-e92a-4519-a0a3-382347fa4051").unwrap(),
            UPGRADE_CODE
        );
        assert!(guid("{00000000-0000-0000-0000-000000000000}").is_err());
        assert!(guid("bad").is_err());
        assert!(text("path\0injection").is_err());
        assert!(text(&"a".repeat(MAX_TEXT)).is_err());
        assert!(decoded(&[0; 2], 2).is_err());
    }
    #[test]
    fn malformed_package_is_never_executed() {
        let path =
            std::env::temp_dir().join(format!("zi-msi-inspect-{}.msi", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not an MSI").unwrap();
        let result = inspect(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_err());
    }
    #[test]
    fn installed_and_portable_can_coexist_but_unknown_ownership_refuses() {
        let root = std::env::temp_dir().join(format!("zi-msi-owner-{}", uuid::Uuid::new_v4()));
        let installed = root.join("安装目录 with spaces");
        let portable = root.join("便携目录");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir(&portable).unwrap();
        let products = [Installed {
            product_code: UPGRADE_CODE.into(),
            version: "0.81.0".into(),
            executable: installed.join("ZiDevTools.exe"),
            install_location: None,
        }];
        assert!(ensure_portable_directory(&installed, &products).is_err());
        assert!(ensure_portable_directory(&portable, &products).is_ok());
        std::fs::remove_dir(&installed).unwrap();
        assert!(ensure_portable_directory(&portable, &products).is_err());
        std::fs::remove_dir(&portable).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
}
