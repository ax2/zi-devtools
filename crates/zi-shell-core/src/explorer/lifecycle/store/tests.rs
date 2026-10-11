use super::super::tests::{put, sample};
use super::*;

#[test]
fn legacy_ledger_only_api_refuses_empty_pending_msi_completion_before_registry_access() {
    let mut m = super::super::tests::sample(Identity::Msi);
    m.executable_sha256 = Some("a".repeat(64));
    let install = msi_operation::install(
        &Ledger::default(),
        m.clone(),
        true,
        &Eligible(vec![m.clone()]),
    )
    .unwrap();
    let completed = install.undo().complete(install.plan().result()).unwrap();
    let remove = msi_operation::remove(completed.result(), Some(&m.generation), &Eligible(vec![]))
        .unwrap()
        .unwrap();
    let finish = remove.undo().complete(remove.plan().result()).unwrap();
    assert!(finish.before.entries().is_empty() && finish.after.entries().is_empty());
    assert!(
        commit(&finish)
            .err()
            .unwrap()
            .to_string()
            .contains("待恢复MSI操作必须通过联合事务维护")
    );
}

struct Fixture(String);
impl Fixture {
    fn new() -> Self {
        Self(format!(
            r"Software\ZiCode\ZiDevTools\Tests\ExplorerLedger-{}",
            uuid::Uuid::new_v4()
        ))
    }
    fn load(&self) -> Ledger {
        read(&self.0, &Transaction::begin().unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(
            self.0
                .starts_with(r"Software\ZiCode\ZiDevTools\Tests\ExplorerLedger-")
        );
        unsafe {
            RegDeleteTreeW(HKEY_CURRENT_USER, wide(&self.0).as_ptr());
        }
    }
}

#[test]
fn real_registry_transactions_commit_rollback_reject_stale_and_keep_other_install() {
    for msi_first in [false, true] {
        for remove_first in [false, true] {
            let fixture = Fixture::new();
            let portable = sample(Identity::portable());
            let msi = sample(Identity::Msi);
            let (a, b) = if msi_first {
                (&msi, &portable)
            } else {
                (&portable, &msi)
            };
            let one = put(&Ledger::default(), a.clone(), None, std::slice::from_ref(a));
            save(&fixture.0, &one, false).unwrap();
            assert_eq!(
                fixture.load(),
                Ledger::default(),
                "aborted first install must not leave a root"
            );
            save(&fixture.0, &one, true).unwrap();
            assert_eq!(&fixture.load(), one.result());
            let two = put(one.result(), b.clone(), None, &[a.clone(), b.clone()]);
            save(&fixture.0, &two, false).unwrap();
            assert_eq!(
                &fixture.load(),
                one.result(),
                "aborted update must preserve committed state"
            );
            save(&fixture.0, &two, true).unwrap();
            assert!(
                save(&fixture.0, &one, true).is_err(),
                "stale preview must not overwrite latest state"
            );
            let (removed, retained) = if remove_first { (a, b) } else { (b, a) };
            let removal = prepare(
                two.result(),
                Change::Remove {
                    identity: removed.identity.clone(),
                    expected_generation: removed.generation.clone(),
                },
                &Eligible(vec![retained.clone()]),
            )
            .unwrap();
            save(&fixture.0, &removal, true).unwrap();
            assert_eq!(fixture.load().active(), Some(retained));
            let empty = prepare(
                removal.result(),
                Change::Remove {
                    identity: retained.identity.clone(),
                    expected_generation: retained.generation.clone(),
                },
                &Eligible(vec![]),
            )
            .unwrap();
            save(&fixture.0, &empty, true).unwrap();
            assert!(fixture.load().entries().is_empty());
            assert_eq!(fixture.load().revision(), 4);
            assert!(
                save(&fixture.0, &one, true).is_err(),
                "empty ledger must not permit old first-install replay"
            );
            assert_eq!(&fixture.load(), empty.result());
            unsafe {
                success(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&fixture.0).as_ptr())).unwrap();
            }
            assert!(
                open(&fixture.0, &Transaction::begin().unwrap(), false)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn concurrent_previews_have_one_winner_and_unknown_registry_content_is_preserved() {
    let fixture = Fixture::new();
    let initial = sample(Identity::Msi);
    let first = put(
        &Ledger::default(),
        initial.clone(),
        None,
        std::slice::from_ref(&initial),
    );
    save(&fixture.0, &first, true).unwrap();
    let a = sample(Identity::Msi);
    let b = sample(Identity::Msi);
    let proposals = [
        put(
            first.result(),
            a.clone(),
            Some(initial.generation.clone()),
            std::slice::from_ref(&a),
        ),
        put(
            first.result(),
            b.clone(),
            Some(initial.generation),
            std::slice::from_ref(&b),
        ),
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = proposals
        .into_iter()
        .map(|proposal| {
            let root = fixture.0.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                save(&root, &proposal, true).is_ok()
            })
        })
        .collect();
    assert_eq!(
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .filter(|won| *won)
            .count(),
        1
    );
    let committed = fixture.load();
    assert_eq!(committed.revision(), 2);
    assert!([a, b].contains(committed.active().unwrap()));
    let tx = Transaction::begin().unwrap();
    let key = open(&fixture.0, &tx, true).unwrap().unwrap();
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
    tx.commit().unwrap();
    assert!(read(&fixture.0, &Transaction::begin().unwrap()).is_err());
    assert!(save(&fixture.0, &first, true).is_err());
    // Exact value still exists after the rejected update (no cleanup by engine).
    let tx = Transaction::begin().unwrap();
    let key = open(&fixture.0, &tx, false).unwrap().unwrap();
    let mut byte = 0;
    let mut size = 1;
    let mut kind = 0;
    success(unsafe {
        RegQueryValueExW(
            key.0,
            wide("ForeignValue").as_ptr(),
            ptr::null(),
            &mut kind,
            &mut byte,
            &mut size,
        )
    })
    .unwrap();
    assert_eq!((kind, size, byte), (REG_BINARY, 1, 42));
    drop(key);
    drop(tx);
    unsafe {
        success(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&fixture.0).as_ptr())).unwrap();
    }
    assert!(
        open(&fixture.0, &Transaction::begin().unwrap(), false)
            .unwrap()
            .is_none()
    );
}
