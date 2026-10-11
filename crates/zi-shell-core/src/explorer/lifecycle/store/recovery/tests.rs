use super::super::super::tests::{put, sample};
use super::super::joint::tests::Fixture as MenuFixture;
use super::*;

struct Fixture {
    menu: MenuFixture,
    scope: Scope,
}
impl Fixture {
    fn new() -> Self {
        let menu = MenuFixture::new();
        let scope = Scope {
            menu: menu.scope.clone(),
            journal: format!("{}-Recovery", menu.scope.ledger),
        };
        Self { menu, scope }
    }
    fn state(&self, id: &str) -> Status {
        read_record(&self.scope, id, &Transaction::begin().unwrap())
            .unwrap()
            .state
            .status()
    }
    fn commit(&self, plan: Prepared) {
        let tx = Transaction::begin().unwrap();
        let preview = joint::preview_in(plan, &self.scope.menu, &tx).unwrap();
        joint::apply_in(&preview, &self.scope.menu, &tx).unwrap();
        tx.commit().unwrap();
    }
    fn change(
        &self,
        id: &str,
        action: Action,
        eligible: Vec<Installation>,
        commit: bool,
    ) -> Result<Ledger> {
        transition(&self.scope, id, &Eligible(eligible), action, commit)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(
            self.scope
                .journal
                .starts_with(r"Software\ZiCode\ZiDevTools\Tests\ExplorerJoint-")
        );
        let context = crate::explorer::audit::native::UserContext::capture().unwrap();
        unsafe {
            RegDeleteTreeW(context.user.0, wide(&self.scope.journal).as_ptr());
        }
    }
}
fn record(identity: Identity) -> Installation {
    let mut m = sample(identity);
    m.executable_sha256 = Some("a".repeat(64));
    m
}
fn install_intent(m: &Installation, expected_generation: Option<String>) -> Intent {
    Intent::Install {
        installation: m.clone(),
        expected_generation,
        activate: true,
    }
}

#[test]
fn journal_listing_is_read_only_bounded_metadata_and_validates_every_record() {
    let fixture = Fixture::new();
    assert!(list_at(&fixture.scope).unwrap().is_empty());
    assert!(
        root(&fixture.scope, &Transaction::begin().unwrap(), false)
            .unwrap()
            .is_none()
    );
    let m = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&m, None)).unwrap();
    let expected = |status| {
        vec![Summary {
            operation_id: id.clone(),
            status,
            operation: Operation::Install {
                version: m.version.clone(),
            },
        }]
    };
    assert_eq!(list_at(&fixture.scope).unwrap(), expected(Status::Prepared));
    assert!(forget_at(&fixture.scope, &id).is_err());
    fixture
        .change(&id, Action::Execute, vec![m.clone()], true)
        .unwrap();
    let ledger = fixture.menu.ledger();
    let menu = fixture.menu.menu();
    assert_eq!(list_at(&fixture.scope).unwrap(), expected(Status::Applied));
    assert_eq!(fixture.menu.ledger(), ledger);
    assert_eq!(fixture.menu.menu(), menu);
    assert!(forget_at(&fixture.scope, &id).is_err());
    fixture.change(&id, Action::Complete, vec![], true).unwrap();
    assert_eq!(
        list_at(&fixture.scope).unwrap(),
        expected(Status::Completed)
    );
    let ledger = fixture.menu.ledger();
    let menu = fixture.menu.menu();
    forget_at(&fixture.scope, &id).unwrap();
    assert!(list_at(&fixture.scope).unwrap().is_empty());
    assert_eq!(fixture.menu.ledger(), ledger);
    assert_eq!(fixture.menu.menu(), menu);
    // Unknown root content must fail the whole read, never hide broken records.
    let tx = Transaction::begin().unwrap();
    let (key, _) = root(&fixture.scope, &tx, false).unwrap().unwrap();
    success(unsafe {
        RegSetValueExW(
            key.0,
            wide("Unknown").as_ptr(),
            0,
            REG_BINARY,
            [1u8].as_ptr(),
            1,
        )
    })
    .unwrap();
    drop(key);
    tx.commit().unwrap();
    assert!(list_at(&fixture.scope).is_err());
    assert_eq!(fixture.menu.ledger(), ledger);
    assert_eq!(fixture.menu.menu(), menu);
}

