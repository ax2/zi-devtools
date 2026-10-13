use super::super::super::tests::{put, sample};
use super::*;

pub(in crate::explorer::lifecycle::store) struct Fixture {
    pub(in crate::explorer::lifecycle::store) scope: Scope,
    classes_root: String,
    alias_root: String,
}
static FIXTURE_SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn native_capability_snapshot_and_menu_subset_rollback_together() {
    let fixture = Fixture::new();
    let mut portable = record(Identity::portable());
    portable.menu_capabilities =
        Some("ZiDevTools/ShellCapabilities/1\nchoose-files\nimport:csv\n".into());
    let initial = preview_at(
        put(
            &Ledger::default(),
            portable.clone(),
            None,
            &[portable.clone()],
        ),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&initial, &fixture.scope, true).unwrap();
    let old_menu = fixture.menu();
    assert_eq!(
        old_menu
            .values
            .keys()
            .filter(|(key, _)| key.ends_with("\\command"))
            .count(),
        2
    );
    let mut msi = record(Identity::Msi);
    msi.menu_capabilities =
        Some(String::from_utf8(crate::capability_manifest::Manifest::current().encode()).unwrap());
    let operation = super::super::super::msi_operation::install(
        &fixture.ledger(),
        msi.clone(),
        true,
        &Eligible(vec![msi.clone(), portable.clone()]),
    )
    .unwrap();
    let forward = preview_at(operation.plan().clone(), &fixture.scope).unwrap();
    apply_at(&forward, &fixture.scope, true).unwrap();
    let full_ledger = fixture.ledger();
    let full_menu = fixture.menu();
    assert_eq!(
        full_menu
            .values
            .keys()
            .filter(|(key, _)| key.ends_with("\\command"))
            .count(),
        12
    );
    let backward = preview_at(
        operation
            .undo()
            .rollback(&full_ledger, &Eligible(vec![portable.clone()]))
            .unwrap(),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&backward, &fixture.scope, false).unwrap();
    assert_eq!(fixture.ledger(), full_ledger);
    assert_eq!(fixture.menu(), full_menu);
    apply_at(&backward, &fixture.scope, true).unwrap();
    assert_eq!(fixture.ledger().active(), Some(&portable));
    assert_eq!(fixture.menu(), old_menu);
}

