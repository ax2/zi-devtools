use super::*;

#[test]
fn manual_msi_removal_and_relocation_refused_before_registry_access() {
    for request in [
        Request::Remove(Identity::Msi),
        Request::Relocate(Identity::Msi),
    ] {
        assert!(
            preview(Path::new("missing-config.yml"), request)
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
            b"MZsynthetic fixture, never executed",
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
    let record = verified.record(Identity::portable());
    assert!(std::fs::write(&record.executable, b"MZchanged").is_err());
    assert!(std::fs::remove_file(&record.executable).is_err());
    let check = VerifiedFile::verify(&record).unwrap();
    drop(check);
    drop(verified);
    std::fs::write(&record.executable, b"MZchanged").unwrap();
    assert!(VerifiedFile::verify(&record).is_err());
    std::fs::write(files.0.join("sample.exe"), b"not an executable").unwrap();
    assert!(VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml")).is_err());
}

#[test]
fn controlled_preview_is_rejected_before_file_or_registry_application() {
    let files = Files::new();
    let record = VerifiedFile::open(&files.0.join("sample.exe"), &files.0.join("config.yml"))
        .unwrap()
        .record(Identity::portable());
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
