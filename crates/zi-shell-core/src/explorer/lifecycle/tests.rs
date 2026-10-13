use super::*;

#[test]
fn capability_snapshots_roundtrip_filter_and_survive_fallback_and_undo() {
    let mut portable = sample(Identity::portable());
    portable.executable_sha256 = Some("a".repeat(64));
    portable.menu_capabilities =
        Some("ZiDevTools/ShellCapabilities/1\nchoose-files\nimport:csv\n".into());
    let first = put(
        &Ledger::default(),
        portable.clone(),
        None,
        &[portable.clone()],
    );
    let roundtrip: Ledger =
        serde_json::from_slice(&serde_json::to_vec(first.result()).unwrap()).unwrap();
    roundtrip.validate().unwrap();
    assert_eq!(
        roundtrip.active().unwrap().menu_capabilities,
        portable.menu_capabilities
    );
    assert_eq!(
        menu_plan(&portable)
            .unwrap()
            .values
            .iter()
            .filter(|v| v.key.ends_with("\\command"))
            .count(),
        2
    );
    let mut msi = sample(Identity::Msi);
    msi.executable_sha256 = Some("b".repeat(64));
    msi.menu_capabilities =
        Some(String::from_utf8(crate::capability_manifest::Manifest::current().encode()).unwrap());
    let install = msi_operation::install(
        &roundtrip,
        msi.clone(),
        true,
        &Eligible(vec![msi.clone(), portable.clone()]),
    )
    .unwrap();
    let undo = msi_operation::Undo::from_bytes(&install.undo().to_bytes().unwrap()).unwrap();
    let restored = undo
        .rollback(install.plan().result(), &Eligible(vec![portable.clone()]))
        .unwrap();
    assert_eq!(restored.result().active(), Some(&portable));
    assert_eq!(
        menu_plan(restored.result().active().unwrap())
            .unwrap()
            .values
            .iter()
            .filter(|v| v.key.ends_with("\\command"))
            .count(),
        2
    );
    let mut old_protocol = serde_json::to_value(&roundtrip).unwrap();
    old_protocol["schema_version"] = 3.into();
    assert!(
        serde_json::from_value::<Ledger>(old_protocol)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut invalid = portable.clone();
    invalid.menu_capabilities = Some("unknown".into());
    assert!(invalid.validate().is_err());
    invalid.menu_capabilities = portable.menu_capabilities.clone();
    invalid.executable_sha256 = None;
    assert!(invalid.validate().is_err());
}

#[test]
fn legacy_layout_is_frozen_and_new_protocol_migration_is_explicit() {
    let mut legacy = sample(Identity::portable());
    legacy.executable_sha256 = Some("a".repeat(64));
    let current = put(&Ledger::default(), legacy.clone(), None, &[legacy.clone()]);
    let mut value = serde_json::to_value(current.result()).unwrap();
    value["schema_version"] = 3.into();
    let old: Ledger = serde_json::from_value(value).unwrap();
    old.validate().unwrap();
    assert_eq!(legacy.menu_declaration().unwrap().targets().len(), 11);
    let migrated = prepare(
        &old,
        Change::Activate(legacy.identity.clone()),
        &Eligible(vec![legacy]),
    )
    .unwrap();
    assert_eq!(migrated.result().schema_version, 4);
    assert!(
        migrated
            .result()
            .active()
            .unwrap()
            .menu_capabilities
            .is_none()
    );
}

pub(super) fn sample(identity: Identity) -> Installation {
    let root = std::env::current_dir().unwrap().join("安装资料");
    Installation::new(
        identity,
        root.join("ZiDevTools.exe"),
        root.join("服务.yml"),
        env!("CARGO_PKG_VERSION").into(),
    )
}
pub(super) fn put(
    before: &Ledger,
    entry: Installation,
    old: Option<String>,
    eligible: &[Installation],
) -> Prepared {
    prepare(
        before,
        Change::Put {
            installation: entry,
            expected_generation: old,
            activate: true,
        },
        &Eligible(eligible.to_vec()),
    )
    .unwrap()
}

#[test]
fn two_install_orders_and_uninstall_orders_preserve_the_other_owner() {
    for msi_first in [false, true] {
        for remove_first_installed in [false, true] {
            let portable = sample(Identity::portable());
            let msi = sample(Identity::Msi);
            let (first, second) = if msi_first {
                (&msi, &portable)
            } else {
                (&portable, &msi)
            };
            let both = [first.clone(), second.clone()];
            let one = put(&Ledger::default(), first.clone(), None, &both);
            let two = put(one.result(), second.clone(), None, &both);
            assert_eq!(two.result().active(), Some(second));
            let (removed, retained) = if remove_first_installed {
                (first, second)
            } else {
                (second, first)
            };
            let removal = prepare(
                two.result(),
                Change::Remove {
                    identity: removed.identity.clone(),
                    expected_generation: removed.generation.clone(),
                },
                &Eligible(vec![retained.clone()]),
            )
            .unwrap();
            assert_eq!(removal.result().active(), Some(retained));
            assert_eq!(removal.result().entries().len(), 1);
            let empty = prepare(
                removal.result(),
                Change::Remove {
                    identity: retained.identity.clone(),
                    expected_generation: retained.generation.clone(),
                },
                &Eligible(vec![]),
            )
            .unwrap();
            assert!(empty.result().active().is_none());
            assert!(empty.result().entries().is_empty());
            assert_eq!(empty.result().revision(), 4);
        }
    }
}

#[test]
fn upgrades_keep_identity_but_stale_uninstall_and_preview_cannot_remove_them() {
    let first = sample(Identity::Msi);
    let baseline = put(
        &Ledger::default(),
        first.clone(),
        None,
        std::slice::from_ref(&first),
    );
    let stale_remove = prepare(
        baseline.result(),
        Change::Remove {
            identity: first.identity.clone(),
            expected_generation: first.generation.clone(),
        },
        &Eligible(vec![]),
    )
    .unwrap();
    let mut upgrade = sample(Identity::Msi);
    upgrade.executable = upgrade.executable.with_file_name("新版.exe");
    // Development versions need not change when installation content changes.
    assert_eq!(upgrade.version, first.version);
    let update = put(
        baseline.result(),
        upgrade.clone(),
        Some(first.generation.clone()),
        std::slice::from_ref(&upgrade),
    );
    assert_eq!(update.result().entries().len(), 1);
    assert_eq!(update.result().active(), Some(&upgrade));
    assert!(stale_remove.require_current(update.result()).is_err());
    assert!(
        prepare(
            update.result(),
            Change::Remove {
                identity: first.identity.clone(),
                expected_generation: first.generation.clone()
            },
            &Eligible(vec![])
        )
        .is_err()
    );
    assert!(
        prepare(
            update.result(),
            Change::Activate(Identity::Msi),
            &Eligible(vec![first])
        )
        .is_err()
    );
    assert!(
        prepare(
            update.result(),
            Change::Put {
                installation: upgrade.clone(),
                expected_generation: Some(upgrade.generation.clone()),
                activate: true
            },
            &Eligible(vec![upgrade])
        )
        .is_err()
    );
}

#[test]
fn fallback_never_selects_unverified_or_old_generation_paths() {
    let a = sample(Identity::portable());
    let b = sample(Identity::portable());
    assert!(
        prepare(
            &Ledger::default(),
            Change::Put {
                installation: a.clone(),
                expected_generation: None,
                activate: true
            },
            &Eligible(vec![])
        )
        .is_err()
    );
    let one = put(
        &Ledger::default(),
        a.clone(),
        None,
        std::slice::from_ref(&a),
    );
    let two = put(one.result(), b.clone(), None, &[a.clone(), b.clone()]);
    let removed = prepare(
        two.result(),
        Change::Remove {
            identity: b.identity,
            expected_generation: b.generation,
        },
        &Eligible(vec![]),
    )
    .unwrap();
    assert_eq!(removed.result().entries().len(), 1);
    assert!(removed.result().active().is_none());
}

#[test]
fn inactive_install_move_and_explicit_selection_preserve_independent_ownership() {
    let msi = sample(Identity::Msi);
    let portable = sample(Identity::portable());
    let one = put(
        &Ledger::default(),
        msi.clone(),
        None,
        std::slice::from_ref(&msi),
    );
    let two = prepare(
        one.result(),
        Change::Put {
            installation: portable.clone(),
            expected_generation: None,
            activate: false,
        },
        &Eligible(vec![msi.clone(), portable.clone()]),
    )
    .unwrap();
    assert_eq!(two.result().active(), Some(&msi));
    assert_eq!(two.result().entries().len(), 2);
    // Moving a portable install preserves its UUID, rotates its generation,
    // and does not take the active alias from the MSI without explicit choice.
    let mut moved = sample(portable.identity.clone());
    moved.executable = moved.executable.with_file_name("移动后的程序.exe");
    let updated = prepare(
        two.result(),
        Change::Put {
            installation: moved.clone(),
            expected_generation: Some(portable.generation.clone()),
            activate: false,
        },
        &Eligible(vec![msi.clone(), moved.clone()]),
    )
    .unwrap();
    assert_eq!(updated.result().active(), Some(&msi));
    assert!(
        updated
            .result()
            .entries()
            .iter()
            .any(|entry| entry.installation == msi)
    );
    let selected = prepare(
        updated.result(),
        Change::Activate(moved.identity.clone()),
        &Eligible(vec![msi, moved.clone()]),
    )
    .unwrap();
    assert_eq!(selected.result().active(), Some(&moved));
    assert!(
        prepare(
            selected.result(),
            Change::Remove {
                identity: portable.identity,
                expected_generation: portable.generation
            },
            &Eligible(vec![moved])
        )
        .is_err()
    );
}

#[test]
fn strict_ledger_rejects_foreign_schema_duplicates_bad_paths_and_overflow() {
    let entry = sample(Identity::portable());
    let prepared = put(
        &Ledger::default(),
        entry.clone(),
        None,
        std::slice::from_ref(&entry),
    );
    let json = serde_json::to_value(prepared.result()).unwrap();
    for (field, value) in [
        ("schema_version", serde_json::json!(5)),
        ("owner", serde_json::json!("foreign")),
        ("revision", serde_json::json!(0)),
    ] {
        let mut corrupt = json.clone();
        corrupt[field] = value;
        assert!(
            serde_json::from_value::<Ledger>(corrupt)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut extra = json.clone();
    extra["unknown"] = true.into();
    assert!(serde_json::from_value::<Ledger>(extra).is_err());
    let mut duplicate = prepared.result().clone();
    duplicate.entries.push(duplicate.entries[0].clone());
    assert!(duplicate.validate().is_err());
    let mut relative = entry.clone();
    relative.executable = "relative.exe".into();
    assert!(relative.validate().is_err());
    let mut invalid = entry.clone();
    invalid.config = invalid.config.with_file_name("bad%1.yml");
    assert!(invalid.validate().is_err());
    let mut unicode = entry.clone();
    unicode.executable = (0..150)
        .fold(std::env::current_dir().unwrap(), |path, _| {
            path.join("文".repeat(80))
        })
        .join("ZiDevTools.exe");
    assert!(unicode.executable.to_str().unwrap().len() > 32767);
    assert!(
        unicode.validate().is_ok(),
        "Windows path limits count UTF-16 units, not UTF-8 bytes; existence is checked separately by the caller"
    );
    unicode.executable = std::env::current_dir().unwrap().join("文".repeat(33000));
    assert!(unicode.validate().is_err());
    let mut overflow = prepared.result().clone();
    overflow.revision = u64::MAX;
    assert!(
        prepare(
            &overflow,
            Change::Activate(entry.identity.clone()),
            &Eligible(vec![entry])
        )
        .is_err()
    );
    let mut ledger = Ledger::default();
    for _ in 0..MAX_INSTALLATIONS {
        let item = sample(Identity::portable());
        ledger = put(&ledger, item.clone(), None, std::slice::from_ref(&item)).after;
    }
    let item = sample(Identity::portable());
    assert!(
        prepare(
            &ledger,
            Change::Put {
                installation: item.clone(),
                expected_generation: None,
                activate: true
            },
            &Eligible(vec![item])
        )
        .is_err()
    );
}