#[test]
fn protected_journal_install_and_restore_are_jointly_atomic_and_callbacks_are_idempotent() {
    let fixture = Fixture::new();
    let m = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&m, None)).unwrap();
    assert_eq!(fixture.state(&id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    assert!(fixture.menu.menu().keys.is_empty());
    assert!(forget_at(&fixture.scope, &id).is_err());
    assert!(prepare_at(&fixture.scope, &id, install_intent(&m, None)).is_err());
    fixture
        .change(&id, Action::Execute, vec![m.clone()], false)
        .unwrap();
    assert_eq!(fixture.state(&id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    assert!(fixture.menu.menu().keys.is_empty());
    fixture
        .change(&id, Action::Execute, vec![m.clone()], true)
        .unwrap();
    let current = fixture.menu.ledger();
    let menu = fixture.menu.menu();
    assert_eq!(fixture.state(&id), Status::Applied);
    assert_eq!(current.active(), Some(&m));
    assert_eq!(current.pending_msi_operation(), Some(id.as_str()));
    assert_eq!(
        fixture.change(&id, Action::Execute, vec![], true).unwrap(),
        current
    );
    fixture
        .change(&id, Action::Rollback, vec![], false)
        .unwrap();
    assert_eq!(fixture.state(&id), Status::Applied);
    assert_eq!(fixture.menu.ledger(), current);
    assert_eq!(fixture.menu.menu(), menu);
    let restored = fixture.change(&id, Action::Rollback, vec![], true).unwrap();
    assert!(restored.entries().is_empty());
    assert!(restored.pending_msi_operation().is_none());
    assert_eq!(fixture.state(&id), Status::Reverted);
    assert!(fixture.menu.menu().keys.is_empty());
    assert_eq!(
        fixture.change(&id, Action::Rollback, vec![], true).unwrap(),
        restored
    );
    assert!(fixture.change(&id, Action::Complete, vec![], true).is_err());
    assert!(fixture.change(&id, Action::Execute, vec![m], true).is_err());
    forget_at(&fixture.scope, &id).unwrap();
    assert!(read_record(&fixture.scope, &id, &Transaction::begin().unwrap()).is_err());
}

#[test]
fn upgrade_rollback_preserves_a_later_portable_selection() {
    let fixture = Fixture::new();
    let old = record(Identity::Msi);
    fixture.commit(put(
        &Ledger::default(),
        old.clone(),
        None,
        std::slice::from_ref(&old),
    ));
    let new = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(
        &fixture.scope,
        &id,
        install_intent(&new, Some(old.generation.clone())),
    )
    .unwrap();
    fixture
        .change(&id, Action::Execute, vec![new.clone()], true)
        .unwrap();
    let portable = record(Identity::portable());
    fixture.commit(put(
        &fixture.menu.ledger(),
        portable.clone(),
        None,
        &[portable.clone(), new],
    ));
    let before = fixture.menu.ledger();
    let restored = fixture
        .change(
            &id,
            Action::Rollback,
            vec![old.clone(), portable.clone()],
            true,
        )
        .unwrap();
    assert_eq!(restored.active(), Some(&portable));
    assert_eq!(restored.revision(), before.revision() + 1);
    assert!(restored.entries().iter().any(|e| e.installation == old));
    assert!(restored.pending_msi_operation().is_none());
    assert_eq!(fixture.state(&id), Status::Reverted);
}

#[test]
fn uninstall_completion_releases_reserved_capacity_and_cannot_be_rolled_back() {
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    fixture.commit(put(
        &Ledger::default(),
        msi.clone(),
        None,
        std::slice::from_ref(&msi),
    ));
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(
        &fixture.scope,
        &id,
        Intent::Remove {
            expected_generation: msi.generation.clone(),
        },
    )
    .unwrap();
    let removed = fixture.change(&id, Action::Execute, vec![], true).unwrap();
    assert!(removed.entries().is_empty());
    assert_eq!(removed.pending_msi_operation(), Some(id.as_str()));
    assert!(forget_at(&fixture.scope, &id).is_err());
    fixture
        .change(&id, Action::Complete, vec![], false)
        .unwrap();
    assert_eq!(fixture.menu.ledger(), removed);
    assert_eq!(fixture.state(&id), Status::Applied);
    let complete = fixture.change(&id, Action::Complete, vec![], true).unwrap();
    assert!(complete.pending_msi_operation().is_none());
    assert_eq!(complete.revision(), removed.revision() + 1);
    assert_eq!(fixture.state(&id), Status::Completed);
    assert_eq!(
        fixture.change(&id, Action::Complete, vec![], true).unwrap(),
        complete
    );
    assert!(
        fixture
            .change(&id, Action::Rollback, vec![msi], true)
            .is_err()
    );
    forget_at(&fixture.scope, &id).unwrap();
    assert!(read_record(&fixture.scope, &id, &Transaction::begin().unwrap()).is_err());
}

#[test]
fn prepared_rollback_and_stale_or_unverified_execute_never_change_menus() {
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
    assert!(fixture.change(&id, Action::Execute, vec![], true).is_err());
    assert_eq!(fixture.state(&id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    assert!(fixture.menu.menu().keys.is_empty());
    fixture.change(&id, Action::Rollback, vec![], true).unwrap();
    assert_eq!(fixture.state(&id), Status::Reverted);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    let stale_id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &stale_id, install_intent(&msi, None)).unwrap();
    let replacement = record(Identity::Msi);
    fixture.commit(put(
        &Ledger::default(),
        replacement.clone(),
        None,
        std::slice::from_ref(&replacement),
    ));
    let before = fixture.menu.ledger();
    let menu = fixture.menu.menu();
    assert!(
        fixture
            .change(&stale_id, Action::Execute, vec![msi], true)
            .is_err()
    );
    assert_eq!(fixture.state(&stale_id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), before);
    assert_eq!(fixture.menu.menu(), menu);
}

fn raw_state(fixture: &Fixture, id: &str, bytes: &[u8], kind: u32) {
    let tx = Transaction::begin().unwrap();
    let root = security::open_protected(&fixture.scope.journal, &tx, false)
        .unwrap()
        .unwrap();
    let key = security::open_child(root.0, id, &tx, false)
        .unwrap()
        .unwrap();
    success(unsafe {
        RegSetValueExW(
            key.0,
            wide("State").as_ptr(),
            0,
            kind,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    })
    .unwrap();
    drop(key);
    drop(root);
    tx.commit().unwrap();
}

#[test]
fn corrupt_protocol_identity_intent_and_undo_fail_without_repair_or_deletion() {
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
    fixture
        .change(&id, Action::Execute, vec![msi], true)
        .unwrap();
    let before = fixture.menu.ledger();
    let menu = fixture.menu.menu();
    let original = serde_json::to_value(
        read_record(&fixture.scope, &id, &Transaction::begin().unwrap()).unwrap(),
    )
    .unwrap();
    for field in [
        "schema_version",
        "owner",
        "operation_id",
        "unknown",
        "intent",
        "undo",
    ] {
        let mut changed = original.clone();
        match field {
            "schema_version" => changed[field] = serde_json::json!(999),
            "owner" => changed[field] = serde_json::json!("foreign"),
            "operation_id" => changed[field] = serde_json::json!(uuid::Uuid::new_v4().to_string()),
            "unknown" => changed[field] = serde_json::json!(true),
            "intent" => {
                changed["intent"]["installation"]["generation"] =
                    serde_json::json!(uuid::Uuid::new_v4().to_string())
            }
            "undo" => {
                changed["state"]["undo"]["operation_id"] =
                    serde_json::json!(uuid::Uuid::new_v4().to_string())
            }
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&changed).unwrap();
        raw_state(&fixture, &id, &bytes, REG_BINARY);
        assert!(
            fixture.change(&id, Action::Rollback, vec![], true).is_err(),
            "{field}"
        );
        assert!(forget_at(&fixture.scope, &id).is_err(), "{field}");
        assert_eq!(fixture.menu.ledger(), before);
        assert_eq!(fixture.menu.menu(), menu);
        let tx = Transaction::begin().unwrap();
        let key = open(&format!("{}\\{id}", fixture.scope.journal), &tx, false)
            .unwrap()
            .unwrap();
        let mut stored = vec![0u8; bytes.len()];
        let mut len = stored.len() as u32;
        success(unsafe {
            RegQueryValueExW(
                key.0,
                wide("State").as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                stored.as_mut_ptr(),
                &mut len,
            )
        })
        .unwrap();
        assert_eq!(stored, bytes);
    }
}

#[test]
fn invalid_encoding_oversize_and_unknown_children_are_rejected() {
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
    let original = serde_json::to_vec(
        &read_record(&fixture.scope, &id, &Transaction::begin().unwrap()).unwrap(),
    )
    .unwrap();
    for (bytes, kind) in [
        (b"not-json".to_vec(), REG_BINARY),
        (original.clone(), REG_SZ),
        (vec![0; MAX_RECORD_BYTES + 1], REG_BINARY),
    ] {
        raw_state(&fixture, &id, &bytes, kind);
        assert!(fixture.change(&id, Action::Rollback, vec![], true).is_err());
        assert_eq!(fixture.menu.ledger(), Ledger::default());
    }
    raw_state(&fixture, &id, &original, REG_BINARY);
    let tx = Transaction::begin().unwrap();
    drop(
        open(
            &format!("{}\\{id}\\Foreign", fixture.scope.journal),
            &tx,
            true,
        )
        .unwrap(),
    );
    tx.commit().unwrap();
    assert!(fixture.change(&id, Action::Rollback, vec![], true).is_err());
    assert!(forget_at(&fixture.scope, &id).is_err());
}

#[test]
fn journal_capacity_preserves_pending_records_until_explicit_terminal_cleanup() {
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    let mut ids = Vec::new();
    for _ in 0..MAX_RECORDS {
        let id = uuid::Uuid::new_v4().to_string();
        prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
        ids.push(id);
    }
    let extra = uuid::Uuid::new_v4().to_string();
    assert!(prepare_at(&fixture.scope, &extra, install_intent(&msi, None)).is_err());
    for id in &ids {
        assert_eq!(fixture.state(id), Status::Prepared);
    }
    assert!(forget_at(&fixture.scope, &ids[0]).is_err());
    fixture
        .change(&ids[0], Action::Rollback, vec![], true)
        .unwrap();
    forget_at(&fixture.scope, &ids[0]).unwrap();
    prepare_at(&fixture.scope, &extra, install_intent(&msi, None)).unwrap();
    assert_eq!(fixture.state(&extra), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
}

#[test]
fn broadened_native_dacl_is_rejected_without_adoption() {
    use windows_sys::Win32::Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let fixture = Fixture::new();
    let msi = record(Identity::Msi);
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
    let context = crate::explorer::audit::native::UserContext::capture().unwrap();
    let sid = &context.identity.0;
    let sddl = wide(&format!(
        "O:{sid}D:P(A;;KA;;;{sid})(A;;KA;;;SY)(A;;KR;;;WD)"
    ));
    let mut descriptor = ptr::null_mut();
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        },
        0
    );
    let mut handle = ptr::null_mut();
    success(unsafe {
        RegOpenKeyExW(
            context.user.0,
            wide(&fixture.scope.journal).as_ptr(),
            REG_OPTION_OPEN_LINK,
            0x00040000,
            &mut handle,
        )
    })
    .unwrap();
    let key = Key(handle);
    let changed = unsafe {
        RegSetKeySecurity(
            key.0,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(descriptor);
    }
    success(changed).unwrap();
    drop(key);
    let tx = Transaction::begin().unwrap();
    assert!(security::open_protected(&fixture.scope.journal, &tx, false).is_err());
    drop(tx);
    for action in [Action::Execute, Action::Rollback, Action::Complete] {
        assert!(fixture.change(&id, action, vec![], true).is_err());
    }
    assert!(forget_at(&fixture.scope, &id).is_err());
    assert!(
        prepare_at(
            &fixture.scope,
            &uuid::Uuid::new_v4().to_string(),
            install_intent(&record(Identity::Msi), None)
        )
        .is_err()
    );
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    assert!(fixture.menu.menu().keys.is_empty());
    // Validation did not silently replace the unexpected DACL.
    assert!(
        security::open_protected(
            &fixture.scope.journal,
            &Transaction::begin().unwrap(),
            false
        )
        .is_err()
    );
}

#[test]
fn unknown_root_children_and_values_are_preserved_and_block_journal_access() {
    for extra_child in [false, true] {
        let fixture = Fixture::new();
        let msi = record(Identity::Msi);
        let id = uuid::Uuid::new_v4().to_string();
        prepare_at(&fixture.scope, &id, install_intent(&msi, None)).unwrap();
        let tx = Transaction::begin().unwrap();
        if extra_child {
            drop(open(&format!("{}\\Foreign", fixture.scope.journal), &tx, true).unwrap());
        } else {
            let root = security::open_protected(&fixture.scope.journal, &tx, false)
                .unwrap()
                .unwrap();
            success(unsafe {
                RegSetValueExW(
                    root.0,
                    wide("Foreign").as_ptr(),
                    0,
                    REG_BINARY,
                    b"keep".as_ptr(),
                    4,
                )
            })
            .unwrap();
        }
        tx.commit().unwrap();
        assert!(fixture.change(&id, Action::Rollback, vec![], true).is_err());
        assert!(forget_at(&fixture.scope, &id).is_err());
        assert_eq!(fixture.menu.ledger(), Ledger::default());
        let tx = Transaction::begin().unwrap();
        if extra_child {
            assert!(
                open(&format!("{}\\Foreign", fixture.scope.journal), &tx, false)
                    .unwrap()
                    .is_some()
            );
        } else {
            let root = open(&fixture.scope.journal, &tx, false).unwrap().unwrap();
            let mut bytes = [0u8; 4];
            let mut len = 4;
            success(unsafe {
                RegQueryValueExW(
                    root.0,
                    wide("Foreign").as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    bytes.as_mut_ptr(),
                    &mut len,
                )
            })
            .unwrap();
            assert_eq!(&bytes, b"keep");
        }
    }
}

// Invoked inside the disposable CI user's impersonation by the existing gate.
pub(in crate::explorer::lifecycle::store) fn cross_account_journal_probe() -> String {
    protected_journal_install_and_restore_are_jointly_atomic_and_callbacks_are_idempotent();
    broadened_native_dacl_is_rejected_without_adoption();
    verified_execution_and_recovery_require_actual_files_at_the_correct_phase();
    assert!(
        !super::super::super::installer::msi_default_config_path()
            .unwrap()
            .exists()
    );
    verified_install_accepts_native_default_config_without_creating_user_files();
    let fixture = Fixture::new();
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(
        &fixture.scope,
        &id,
        install_intent(&record(Identity::Msi), None),
    )
    .unwrap();
    fixture.scope.journal.clone()
}

struct ActualFiles(PathBuf);
impl ActualFiles {
    fn new() -> Self {
        // A thread impersonating another user must not inherit the process's
        // TEMP environment (which may be inaccessible to that user).
        let root = crate::explorer::audit::native::effective_profile_dir()
            .unwrap()
            .join(format!("zi-msi-files-proof-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("app.exe"), b"MZoriginal synthetic file").unwrap();
        std::fs::write(
            root.join("config.yml"),
            b"synthetic configuration, never read",
        )
        .unwrap();
        Self(root)
    }
    fn installation(&self, content: &[u8]) -> Installation {
        use sha2::{Digest, Sha256};
        let mut value = Installation::new(
            Identity::Msi,
            std::fs::canonicalize(self.0.join("app.exe")).unwrap(),
            std::fs::canonicalize(self.0.join("config.yml")).unwrap(),
            "9.8.7".into(),
        );
        value.executable_sha256 = Some(format!("{:x}", Sha256::digest(content)));
        value
    }
}
impl Drop for ActualFiles {
    fn drop(&mut self) {
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("zi-msi-files-proof-")
        );
        for leaf in ["app.exe", "config.yml"] {
            if self.0.join(leaf).exists() {
                std::fs::remove_file(self.0.join(leaf)).unwrap();
            }
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn verified_execution_and_recovery_require_actual_files_at_the_correct_phase() {
    let fixture = Fixture::new();
    let files = ActualFiles::new();
    let old = files.installation(b"MZoriginal synthetic file");
    fixture.commit(put(
        &Ledger::default(),
        old.clone(),
        None,
        std::slice::from_ref(&old),
    ));
    let new = files.installation(b"MZnew synthetic file");
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(
        &fixture.scope,
        &id,
        install_intent(&new, Some(old.generation.clone())),
    )
    .unwrap();
    let before = fixture.menu.ledger();
    let menus = fixture.menu.menu();
    // The expected new digest is not proof that new files are installed.
    assert!(transition_verified(&fixture.scope, &id, Action::Execute).is_err());
    assert_eq!(fixture.state(&id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), before);
    assert_eq!(fixture.menu.menu(), menus);
    std::fs::write(files.0.join("app.exe"), b"MZnew synthetic file").unwrap();
    let applied = transition_verified(&fixture.scope, &id, Action::Execute).unwrap();
    assert_eq!(applied.active(), Some(&new));
    assert_eq!(fixture.state(&id), Status::Applied);
    // Recovery cannot claim old files were restored while the new file remains.
    assert!(transition_verified(&fixture.scope, &id, Action::Rollback).is_err());
    assert_eq!(fixture.menu.ledger(), applied);
    assert_eq!(fixture.state(&id), Status::Applied);
    std::fs::write(files.0.join("app.exe"), b"MZoriginal synthetic file").unwrap();
    let restored = transition_verified(&fixture.scope, &id, Action::Rollback).unwrap();
    assert_eq!(restored.active(), Some(&old));
    assert_eq!(fixture.state(&id), Status::Reverted);
}

#[test]
fn verified_install_accepts_native_default_config_without_creating_user_files() {
    let fixture = Fixture::new();
    let files = ActualFiles::new();
    let default = super::super::super::installer::msi_default_config_path().unwrap();
    let config_existed = default.exists();
    let parent_existed = default.parent().unwrap().exists();
    let mut value = files.installation(b"MZoriginal synthetic file");
    value.config = default.clone();
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&value, None)).unwrap();
    let installed = transition_verified(&fixture.scope, &id, Action::Execute).unwrap();
    assert_eq!(installed.active(), Some(&value));
    transition_verified(&fixture.scope, &id, Action::Rollback).unwrap();
    assert_eq!(fixture.menu.ledger().entries().len(), 0);
    assert_eq!(default.exists(), config_existed);
    assert_eq!(default.parent().unwrap().exists(), parent_existed);
}

#[test]
fn verified_install_rejects_a_missing_custom_config_without_changing_registry() {
    let fixture = Fixture::new();
    let files = ActualFiles::new();
    let value = files.installation(b"MZoriginal synthetic file");
    std::fs::remove_file(files.0.join("config.yml")).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    prepare_at(&fixture.scope, &id, install_intent(&value, None)).unwrap();
    assert!(transition_verified(&fixture.scope, &id, Action::Execute).is_err());
    assert_eq!(fixture.state(&id), Status::Prepared);
    assert_eq!(fixture.menu.ledger(), Ledger::default());
    assert!(fixture.menu.menu().keys.is_empty());
    assert!(!files.0.join("config.yml").exists());
}