#[test]
fn native_full_ledger_uninstall_reserves_undo_capacity_and_finalizes_atomically() {
    use super::super::super::msi_operation;
    for restore in [false, true] {
        let fixture = Fixture::new();
        let commit = |plan| {
            let preview = preview_at(plan, &fixture.scope).unwrap();
            apply_at(&preview, &fixture.scope, true).unwrap();
        };
        let m = record(Identity::Msi);
        let first = msi_operation::install(
            &Ledger::default(),
            m.clone(),
            true,
            &Eligible(vec![m.clone()]),
        )
        .unwrap();
        commit(first.plan().clone());
        commit(first.undo().complete(&fixture.ledger()).unwrap());
        for _ in 0..super::super::super::MAX_INSTALLATIONS - 1 {
            let p = record(Identity::portable());
            commit(put(
                &fixture.ledger(),
                p.clone(),
                None,
                std::slice::from_ref(&p),
            ));
        }
        commit(
            prepare(
                &fixture.ledger(),
                Change::Activate(Identity::Msi),
                &Eligible(vec![m.clone()]),
            )
            .unwrap(),
        );
        let eligible = |ledger: &Ledger| {
            Eligible(
                ledger
                    .entries()
                    .iter()
                    .map(|e| e.installation.clone())
                    .collect(),
            )
        };
        let op = msi_operation::remove(
            &fixture.ledger(),
            Some(&m.generation),
            &eligible(&fixture.ledger()),
        )
        .unwrap()
        .unwrap();
        commit(op.plan().clone());
        let current = fixture.ledger();
        let menu = fixture.menu();
        assert_eq!(current.entries().len(), 31);
        assert_eq!(
            current.pending_msi_operation(),
            Some(op.undo().operation_id())
        );
        let extra = record(Identity::portable());
        assert!(
            prepare(
                &current,
                Change::Put {
                    installation: extra.clone(),
                    expected_generation: None,
                    activate: false
                },
                &eligible(&current)
            )
            .is_err()
        );
        assert_eq!(fixture.ledger(), current);
        assert_eq!(fixture.menu(), menu);
        let old = current.entries()[0].installation.clone();
        let mut updated = old.clone();
        updated.generation = uuid::Uuid::new_v4().to_string();
        let mut available = eligible(&current);
        available.0.retain(|p| p.identity != old.identity);
        available.0.push(updated.clone());
        commit(
            prepare(
                &current,
                Change::Put {
                    installation: updated.clone(),
                    expected_generation: Some(old.generation),
                    activate: true,
                },
                &available,
            )
            .unwrap(),
        );
        let current = fixture.ledger();
        let menu = fixture.menu();
        let mut available = eligible(&current);
        available.0.push(m.clone());
        let plan = if restore {
            op.undo().rollback(&current, &available).unwrap()
        } else {
            op.undo().complete(&current).unwrap()
        };
        let final_preview = preview_at(plan, &fixture.scope).unwrap();
        apply_at(&final_preview, &fixture.scope, false).unwrap();
        assert_eq!(fixture.ledger(), current);
        assert_eq!(fixture.menu(), menu);
        apply_at(&final_preview, &fixture.scope, true).unwrap();
        let final_ledger = fixture.ledger();
        assert!(final_ledger.pending_msi_operation().is_none());
        assert_eq!(final_ledger.active(), Some(&updated));
        assert_eq!(final_ledger.entries().len(), if restore { 32 } else { 31 });
        let portable_entries: Vec<_> = final_ledger
            .entries()
            .iter()
            .filter(|e| e.installation.identity != Identity::Msi)
            .cloned()
            .collect();
        assert_eq!(portable_entries, current.entries());
        assert_eq!(fixture.menu(), final_preview.after);
        assert!(op.undo().rollback(&final_ledger, &available).is_err());
        assert!(op.undo().complete(&final_ledger).is_err());
        if !restore {
            commit(put(
                &final_ledger,
                extra.clone(),
                None,
                std::slice::from_ref(&extra),
            ));
            assert_eq!(fixture.ledger().entries().len(), 32);
        }
        fixture.clear();
        assert_eq!(fixture.ledger(), Ledger::default());
        assert!(fixture.menu().keys.is_empty());
    }
}

