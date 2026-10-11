use super::super::tests::{put, sample};
use super::*;

fn record(identity: Identity) -> Installation {
    let mut value = sample(identity);
    value.executable_sha256 = Some("a".repeat(64));
    value
}
fn completed(op: &Operation) -> Ledger {
    op.undo().complete(op.plan().result()).unwrap().after
}
fn portable(
    before: &Ledger,
    value: &Installation,
    activate: bool,
    eligible: Vec<Installation>,
) -> Ledger {
    prepare(
        before,
        Change::Put {
            installation: value.clone(),
            expected_generation: None,
            activate,
        },
        &Eligible(eligible),
    )
    .unwrap()
    .after
}

#[test]
fn install_upgrade_and_uninstall_undo_restore_only_msi_and_increment_revision() {
    let first = record(Identity::Msi);
    let install = install(
        &Ledger::default(),
        first.clone(),
        true,
        &Eligible(vec![first.clone()]),
    )
    .unwrap();
    let encoded = install.undo().to_bytes().unwrap();
    let restored = Undo::from_bytes(&encoded)
        .unwrap()
        .rollback(install.plan().result(), &Eligible(vec![]))
        .unwrap();
    assert!(restored.result().entries().is_empty());
    assert_eq!(restored.result().revision(), 2);
    assert!(
        install
            .undo()
            .rollback(restored.result(), &Eligible(vec![]))
            .is_err()
    );

    let old = completed(&install);
    let mut new = record(Identity::Msi);
    new.version = "9.8.7".into();
    let upgrade = super::install(&old, new.clone(), false, &Eligible(vec![new])).unwrap();
    let restored = upgrade
        .undo()
        .rollback(upgrade.plan().result(), &Eligible(vec![first.clone()]))
        .unwrap();
    assert_eq!(restored.result().active(), Some(&first));
    assert_eq!(restored.result().revision(), old.revision() + 2);
    assert!(
        upgrade
            .undo()
            .rollback(restored.result(), &Eligible(vec![first.clone()]))
            .is_err()
    );

    assert!(
        remove(&Ledger::default(), None, &Eligible(vec![]))
            .unwrap()
            .is_none()
    );
    assert!(remove(&old, None, &Eligible(vec![])).is_err());
    assert!(
        remove(
            &Ledger::default(),
            Some(&first.generation),
            &Eligible(vec![])
        )
        .is_err()
    );
    let removal = remove(&old, Some(&first.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let restored = removal
        .undo()
        .rollback(removal.plan().result(), &Eligible(vec![first.clone()]))
        .unwrap();
    assert_eq!(restored.result().active(), Some(&first));
}

#[test]
fn undo_preserves_concurrent_portable_add_update_remove_and_explicit_selection() {
    let p = record(Identity::portable());
    let base = portable(&Ledger::default(), &p, true, vec![p.clone()]);
    let m = record(Identity::Msi);
    let op = install(
        &base,
        m.clone(),
        true,
        &Eligible(vec![p.clone(), m.clone()]),
    )
    .unwrap();
    let added = record(Identity::portable());
    let mut current = portable(
        op.plan().result(),
        &added,
        false,
        vec![p.clone(), m.clone()],
    );
    let mut updated = p.clone();
    updated.generation = uuid::Uuid::new_v4().to_string();
    updated.version = "2.0.0".into();
    current = prepare(
        &current,
        Change::Put {
            installation: updated.clone(),
            expected_generation: Some(p.generation.clone()),
            activate: false,
        },
        &Eligible(vec![m.clone(), updated.clone(), added.clone()]),
    )
    .unwrap()
    .after;
    current = prepare(
        &current,
        Change::Activate(added.identity.clone()),
        &Eligible(vec![added.clone()]),
    )
    .unwrap()
    .after;
    current = prepare(
        &current,
        Change::Remove {
            identity: updated.identity.clone(),
            expected_generation: updated.generation.clone(),
        },
        &Eligible(vec![added.clone()]),
    )
    .unwrap()
    .after;
    let expected = current
        .entries()
        .iter()
        .filter(|e| e.installation.identity != Identity::Msi)
        .cloned()
        .collect::<Vec<_>>();
    let restored = op
        .undo()
        .rollback(&current, &Eligible(vec![added.clone()]))
        .unwrap();
    assert_eq!(restored.result().entries(), expected);
    assert_eq!(restored.result().active(), Some(&added));
    assert_eq!(restored.result().revision(), current.revision() + 1);
}

#[test]
fn reselecting_same_fallback_is_user_intent_and_not_overridden_by_uninstall_undo() {
    let p = record(Identity::portable());
    let m = record(Identity::Msi);
    let base = portable(&Ledger::default(), &p, true, vec![p.clone()]);
    let base = completed(&install(&base, m.clone(), true, &Eligible(vec![m.clone()])).unwrap());
    let op = remove(&base, Some(&m.generation), &Eligible(vec![p.clone()]))
        .unwrap()
        .unwrap();
    assert_eq!(op.plan().result().active(), Some(&p));
    let unchanged = op
        .undo()
        .rollback(op.plan().result(), &Eligible(vec![p.clone(), m.clone()]))
        .unwrap();
    assert_eq!(unchanged.result().active(), Some(&m));
    let selected = prepare(
        op.plan().result(),
        Change::Activate(p.identity.clone()),
        &Eligible(vec![p.clone()]),
    )
    .unwrap();
    let restored = op
        .undo()
        .rollback(selected.result(), &Eligible(vec![p.clone(), m]))
        .unwrap();
    assert_eq!(restored.result().active(), Some(&p));
}

#[test]
fn removed_none_state_does_not_authorize_old_undo_after_another_install_remove_cycle() {
    let m = record(Identity::Msi);
    let base = completed(
        &install(
            &Ledger::default(),
            m.clone(),
            true,
            &Eligible(vec![m.clone()]),
        )
        .unwrap(),
    );
    let old = remove(&base, Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let next = record(Identity::Msi);
    let replacement = install(
        &completed(&old),
        next.clone(),
        true,
        &Eligible(vec![next.clone()]),
    )
    .unwrap();
    let removed = remove(
        &completed(&replacement),
        Some(&next.generation),
        &Eligible(vec![]),
    )
    .unwrap()
    .unwrap();
    assert!(msi(old.plan().result()).is_none() && msi(removed.plan().result()).is_none());
    assert!(
        old.undo()
            .rollback(removed.plan().result(), &Eligible(vec![m]))
            .is_err()
    );
    assert!(
        old.undo()
            .rollback(&Ledger::default(), &Eligible(vec![]))
            .is_err()
    );
    assert!(
        replacement
            .undo()
            .rollback(removed.plan().result(), &Eligible(vec![]))
            .is_err()
    );
}

#[test]
fn legacy_schema_reads_and_migrates_but_invalid_cursors_and_protocols_refuse() {
    let m = record(Identity::Msi);
    let model = put(
        &Ledger::default(),
        m.clone(),
        None,
        std::slice::from_ref(&m),
    )
    .after;
    let mut value = serde_json::to_value(&model).unwrap();
    value["schema_version"] = 1.into();
    value.as_object_mut().unwrap().remove("msi_cursor");
    value.as_object_mut().unwrap().remove("activation_cursor");
    let old: Ledger = serde_json::from_value(value.clone()).unwrap();
    old.validate().unwrap();
    let migrated = prepare(&old, Change::Activate(Identity::Msi), &Eligible(vec![m])).unwrap();
    assert_eq!(migrated.result().schema_version, 3);
    assert!(
        migrated.result().msi_cursor.is_some() && migrated.result().activation_cursor.is_some()
    );
    for (field, invalid) in [
        (
            "msi_cursor",
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        ),
        ("schema_version", serde_json::json!(4)),
    ] {
        let mut bad = value.clone();
        bad[field] = invalid;
        assert!(
            serde_json::from_value::<Ledger>(bad)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for cursor in ["", "bad", "00000000-0000-0000-0000-000000000000"] {
        let mut bad = model.clone();
        bad.msi_cursor = Some(cursor.into());
        assert!(bad.validate().is_err());
    }
}

#[test]
fn undo_rejects_unknown_fields_oversize_invalid_id_and_wrong_family() {
    let m = record(Identity::Msi);
    let op = install(&Ledger::default(), m.clone(), true, &Eligible(vec![m])).unwrap();
    let raw = op.undo().to_bytes().unwrap();
    for (field, invalid) in [
        ("unknown", serde_json::json!(true)),
        ("schema_version", serde_json::json!(3)),
        ("operation_id", serde_json::json!("bad")),
        ("applied_msi_cursor", serde_json::json!("")),
        ("applied_revision", serde_json::json!(0)),
    ] {
        let mut bad: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        bad[field] = invalid;
        assert!(Undo::from_bytes(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    assert!(Undo::from_bytes(&vec![b' '; MAX_UNDO_BYTES + 1]).is_err());
    let mut wrong = op.undo().clone();
    wrong.after.as_mut().unwrap().identity = Identity::portable();
    assert!(wrong.to_bytes().is_err());
    assert!(
        install(
            &Ledger::default(),
            record(Identity::portable()),
            true,
            &Eligible(vec![])
        )
        .is_err()
    );
}

#[test]
fn reservation_preserves_full_capacity_for_undo_and_complete_consumes_its_receipt() {
    let m = record(Identity::Msi);
    let base = completed(
        &install(
            &Ledger::default(),
            m.clone(),
            true,
            &Eligible(vec![m.clone()]),
        )
        .unwrap(),
    );
    let op = remove(&base, Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let mut current = op.plan().result().clone();
    for _ in 0..MAX_INSTALLATIONS - 1 {
        let p = record(Identity::portable());
        current = portable(&current, &p, true, vec![p.clone()]);
    }
    let snapshot = current.clone();
    let extra = record(Identity::portable());
    assert!(
        prepare(
            &current,
            Change::Put {
                installation: extra.clone(),
                expected_generation: None,
                activate: false
            },
            &Eligible(vec![])
        )
        .is_err()
    );
    assert_eq!(current, snapshot);
    assert_eq!(
        current.pending_msi_operation(),
        Some(op.undo().operation_id())
    );
    assert!(install(&current, record(Identity::Msi), false, &Eligible(vec![])).is_err());
    assert!(remove(&current, None, &Eligible(vec![])).is_err());
    let restored = op
        .undo()
        .rollback(&current, &Eligible(vec![m.clone()]))
        .unwrap();
    assert_eq!(restored.result().entries().len(), MAX_INSTALLATIONS);
    assert!(restored.result().pending_msi_operation().is_none());
    let finished = op.undo().complete(&current).unwrap();
    assert!(finished.result().pending_msi_operation().is_none());
    assert_eq!(finished.result().active(), current.active());
    assert_eq!(finished.result().entries(), current.entries());
    assert!(
        op.undo()
            .rollback(finished.result(), &Eligible(vec![m.clone()]))
            .is_err()
    );
    assert!(op.undo().complete(finished.result()).is_err());
    let filled = portable(finished.result(), &extra, true, vec![extra.clone()]);
    assert_eq!(filled.entries().len(), MAX_INSTALLATIONS);
    let next = install(&base, record(Identity::Msi), false, &Eligible(vec![])).unwrap();
    let mut exhausted = next.plan().result().clone();
    exhausted.revision = u64::MAX;
    assert!(
        next.undo()
            .rollback(&exhausted, &Eligible(vec![]))
            .err()
            .unwrap()
            .to_string()
            .contains("修订号耗尽")
    );
}

#[test]
fn old_receipts_migrate_without_claiming_a_new_pending_operation() {
    let m = record(Identity::Msi);
    let op = install(
        &Ledger::default(),
        m.clone(),
        true,
        &Eligible(vec![m.clone()]),
    )
    .unwrap();
    let mut value = serde_json::to_value(op.undo()).unwrap();
    value["schema_version"] = 1.into();
    let legacy = Undo::from_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        legacy
            .rollback(op.plan().result(), &Eligible(vec![]))
            .is_err()
    );
    let mut old = op.plan().result().clone();
    old.schema_version = 2;
    old.pending_msi = None;
    old.validate().unwrap();
    assert!(legacy.complete(&old).is_err());
    let restored = legacy.rollback(&old, &Eligible(vec![])).unwrap();
    assert_eq!(restored.result().schema_version, 3);
    assert!(restored.result().entries().is_empty());
    assert!(restored.result().pending_msi_operation().is_none());
    for version in [1, 2] {
        let mut bad = op.plan().result().clone();
        bad.schema_version = version;
        assert!(bad.validate().is_err());
    }
    let mut wrong = op.plan().result().clone();
    wrong.pending_msi = Some(uuid::Uuid::new_v4().to_string());
    wrong.validate().unwrap();
    assert!(op.undo().complete(&wrong).is_err());
    assert!(op.undo().rollback(&wrong, &Eligible(vec![])).is_err());
    for id in ["", "not-uuid", "00000000-0000-0000-0000-000000000000"] {
        let mut bad = op.plan().result().clone();
        bad.pending_msi = Some(id.into());
        assert!(bad.validate().is_err());
    }
    let mut bad = op.plan().result().clone();
    bad.msi_cursor = None;
    assert!(bad.validate().is_err());
    let mut bad = op.plan().result().clone();
    bad.revision = 0;
    assert!(bad.validate().is_err());
}

#[test]
fn old_uninstall_without_reservation_still_refuses_overflow_instead_of_deleting_portables() {
    let m = record(Identity::Msi);
    let base = completed(
        &install(
            &Ledger::default(),
            m.clone(),
            true,
            &Eligible(vec![m.clone()]),
        )
        .unwrap(),
    );
    let op = remove(&base, Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let mut value = serde_json::to_value(op.undo()).unwrap();
    value["schema_version"] = 1.into();
    let old = Undo::from_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
    let mut current = op.plan().result().clone();
    current.schema_version = 2;
    current.pending_msi = None;
    for _ in 0..MAX_INSTALLATIONS {
        let p = record(Identity::portable());
        current = portable(&current, &p, true, vec![p.clone()]);
    }
    let before = current.clone();
    assert!(old.rollback(&current, &Eligible(vec![m])).is_err());
    assert_eq!(current, before);
}
