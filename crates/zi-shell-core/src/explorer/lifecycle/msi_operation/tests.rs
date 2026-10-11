use super::super::tests::{put, sample};
use super::*;

fn record(identity: Identity) -> Installation {
    let mut value = sample(identity);
    value.executable_sha256 = Some("a".repeat(64));
    value
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

    let old = install.plan().result().clone();
    let mut new = record(Identity::Msi);
    new.version = "9.8.7".into();
    let upgrade = super::install(&old, new.clone(), false, &Eligible(vec![new])).unwrap();
    let restored = upgrade
        .undo()
        .rollback(upgrade.plan().result(), &Eligible(vec![first.clone()]))
        .unwrap();
    assert_eq!(restored.result().active(), Some(&first));
    assert_eq!(restored.result().revision(), 3);
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
    let base = install(&base, m.clone(), true, &Eligible(vec![m.clone()]))
        .unwrap()
        .plan
        .after;
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
    let base = install(
        &Ledger::default(),
        m.clone(),
        true,
        &Eligible(vec![m.clone()]),
    )
    .unwrap()
    .plan
    .after;
    let old = remove(&base, Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let next = record(Identity::Msi);
    let replacement = install(
        old.plan().result(),
        next.clone(),
        true,
        &Eligible(vec![next.clone()]),
    )
    .unwrap();
    let removed = remove(
        replacement.plan().result(),
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
    assert_eq!(migrated.result().schema_version, 2);
    assert!(
        migrated.result().msi_cursor.is_some() && migrated.result().activation_cursor.is_some()
    );
    for (field, invalid) in [
        (
            "msi_cursor",
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        ),
        ("schema_version", serde_json::json!(3)),
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
        ("schema_version", serde_json::json!(2)),
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
fn capacity_or_revision_exhaustion_refuses_undo_without_changing_current_records() {
    let m = record(Identity::Msi);
    let base = install(
        &Ledger::default(),
        m.clone(),
        true,
        &Eligible(vec![m.clone()]),
    )
    .unwrap()
    .plan
    .after;
    let op = remove(&base, Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let mut current = op.plan().result().clone();
    for _ in 0..MAX_INSTALLATIONS {
        let p = record(Identity::portable());
        current = portable(&current, &p, true, vec![p.clone()]);
    }
    let snapshot = current.clone();
    assert!(
        op.undo()
            .rollback(&current, &Eligible(vec![m.clone()]))
            .is_err()
    );
    assert_eq!(current, snapshot);
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
