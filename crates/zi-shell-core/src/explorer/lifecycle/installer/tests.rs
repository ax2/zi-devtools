use super::*;

#[test]
fn manual_msi_removal_and_relocation_refused_before_registry_access() {
    for request in [
        Request::Remove(Identity::Msi),
        Request::Relocate(Identity::Msi),
    ] {
        assert!(
            preview(Path::new("missing-config.yml"), request, "9.8.7")
                .err()
                .unwrap()
                .to_string()
                .contains("必须由Windows Installer维护")
        );
    }
    assert!(validate_manual_request(&Request::Activate(Identity::Msi)).is_ok());
    assert!(validate_manual_request(&Request::Install).is_ok());
    let portable = Identity::portable();
    assert!(validate_manual_request(&Request::Remove(portable.clone())).is_ok());
    assert!(validate_manual_request(&Request::Relocate(portable)).is_ok());
}

#[test]
fn unregistered_file_origin_is_allowed_but_missing_file_is_not() {
    let files = Files::new();
    assert!(ensure_portable_origin(&files.0.join("sample.exe")).is_ok());
    assert!(ensure_portable_origin(&files.0.join("missing.exe")).is_err());
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("zi-explorer-file-proof-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("sample.exe"),
            crate::capability_image::tests::fixture(),
        )
        .unwrap();
        std::fs::write(root.join("config.yml"), b"services: []\n").unwrap();
        Self(root)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("zi-explorer-file-proof-")
        );
        std::fs::remove_file(self.0.join("sample.exe")).unwrap();
        std::fs::remove_file(self.0.join("config.yml")).unwrap();
        std::fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn fresh_file_hash_and_write_delete_lock_protect_commit_identity() {
    let files = Files::new();
    let verified =
        VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml")).unwrap();
    let record = verified.record(Identity::portable(), "9.8.7").unwrap();
    assert_eq!(record.version, "9.8.7");
    assert!(record.menu_capabilities.is_some());
    let mut mismatched = record.clone();
    mismatched.menu_capabilities =
        Some("ZiDevTools/ShellCapabilities/1\nchoose-files\nimport:csv\n".into());
    assert!(VerifiedFile::verify(&mismatched).is_err());
    assert!(std::fs::write(&record.executable, b"MZchanged").is_err());
    assert!(std::fs::remove_file(&record.executable).is_err());
    let check = VerifiedFile::verify(&record).unwrap();
    drop(check);
    drop(verified);
    std::fs::write(&record.executable, b"MZchanged").unwrap();
    assert!(VerifiedFile::verify(&record).is_err());
    let undeclared =
        VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml")).unwrap();
    assert!(undeclared.record(Identity::portable(), "9.8.7").is_err());
    drop(undeclared);
    std::fs::write(files.0.join("sample.exe"), b"not an executable").unwrap();
    assert!(VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml")).is_err());
}

#[test]
fn missing_config_is_allowed_only_at_the_explicit_default_without_creating_it() {
    let files = Files::new();
    let default = canonical_location(&crate::paths::default_config_path(&files.0), true).unwrap();
    assert!(!default.exists());
    assert!(!files.0.join(".zi-devtools").exists());
    assert_eq!(
        canonical_config_for(&default, Some(&default)).unwrap(),
        default
    );
    assert!(canonical_config_for(&default, None).is_err());
    assert!(canonical_config_for(&files.0.join("other.yml"), Some(&default)).is_err());
    assert!(canonical_location(&files.0.join("config.yml/child.yml"), true).is_err());
    assert!(canonical_location(&files.0.join("../foreign.yml"), true).is_err());
    assert!(!default.exists());
    assert!(!files.0.join(".zi-devtools").exists());
    // Existing custom config remains valid, without reading its contents.
    assert!(canonical_config_for(&files.0.join("config.yml"), Some(&default)).is_ok());
}

#[test]
fn native_default_config_uses_the_effective_profile_layout() {
    let profile = crate::explorer::audit::native::effective_profile_dir().unwrap();
    let expected = canonical_location(&crate::paths::default_config_path(&profile), true).unwrap();
    assert_eq!(msi_default_config_path().unwrap(), expected);
}

#[test]
fn package_candidate_is_bounded_metadata_and_not_proof_of_installed_files() {
    let files = Files::new();
    let target = files.0.join("not-installed");
    let declaration = crate::capability_manifest::Manifest::current()
        .encode_msi_property()
        .unwrap();
    let candidate = msi_candidate(&target, "9.8.7", &"a".repeat(64), &declaration).unwrap();
    assert_eq!(candidate.identity, Identity::Msi);
    assert_eq!(
        candidate.menu_capabilities.as_deref(),
        Some(
            std::str::from_utf8(&crate::capability_manifest::Manifest::current().encode()).unwrap()
        )
    );
    assert_eq!(candidate.executable.file_name().unwrap(), "ZiDevTools.exe");
    assert!(!target.exists());
    assert!(VerifiedFile::verify(&candidate).is_err());
    assert!(
        msi_candidate(
            Path::new("relative"),
            "9.8.7",
            &"a".repeat(64),
            &declaration
        )
        .is_err()
    );
    assert!(msi_candidate(&target, "9.8.7-dev.1", &"a".repeat(64), &declaration).is_err());
    assert!(msi_candidate(&target, "9.8.7", "not-a-digest", &declaration).is_err());
    assert!(msi_candidate(&target, "9.8.7", &"a".repeat(64), "not-a-capability").is_err());
    assert!(
        msi_candidate(
            &target,
            "9.8.7",
            &"a".repeat(64),
            "ZiDevTools/ShellCapabilities/1|"
        )
        .is_ok()
    );
    assert!(
        msi_candidate(
            &target,
            "9.8.7",
            &"a".repeat(64),
            &format!("ZiDevTools/ShellCapabilities/1|{}", "x".repeat(240))
        )
        .is_err()
    );
    assert!(!target.exists());
}

#[test]
fn controlled_preview_is_rejected_before_file_or_registry_application() {
    let files = Files::new();
    let record = VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml"))
        .unwrap()
        .record(Identity::portable(), "9.8.7")
        .unwrap();
    let config = files.0.join("config.yml");
    let ledger = prepare(
        &Ledger::default(),
        Change::Put {
            installation: record.clone(),
            expected_generation: None,
            activate: true,
        },
        &Eligible(vec![record]),
    )
    .unwrap();
    let ticket = Ticket {
        prepared: joint::unapplicable_fixture(ledger),
        binding: binding(&config).unwrap(),
        title: "受控预览",
        preview_only: true,
    };
    assert!(
        apply(ticket, &config)
            .err()
            .unwrap()
            .to_string()
            .contains("受控预览禁止")
    );
}