#[test]
#[ignore = "requires an explicitly created disposable CI user with a loaded profile"]
fn cross_account_joint_transaction_does_not_use_cached_process_hkcu() {
    let original_config = super::super::super::installer::msi_default_config_path().unwrap();
    let (scope, prepared, journal_path, other_config) = audit::native::with_ci_user(|| {
        let fixture = Fixture::new();
        let p = record(Identity::portable());
        let prepared = preview_at(
            put(
                &Ledger::default(),
                p.clone(),
                None,
                std::slice::from_ref(&p),
            ),
            &fixture.scope,
        )
        .unwrap();
        apply_at(&prepared, &fixture.scope, false).unwrap();
        assert_eq!(fixture.ledger(), Ledger::default());
        assert!(fixture.menu().keys.is_empty());
        apply_at(&prepared, &fixture.scope, true).unwrap();
        assert_eq!(fixture.ledger().active(), Some(&p));
        assert_eq!(fixture.menu(), prepared.after);
        let scope = fixture.scope.clone();
        fixture.clear();
        assert_eq!(fixture.ledger(), Ledger::default());
        assert!(fixture.menu().keys.is_empty());
        let journal_path = super::super::recovery::tests::cross_account_journal_probe();
        let other_config = super::super::super::installer::msi_default_config_path().unwrap();
        (scope, prepared, journal_path, other_config)
    });
    assert_ne!(original_config, other_config);
    // Now back under the original process user. No corresponding keys may exist.
    assert!(
        open(&journal_path, &Transaction::begin().unwrap(), false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        read(&scope.ledger, &Transaction::begin().unwrap()).unwrap(),
        Ledger::default()
    );
    assert!(
        audit::native::read_tx(&scope.roots(), None)
            .unwrap()
            .keys
            .is_empty()
    );
    let error = apply_at(&prepared, &scope, true).err().unwrap();
    assert!(error.to_string().contains("操作用户已变化"));
    assert_eq!(
        read(&scope.ledger, &Transaction::begin().unwrap()).unwrap(),
        Ledger::default()
    );
    assert!(
        audit::native::read_tx(&scope.roots(), None)
            .unwrap()
            .keys
            .is_empty()
    );
}

#[test]
fn native_joint_transaction_uses_effective_thread_user_and_rejects_foreign_ticket() {
    audit::native::with_impersonated_user(|| {
        let fixture = Fixture::new();
        let p = record(Identity::portable());
        let prepared = preview_at(
            put(
                &Ledger::default(),
                p.clone(),
                None,
                std::slice::from_ref(&p),
            ),
            &fixture.scope,
        )
        .unwrap();
        let mut foreign = prepared.clone();
        foreign.user = audit::native::UserIdentity("different-fixture-user".into());
        assert!(apply_at(&foreign, &fixture.scope, true).is_err());
        assert_eq!(fixture.ledger(), Ledger::default());
        assert!(fixture.menu().keys.is_empty());
        apply_at(&prepared, &fixture.scope, false).unwrap();
        assert_eq!(fixture.ledger(), Ledger::default());
        assert!(fixture.menu().keys.is_empty());
        apply_at(&prepared, &fixture.scope, true).unwrap();
        assert_eq!(fixture.ledger().active(), Some(&p));
        assert_eq!(fixture.menu(), prepared.after);
        fixture.clear();
        assert!(fixture.menu().keys.is_empty());
        assert_eq!(fixture.ledger(), Ledger::default());
    });
}

#[test]
fn msi_undo_preserves_later_portable_selection_and_migrates_native_legacy_ledger() {
    use super::super::super::msi_operation;
    let fixture = Fixture::new();
    let p = record(Identity::portable());
    let first = preview_at(
        put(
            &Ledger::default(),
            p.clone(),
            None,
            std::slice::from_ref(&p),
        ),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&first, &fixture.scope, true).unwrap();
    // Write an authentic old schema fixture without changing its existing menu.
    let mut legacy = fixture.ledger();
    legacy.schema_version = 1;
    legacy.msi_cursor = None;
    legacy.activation_cursor = None;
    let tx = Transaction::begin().unwrap();
    write(&fixture.scope.ledger, &legacy, &tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(fixture.ledger(), legacy);
    let m = record(Identity::Msi);
    let op = msi_operation::install(
        &legacy,
        m.clone(),
        true,
        &Eligible(vec![p.clone(), m.clone()]),
    )
    .unwrap();
    let forward = preview_at(op.plan().clone(), &fixture.scope).unwrap();
    apply_at(&forward, &fixture.scope, false).unwrap();
    assert_eq!(fixture.ledger(), legacy);
    assert_eq!(fixture.menu(), first.after);
    apply_at(&forward, &fixture.scope, true).unwrap();
    assert_eq!(fixture.ledger().schema_version, 4);
    let added = record(Identity::portable());
    let later = preview_at(
        put(
            &fixture.ledger(),
            added.clone(),
            None,
            &[p.clone(), m, added.clone()],
        ),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&later, &fixture.scope, true).unwrap();
    let before = fixture.ledger();
    let undo = msi_operation::Undo::from_bytes(&op.undo().to_bytes().unwrap()).unwrap();
    let backward = preview_at(
        undo.rollback(&before, &Eligible(vec![p.clone(), added.clone()]))
            .unwrap(),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&backward, &fixture.scope, false).unwrap();
    assert_eq!(fixture.ledger(), before);
    assert_eq!(fixture.menu(), later.after);
    apply_at(&backward, &fixture.scope, true).unwrap();
    assert_eq!(fixture.ledger().active(), Some(&added));
    assert_eq!(fixture.ledger().entries().len(), 2);
    assert!(
        fixture
            .ledger()
            .entries()
            .iter()
            .any(|e| e.installation == p)
    );
    assert_eq!(fixture.menu(), backward.after);
    assert!(
        undo.rollback(&fixture.ledger(), &Eligible(vec![added]))
            .is_err()
    );
    fixture.clear();
    assert!(fixture.menu().keys.is_empty());
    assert_eq!(fixture.ledger(), Ledger::default());
}

#[test]
fn native_msi_upgrade_and_uninstall_inverse_commit_and_rollback_both_hives() {
    use super::super::super::msi_operation;
    for uninstall in [false, true] {
        let fixture = Fixture::new();
        let p = record(Identity::portable());
        let old = record(Identity::Msi);
        let portable = put(
            &Ledger::default(),
            p.clone(),
            None,
            std::slice::from_ref(&p),
        );
        let portable = preview_at(portable, &fixture.scope).unwrap();
        apply_at(&portable, &fixture.scope, true).unwrap();
        let base = put(
            &fixture.ledger(),
            old.clone(),
            None,
            &[p.clone(), old.clone()],
        );
        let initial = preview_at(base, &fixture.scope).unwrap();
        apply_at(&initial, &fixture.scope, true).unwrap();
        let op = if uninstall {
            msi_operation::remove(
                &fixture.ledger(),
                Some(&old.generation),
                &Eligible(vec![p.clone()]),
            )
            .unwrap()
            .unwrap()
        } else {
            let next = record(Identity::Msi);
            msi_operation::install(
                &fixture.ledger(),
                next.clone(),
                false,
                &Eligible(vec![p.clone(), next]),
            )
            .unwrap()
        };
        let forward = preview_at(op.plan().clone(), &fixture.scope).unwrap();
        apply_at(&forward, &fixture.scope, true).unwrap();
        let current = fixture.ledger();
        let backward = preview_at(
            op.undo()
                .rollback(&current, &Eligible(vec![p, old.clone()]))
                .unwrap(),
            &fixture.scope,
        )
        .unwrap();
        apply_at(&backward, &fixture.scope, false).unwrap();
        assert_eq!(fixture.ledger(), current);
        assert_eq!(fixture.menu(), forward.after);
        apply_at(&backward, &fixture.scope, true).unwrap();
        assert_eq!(fixture.ledger().active(), Some(&old));
        assert_eq!(fixture.menu(), initial.after);
        assert!(fixture.ledger().revision() > current.revision());
        fixture.clear();
        assert!(fixture.menu().keys.is_empty());
        assert_eq!(fixture.ledger(), Ledger::default());
    }
}
impl Fixture {
    pub(in crate::explorer::lifecycle::store) fn new() -> Self {
        let id = uuid::Uuid::new_v4();
        let ledger = format!(r"Software\ZiCode\ZiDevTools\Tests\ExplorerJoint-{id}");
        let classes_root = format!(r"ZiCode.DevTools.Tests\ExplorerJoint-{id}");
        let alias_root = format!("ZiCode.DevTools.Tests.ExplorerJointActions-{id}");
        let menus = [
            format!(r"Software\Classes\{classes_root}\parent"),
            format!(r"Software\Classes\{alias_root}"),
        ];
        let result = Self {
            scope: Scope { ledger, menus },
            classes_root,
            alias_root,
        };
        // Isolate test operations below pre-existing UUID parents. Concurrent
        // tests must not race to create their shared test container in KTM.
        let _setup = FIXTURE_SETUP.lock().unwrap();
        let tx = Transaction::begin().unwrap();
        let classes = audit::native::classes().unwrap();
        drop(open_at(classes.0, &result.classes_root, &tx, true).unwrap());
        drop(open(r"Software\ZiCode\ZiDevTools\Tests", &tx, true).unwrap());
        drop(classes);
        tx.commit().unwrap();
        result
    }
    pub(in crate::explorer::lifecycle::store) fn ledger(&self) -> Ledger {
        read(&self.scope.ledger, &Transaction::begin().unwrap()).unwrap()
    }
    pub(in crate::explorer::lifecycle::store) fn menu(&self) -> Snapshot {
        audit::native::read_tx(&self.scope.roots(), None).unwrap()
    }
    fn clear(&self) {
        assert!(
            self.scope
                .ledger
                .starts_with(r"Software\ZiCode\ZiDevTools\Tests\ExplorerJoint-")
        );
        assert!(
            self.classes_root
                .starts_with(r"ZiCode.DevTools.Tests\ExplorerJoint-")
        );
        assert!(
            self.alias_root
                .starts_with("ZiCode.DevTools.Tests.ExplorerJointActions-")
        );
        unsafe {
            if let Ok(user) = audit::native::UserContext::capture() {
                RegDeleteTreeW(user.user.0, wide(&self.scope.ledger).as_ptr());
                RegDeleteTreeW(user.classes.0, wide(&self.classes_root).as_ptr());
                RegDeleteTreeW(user.classes.0, wide(&self.alias_root).as_ptr());
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.clear();
    }
}
fn record(identity: Identity) -> Installation {
    let mut record = sample(identity);
    record.executable_sha256 = Some("a".repeat(64));
    record
}

#[test]
fn cross_hive_transaction_keeps_ledger_and_menus_atomic_in_all_install_orders() {
    for msi_first in [false, true] {
        for remove_first in [false, true] {
            let fixture = Fixture::new();
            let portable = record(Identity::portable());
            let msi = record(Identity::Msi);
            let (a, b) = if msi_first {
                (&msi, &portable)
            } else {
                (&portable, &msi)
            };
            let first = preview_at(
                put(&Ledger::default(), a.clone(), None, std::slice::from_ref(a)),
                &fixture.scope,
            )
            .unwrap();
            assert_eq!(first.counts().0, 49);
            apply_at(&first, &fixture.scope, false).unwrap();
            assert_eq!(fixture.ledger(), Ledger::default());
            assert!(fixture.menu().keys.is_empty());
            apply_at(&first, &fixture.scope, true).unwrap();
            assert_eq!(&fixture.ledger(), first.ledger());
            assert_eq!(fixture.menu(), first.after);
            let second = preview_at(
                put(first.ledger(), b.clone(), None, &[a.clone(), b.clone()]),
                &fixture.scope,
            )
            .unwrap();
            apply_at(&second, &fixture.scope, false).unwrap();
            assert_eq!(&fixture.ledger(), first.ledger());
            assert_eq!(fixture.menu(), first.after);
            apply_at(&second, &fixture.scope, true).unwrap();
            assert!(apply_at(&first, &fixture.scope, true).is_err());
            let (removed, retained) = if remove_first { (a, b) } else { (b, a) };
            let removal = prepare(
                second.ledger(),
                Change::Remove {
                    identity: removed.identity.clone(),
                    expected_generation: removed.generation.clone(),
                },
                &Eligible(vec![retained.clone()]),
            )
            .unwrap();
            let removal = preview_at(removal, &fixture.scope).unwrap();
            apply_at(&removal, &fixture.scope, true).unwrap();
            assert_eq!(fixture.ledger().active(), Some(retained));
            assert_eq!(fixture.menu(), removal.after);
            let empty = prepare(
                removal.ledger(),
                Change::Remove {
                    identity: retained.identity.clone(),
                    expected_generation: retained.generation.clone(),
                },
                &Eligible(vec![]),
            )
            .unwrap();
            let empty = preview_at(empty, &fixture.scope).unwrap();
            apply_at(&empty, &fixture.scope, false).unwrap();
            assert_eq!(&fixture.ledger(), removal.ledger());
            assert_eq!(fixture.menu(), removal.after);
            apply_at(&empty, &fixture.scope, true).unwrap();
            assert!(fixture.menu().keys.is_empty());
            assert!(fixture.ledger().entries().is_empty());
            fixture.clear();
            assert_eq!(fixture.ledger(), Ledger::default());
            assert!(fixture.menu().keys.is_empty());
        }
    }
}

#[test]
fn unknown_menu_value_and_changes_since_preview_are_never_overwritten() {
    let fixture = Fixture::new();
    let a = record(Identity::portable());
    let first = preview_at(
        put(
            &Ledger::default(),
            a.clone(),
            None,
            std::slice::from_ref(&a),
        ),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&first, &fixture.scope, true).unwrap();
    let repair = preview_at(
        prepare(
            first.ledger(),
            Change::Activate(a.identity.clone()),
            &Eligible(vec![a.clone()]),
        )
        .unwrap(),
        &fixture.scope,
    )
    .unwrap();
    let tx = Transaction::begin().unwrap();
    let classes = audit::native::classes().unwrap();
    let key = open_at(
        classes.0,
        &relative(&fixture.scope.menus[0]).unwrap(),
        &tx,
        true,
    )
    .unwrap()
    .unwrap();
    success(unsafe {
        RegSetValueExW(
            key.0,
            wide("ForeignValue").as_ptr(),
            0,
            REG_BINARY,
            [42u8].as_ptr(),
            1,
        )
    })
    .unwrap();
    drop(key);
    drop(classes);
    tx.commit().unwrap();
    let foreign = fixture.menu();
    assert!(apply_at(&repair, &fixture.scope, true).is_err());
    assert!(
        preview_at(
            prepare(
                first.ledger(),
                Change::Activate(a.identity.clone()),
                &Eligible(vec![a])
            )
            .unwrap(),
            &fixture.scope
        )
        .is_err()
    );
    assert_eq!(fixture.menu(), foreign);
    assert_eq!(&fixture.ledger(), first.ledger());
    fixture.clear();
    assert!(fixture.menu().keys.is_empty());
    assert_eq!(fixture.ledger(), Ledger::default());
}

#[test]
fn missing_owned_command_can_be_repaired_but_ownership_marker_cannot_be_adopted() {
    let fixture = Fixture::new();
    let a = record(Identity::portable());
    let first = preview_at(
        put(
            &Ledger::default(),
            a.clone(),
            None,
            std::slice::from_ref(&a),
        ),
        &fixture.scope,
    )
    .unwrap();
    apply_at(&first, &fixture.scope, true).unwrap();
    let command = first
        .after
        .values
        .keys()
        .find(|(path, _)| path.ends_with("\\command"))
        .unwrap()
        .clone();
    let tx = Transaction::begin().unwrap();
    let classes = audit::native::classes().unwrap();
    let key = open_at(classes.0, &relative(&command.0).unwrap(), &tx, true)
        .unwrap()
        .unwrap();
    success(unsafe { RegDeleteValueW(key.0, wide(&command.1).as_ptr()) }).unwrap();
    drop(key);
    drop(classes);
    tx.commit().unwrap();
    let repair = preview_at(
        prepare(
            first.ledger(),
            Change::Activate(a.identity.clone()),
            &Eligible(vec![a.clone()]),
        )
        .unwrap(),
        &fixture.scope,
    )
    .unwrap();
    assert_eq!(repair.counts().0, 1);
    apply_at(&repair, &fixture.scope, true).unwrap();
    assert_eq!(fixture.menu(), first.after);
    let tx = Transaction::begin().unwrap();
    let classes = audit::native::classes().unwrap();
    let key = open_at(
        classes.0,
        &relative(&fixture.scope.menus[0]).unwrap(),
        &tx,
        true,
    )
    .unwrap()
    .unwrap();
    success(unsafe { RegDeleteValueW(key.0, wide("ZiCodeOwner").as_ptr()) }).unwrap();
    drop(key);
    drop(classes);
    tx.commit().unwrap();
    let before = fixture.menu();
    assert!(
        preview_at(
            prepare(
                repair.ledger(),
                Change::Activate(a.identity.clone()),
                &Eligible(vec![a])
            )
            .unwrap(),
            &fixture.scope
        )
        .is_err()
    );
    assert_eq!(fixture.menu(), before);
    assert_eq!(&fixture.ledger(), repair.ledger());
    fixture.clear();
    assert!(fixture.menu().keys.is_empty());
    assert_eq!(fixture.ledger(), Ledger::default());
}
