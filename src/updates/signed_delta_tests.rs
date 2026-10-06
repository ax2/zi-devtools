use super::*;
use ring::rand::SystemRandom;
use std::fs;
struct Fixture {
    key: Ed25519KeyPair,
    trust: signed::TrustStore,
    raw: Vec<u8>,
    target: signed::Authenticated,
    sources: [Vec<u8>; 2],
    targets: [Vec<u8>; 2],
    manifest: Manifest,
    patches: [Vec<u8>; 2],
}
fn fixture() -> Fixture {
    let pkcs = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(pkcs.as_ref()).unwrap();
    let trust = signed::TrustStore {
        schema: 1,
        keys: vec![signed::publisher(&key)],
    };
    let bytes = |seed: u64| {
        let mut state = seed;
        (0..98304)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                (state >> 32) as u8
            })
            .collect::<Vec<_>>()
    };
    let sources = [bytes(1), bytes(7)];
    let mut targets = sources.clone();
    targets[0][25000..25128].fill(42);
    targets[1][43000..43128].fill(99);
    let manifest = signed::Manifest {
        schema: 1,
        version: "0.83.0".into(),
        source_commit: "a".repeat(40),
        platform: "windows-x64".into(),
        minimum_windows_major: 10,
        files: [
            ("ZiDevTools-0.83.0-windows-x64.exe", targets[0].as_slice()),
            (
                "ZiDevToolsMcp-0.83.0-windows-x64.exe",
                targets[1].as_slice(),
            ),
            (
                "ZiDevTools-0.83.0-windows-x64.msi",
                b"fixture msi".as_slice(),
            ),
        ]
        .into_iter()
        .map(|(name, b)| signed::File {
            name: name.into(),
            size: b.len() as u64,
            sha256: portable::digest(b),
        })
        .collect(),
        tools: vec![signed::Tool {
            id: "fixture".into(),
            name: "Synthetic".into(),
            version: Some("1.0.0".into()),
            status: "implemented".into(),
        }],
    };
    let (raw, sig) = signed::sign(&manifest, &key).unwrap();
    let target = signed::verify(&raw, &sig, &trust).unwrap();
    let (manifest, patches) = create(
        "0.82.0",
        &target,
        &raw,
        [&sources[0], &sources[1]],
        [&targets[0], &targets[1]],
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    Fixture {
        key,
        trust,
        raw,
        target,
        sources,
        targets,
        manifest,
        patches,
    }
}
#[test]
fn authenticated_pair_reconstructs_exact_bytes_with_real_block_reuse() {
    let f = fixture();
    let (raw, sig) = sign(&f.manifest, &f.target, &f.raw, &f.key).unwrap();
    let m = verify(&raw, &sig, &f.target, &f.raw, &f.trust).unwrap();
    assert!(baseline(&m, "0.82.0", [&f.sources[0], &f.sources[1]]));
    assert!(!baseline(&m, "0.82.1", [&f.sources[0], &f.sources[1]]));
    for i in 0..2 {
        assert_eq!(
            delta::inspect(&f.patches[i], &AtomicBool::new(false))
                .unwrap()
                .0
                .kind,
            delta::Kind::Delta
        );
        assert_eq!(
            reconstruct(&m, i, &f.sources[i], &f.patches[i], &AtomicBool::new(false)).unwrap(),
            f.targets[i]
        );
    }
}
#[test]
fn wrong_key_domain_release_unknown_fields_and_bad_patch_are_refused() {
    let f = fixture();
    let (raw, sig) = sign(&f.manifest, &f.target, &f.raw, &f.key).unwrap();
    assert!(verify_official(&raw, &sig, &f.target, &f.raw).is_err());
    assert!(signed::verify(&raw, &sig, &f.trust).is_err());
    assert!(
        verify(
            &raw,
            &sig,
            &f.target,
            b"different target manifest",
            &f.trust
        )
        .is_err()
    );
    let mut unknown: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    unknown["execute"] = "arbitrary".into();
    let unknown = serde_json::to_vec(&unknown).unwrap();
    let signature = signed::sign_detached(&unknown, DOMAIN, &f.key).unwrap();
    assert!(verify(&unknown, &signature, &f.target, &f.raw, &f.trust).is_err());
    let mut patch = f.patches[0].clone();
    *patch.last_mut().unwrap() ^= 1;
    assert!(
        reconstruct(
            &f.manifest,
            0,
            &f.sources[0],
            &patch,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        reconstruct(
            &f.manifest,
            0,
            &f.sources[1],
            &f.patches[0],
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        reconstruct(
            &f.manifest,
            0,
            &f.sources[0],
            &f.patches[0],
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert!(!asset_name(
        "../0.82.0-to-0.83.0-windows-x64.zidelta",
        "0.83.0"
    ));
    assert!(asset_name(&f.manifest.files[0].patch.name, "0.83.0"));
}
#[test]
fn signed_pair_enters_production_portable_staging_without_touching_source() {
    let f = fixture();
    let root = std::env::temp_dir().join(format!("zi-signed-delta-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let root = portable::root(&root).unwrap();
    fs::write(root.join("ZiDevTools.exe"), &f.sources[0]).unwrap();
    fs::write(root.join("ZiDevToolsMcp.exe"), &f.sources[1]).unwrap();
    fs::write(root.join("user-data.txt"), b"untouched").unwrap();
    let (raw, sig) = sign(&f.manifest, &f.target, &f.raw, &f.key).unwrap();
    let m = verify(&raw, &sig, &f.target, &f.raw, &f.trust).unwrap();
    let outputs: Vec<_> = (0..2)
        .map(|i| reconstruct(&m, i, &f.sources[i], &f.patches[i], &AtomicBool::new(false)).unwrap())
        .collect();
    let (_, target_sig) = signed::sign(&f.target.manifest, &f.key).unwrap();
    let prepared = portable::prepare_authenticated(
        &root,
        "0.82.0",
        &f.raw,
        &target_sig,
        [&outputs[0], &outputs[1]],
        &f.sources[0],
        &AtomicBool::new(false),
        &f.target,
    )
    .unwrap();
    assert_eq!(
        fs::read(prepared.directory.join("new-ZiDevTools.exe")).unwrap(),
        f.targets[0]
    );
    assert_eq!(
        fs::read(prepared.directory.join("new-ZiDevToolsMcp.exe")).unwrap(),
        f.targets[1]
    );
    assert_eq!(fs::read(root.join("ZiDevTools.exe")).unwrap(), f.sources[0]);
    assert_eq!(fs::read(root.join("user-data.txt")).unwrap(), b"untouched");
    portable::test_apply_or_restore(&prepared.directory, &f.target, false).unwrap();
    assert_eq!(fs::read(root.join("ZiDevTools.exe")).unwrap(), f.targets[0]);
    assert_eq!(
        fs::read(root.join("ZiDevToolsMcp.exe")).unwrap(),
        f.targets[1]
    );
    portable::test_apply_or_restore(&prepared.directory, &f.target, true).unwrap();
    assert_eq!(fs::read(root.join("ZiDevTools.exe")).unwrap(), f.sources[0]);
    assert_eq!(
        fs::read(root.join("ZiDevToolsMcp.exe")).unwrap(),
        f.sources[1]
    );
    assert_eq!(fs::read(root.join("user-data.txt")).unwrap(), b"untouched");
}
#[test]
#[ignore = "read previous release and generated EXEs only in disposable CI; never launch binaries"]
fn actual_release_pair() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").as_deref(),
        Ok("github-hosted")
    );
    let base = portable::root(&std::path::PathBuf::from(
        std::env::var_os("ZI_DELTA_BASELINE_DIR").unwrap(),
    ))
    .unwrap();
    let runner = portable::root(&std::path::PathBuf::from(
        std::env::var_os("RUNNER_TEMP").unwrap(),
    ))
    .unwrap();
    assert!(base.starts_with(&runner));
    let source_version = std::env::var("ZI_DELTA_BASELINE_VERSION").unwrap();
    let target_version = env!("CARGO_PKG_VERSION");
    let cancel = AtomicBool::new(false);
    let mut sources = [Vec::new(), Vec::new()];
    let mut targets = [Vec::new(), Vec::new()];
    let mut manifest = fixture().target.manifest;
    manifest.version = target_version.into();
    for (i, prefix) in ["ZiDevTools", "ZiDevToolsMcp"].iter().enumerate() {
        sources[i] = super::super::delta_files::read(
            &base.join(format!("{prefix}-{source_version}-windows-x64.exe")),
            false,
            &cancel,
        )
        .unwrap();
        targets[i] = super::super::delta_files::read(
            &std::path::PathBuf::from(format!("target/release/{prefix}.exe")),
            false,
            &cancel,
        )
        .unwrap();
    }
    manifest.files = [
        ("ZiDevTools", "exe", targets[0].as_slice()),
        ("ZiDevToolsMcp", "exe", targets[1].as_slice()),
        (
            "ZiDevTools",
            "msi",
            b"synthetic MSI descriptor only; not an actual installer".as_slice(),
        ),
    ]
    .into_iter()
    .map(|(prefix, ext, bytes)| signed::File {
        name: format!("{prefix}-{target_version}-windows-x64.{ext}"),
        size: bytes.len() as u64,
        sha256: portable::digest(bytes),
    })
    .collect();
    let key = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let trust = signed::TrustStore {
        schema: 1,
        keys: vec![signed::publisher(&key)],
    };
    let (target_raw, target_sig) = signed::sign(&manifest, &key).unwrap();
    let authenticated = signed::verify(&target_raw, &target_sig, &trust).unwrap();
    let (m, patches) = create(
        &source_version,
        &authenticated,
        &target_raw,
        [&sources[0], &sources[1]],
        [&targets[0], &targets[1]],
        &cancel,
    )
    .unwrap()
    .expect("actual EXEs must have beneficial compressed update pair");
    let (raw, sig) = sign(&m, &authenticated, &target_raw, &key).unwrap();
    let m = verify(&raw, &sig, &authenticated, &target_raw, &trust).unwrap();
    let root = base.join("disposable-current");
    fs::create_dir(&root).unwrap();
    let root = portable::root(&root).unwrap();
    fs::write(root.join("user-data.txt"), b"retained").unwrap();
    let mut outputs = [Vec::new(), Vec::new()];
    for (i, name) in ["ZiDevTools.exe", "ZiDevToolsMcp.exe"].iter().enumerate() {
        fs::write(root.join(name), &sources[i]).unwrap();
        outputs[i] = reconstruct(&m, i, &sources[i], &patches[i], &cancel).unwrap();
        assert_eq!(outputs[i], targets[i]);
        let (h, _) = delta::inspect(&patches[i], &cancel).unwrap();
        println!(
            "PASS actual EXE {name}: {:?}, patch={} full={} copied={}",
            h.kind,
            patches[i].len(),
            targets[i].len(),
            h.copied_bytes
        );
    }
    let p = portable::prepare_authenticated(
        &root,
        &source_version,
        &target_raw,
        &target_sig,
        [&outputs[0], &outputs[1]],
        &sources[0],
        &cancel,
        &authenticated,
    )
    .unwrap();
    portable::test_apply_or_restore(&p.directory, &authenticated, false).unwrap();
    for (i, name) in ["ZiDevTools.exe", "ZiDevToolsMcp.exe"].iter().enumerate() {
        assert_eq!(fs::read(root.join(name)).unwrap(), targets[i]);
    }
    portable::test_apply_or_restore(&p.directory, &authenticated, true).unwrap();
    for (i, name) in ["ZiDevTools.exe", "ZiDevToolsMcp.exe"].iter().enumerate() {
        assert_eq!(fs::read(root.join(name)).unwrap(), sources[i]);
    }
    assert_eq!(fs::read(root.join("user-data.txt")).unwrap(), b"retained");
    println!(
        "PASS actual EXE signed reconstruction, portable apply and explicit restore; no program or MSI executed"
    );
}
