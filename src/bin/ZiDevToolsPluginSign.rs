use anyhow::{Context, Result, ensure};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroize;

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("无法新建文件：{}（不会覆盖现有文件）", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_private_key(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("无法检查私钥文件：{}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 4096,
        "私钥必须是小于等于 4 KiB 的普通文件"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "私钥文件超过 4 KiB");
    Ok(bytes)
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            ensure!(
                args.len() == 3,
                "用法：ZiDevToolsPluginSign keygen <发布者名称> <新私钥.pk8>"
            );
            let (mut private_key, publisher) =
                zi_devtools::plugins::generate_publisher_keypair(&args[1])?;
            let result = write_new(Path::new(&args[2]), &private_key);
            private_key.zeroize();
            result?;
            println!("私钥已写入：{}", args[2]);
            println!("发布者：{}", publisher.name);
            println!("SHA-256 指纹：{}", publisher.id);
            println!("公钥 Base64：{}", publisher.public_key);
            println!("请将私钥文件离线备份并严格保密；只向用户发布公钥及指纹。");
        }
        Some("sign") => {
            ensure!(
                args.len() == 4,
                "用法：ZiDevToolsPluginSign sign <私钥.pk8> <插件清单.json> <新签名清单.json>"
            );
            let manifest = zi_devtools::plugins::read_manifest(Path::new(&args[2]))?;
            let mut private_key = read_private_key(Path::new(&args[1]))?;
            let result = zi_devtools::plugins::sign_manifest_with_pkcs8(&manifest, &private_key);
            private_key.zeroize();
            let envelope = result?;
            write_new(Path::new(&args[3]), &envelope)?;
            println!("签名清单已写入：{}", args[3]);
            println!("输出不会覆盖已有文件；核对 key_id 与已发布的公钥指纹。");
        }
        Some("sign-catalog") => {
            ensure!(
                args.len() == 4,
                "用法：ZiDevToolsPluginSign sign-catalog <私钥.pk8> <插件目录.json> <新签名目录.json>"
            );
            let catalog = zi_devtools::plugins::read_manifest(Path::new(&args[2]))?;
            let mut private_key = read_private_key(Path::new(&args[1]))?;
            let result = zi_devtools::plugins::sign_catalog_with_pkcs8(&catalog, &private_key);
            private_key.zeroize();
            let envelope = result?;
            write_new(Path::new(&args[3]), &envelope)?;
            println!("签名插件目录已写入：{}", args[3]);
            println!("输出不会覆盖已有文件；核对 key_id 与已发布的公钥指纹。");
        }
        _ => anyhow::bail!(
            "用法：ZiDevToolsPluginSign keygen <发布者名称> <新私钥.pk8> | sign <私钥.pk8> <插件清单.json> <新签名清单.json> | sign-catalog <私钥.pk8> <插件目录.json> <新签名目录.json>"
        ),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
