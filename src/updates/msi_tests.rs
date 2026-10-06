use super::*;
use ring::signature::Ed25519KeyPair;
fn authenticated(
    version: &str,
    package: &[u8],
    desktop: &[u8],
    mcp: &[u8],
) -> (Vec<u8>, Vec<u8>, signed::Authenticated) {
    let key = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let manifest = signed::Manifest {
        schema: 1,
        version: version.into(),
        source_commit: "a".repeat(40),
        platform: "windows-x64".into(),
        minimum_windows_major: 10,
        files: [
            ("ZiDevTools", "msi", package),
            ("ZiDevTools", "exe", desktop),
            ("ZiDevToolsMcp", "exe", mcp),
        ]
        .into_iter()
        .map(|(name, ext, data)| signed::File {
            name: format!("{name}-{version}-windows-x64.{ext}"),
            size: data.len() as u64,
            sha256: io::digest(data),
        })
        .collect(),
        tools: vec![signed::Tool {
            id: "fixture".into(),
            name: "Fixture".into(),
            version: Some("0.1.0".into()),
            status: "implemented".into(),
        }],
    };
    let (raw, signature) = signed::sign(&manifest, &key).unwrap();
    let auth = signed::verify(
        &raw,
        &signature,
        &signed::TrustStore {
            schema: 1,
            keys: vec![signed::publisher(&key)],
        },
    )
    .unwrap();
    (raw, signature, auth)
}
fn temporary() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zi-msi-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    io::root(&dir).unwrap()
}
fn test_plan(directory: PathBuf) -> Plan {
    Plan {
        schema: 1,
        program_version: "0.1.0".into(),
        product_code: "{11111111-1111-4111-8111-111111111111}".into(),
        installed_version: "0.1.0".into(),
        directory,
        old_hashes: ["a".repeat(64), "b".repeat(64)],
        helper_hash: "a".repeat(64),
        manifest: String::new(),
        signature: String::new(),
    }
}
#[test]
fn unsigned_or_wrongly_signed_package_refused_before_preparation() {
    assert!(prepare(b"{}", b"{}", b"msi", &AtomicBool::new(false)).is_err());
    let (raw, signature, _) = authenticated("0.2.0", b"msi", b"desktop", b"mcp");
    assert!(prepare(&raw, &signature, b"msi", &AtomicBool::new(false)).is_err());
}
#[test]
fn stage_identity_parent_and_plan_fields_are_bounded() {
    let root = temporary();
    let stage = root.join(format!("msi-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage).unwrap();
    assert!(stage_at(&stage, &root).is_ok());
    assert!(stage_at(&root, &root).is_err());
    let other = temporary();
    assert!(stage_at(&stage, &other).is_err());
    let mut p = test_plan(root.clone());
    fs::write(stage.join("plan.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(plan(&stage).is_ok());
    p.old_hashes[0] = "INVALID".into();
    fs::write(stage.join("plan.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(plan(&stage).is_err());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir(other).unwrap();
}
#[test]
fn wrong_version_or_tampered_bytes_refuse_before_native_open() {
    let root = temporary();
    let p = test_plan(root.clone());
    fs::write(root.join("package.msi"), b"tampered").unwrap();
    let (_, _, auth) = authenticated("0.2.0", b"original", b"desktop", b"mcp");
    assert!(
        validate_package(&root, &p, &auth)
            .unwrap_err()
            .to_string()
            .contains("摘要")
    );
    let (_, _, old) = authenticated("0.1.0", b"tampered", b"desktop", b"mcp");
    assert!(
        validate_package(&root, &p, &old)
            .unwrap_err()
            .to_string()
            .contains("降级")
    );
    let (_, _, preview) = authenticated("0.2.0-dev.1", b"tampered", b"desktop", b"mcp");
    assert!(validate_package(&root, &p, &preview).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "only disposable GitHub-hosted runner; installs MSI fixture, never developer host"]
fn native_authenticated_upgrade_transaction() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").as_deref(),
        Ok("github-hosted")
    );
    let dir = PathBuf::from(
        std::env::var_os("ZI_MSI_FIXTURE_DIR").expect("Explicit fixture directory required"),
    );
    let runner = io::root(&PathBuf::from(std::env::var_os("RUNNER_TEMP").unwrap())).unwrap();
    let dir = io::root(&dir).unwrap();
    assert!(dir.starts_with(&runner));
    assert!(
        installer::registered_products().unwrap().is_empty(),
        "Refuse any preexisting real product"
    );
    let target = dir.join("Rust 安装 & updater with spaces");
    let staging = dir.join("rust-updates");
    fs::create_dir(&staging).unwrap();
    let codes: Vec<_> = ["old", "failure", "new"]
        .iter()
        .map(|name| {
            installer::inspect(&dir.join(format!("{name}.msi")))
                .unwrap()
                .product_code
        })
        .collect();
    struct Cleanup(Vec<String>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
                INSTALLSTATE_ABSENT, MsiConfigureProductExW,
            };
            for code in &self.0 {
                let code: Vec<_> = code.encode_utf16().chain([0]).collect();
                let properties: Vec<_> =
                    "REBOOT=ReallySuppress".encode_utf16().chain([0]).collect();
                let status = unsafe {
                    MsiConfigureProductExW(
                        code.as_ptr(),
                        0,
                        INSTALLSTATE_ABSENT,
                        properties.as_ptr(),
                    )
                };
                assert!(
                    matches!(status, 0 | 1605),
                    "Fixture uninstall failed: {status}"
                );
            }
        }
    }
    let cleanup = Cleanup(codes);
    assert_eq!(
        installer::install(&dir.join("old.msi"), &target, false).unwrap(),
        0
    );
    fs::write(target.join("user-data.txt"), b"must survive").unwrap();
    let installed = installer::owner(&target).unwrap().unwrap();
    let helper = fs::read(target.join(NAMES[0])).unwrap();
    for (label, state) in [("failure", "old-verified"), ("new", "installed")] {
        let package = fs::read(dir.join(format!("{label}.msi"))).unwrap();
        let desktop = fs::read(dir.join(label).join(NAMES[0])).unwrap();
        let mcp = fs::read(dir.join(label).join(NAMES[1])).unwrap();
        let (raw, signature, auth) = authenticated("0.2.0", &package, &desktop, &mcp);
        let prepared = prepare_checked(
            &staging,
            &installed,
            "0.1.0",
            &raw,
            &signature,
            &package,
            &helper,
            &AtomicBool::new(false),
            &auth,
        )
        .unwrap();
        let p = plan(&prepared.stage).unwrap();
        fs::write(prepared.stage.join("package.msi"), b"tampered").unwrap();
        assert!(apply_checked(&prepared.stage, &p, &auth, false).is_err());
        assert!(!prepared.stage.join("receipt-installing.json").exists());
        baseline(&p).unwrap();
        fs::write(prepared.stage.join("package.msi"), &package).unwrap();
        let receipt = apply_checked(&prepared.stage, &p, &auth, false).unwrap();
        assert_eq!(receipt.state, state);
        assert_eq!(
            receipt.status,
            Some(if label == "failure" { 1603 } else { 0 })
        );
        assert_eq!(
            fs::read(target.join("user-data.txt")).unwrap(),
            b"must survive"
        );
        assert!(
            apply_checked(&prepared.stage, &p, &auth, false).is_err(),
            "No duplicate install"
        );
        if label == "new" {
            let installed_package =
                installer::inspect(&prepared.stage.join("package.msi")).unwrap();
            fs::write(target.join(NAMES[1]), b"unknown changed MCP").unwrap();
            assert!(
                verified_target(&p, &installed_package, &auth).is_err(),
                "Reported installer success cannot mask changed target bytes"
            );
            fs::write(target.join(NAMES[1]), &mcp).unwrap();
            verified_target(&p, &installed_package, &auth).unwrap();
        }
        println!(
            "PASS: authenticated Rust MSI {label}: {} status {:?}",
            receipt.state, receipt.status
        );
    }
    drop(cleanup);
    assert!(installer::registered_products().unwrap().is_empty());
    assert!(!target.join(NAMES[0]).exists());
    assert!(!target.join(NAMES[1]).exists());
    assert_eq!(
        fs::read(target.join("user-data.txt")).unwrap(),
        b"must survive"
    );
    println!("PASS: Rust updater native transaction and fixture uninstall preserved user data");
}
