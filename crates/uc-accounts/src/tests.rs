use super::*;

fn store() -> (AccountStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (AccountStore::new(dir.path().join("accounts")), dir)
}

fn secret(token: &str) -> Value {
    serde_json::json!({"tokens": {"access_token": token, "refresh_token": "test-refresh-secret"}})
}

#[test]
fn default_label_update_changes_only_label_and_update_time() {
    let (store, _dir) = store();
    let record = store
        .import(
            "claude",
            "claude",
            "fixture",
            &secret("fixture"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let mut registry = store.registry().unwrap();
    registry
        .accounts
        .get_mut(&record.id)
        .unwrap()
        .record
        .updated_at = Utc::now() - chrono::Duration::days(1);
    store.save_registry(&registry).unwrap();
    let before = &registry.accounts[&record.id];
    let secret_path = store.secret_path(&record.id, before.credential_revision);
    let bytes = std::fs::read(&secret_path).unwrap();
    let modified = std::fs::metadata(&secret_path).unwrap().modified().unwrap();

    assert!(
        store
            .set_label_if_default(&before.record, " fixture@example.test ")
            .unwrap()
    );

    let updated = store.registry().unwrap();
    let after = &updated.accounts[&record.id];
    assert_eq!(after.record.label, "fixture@example.test");
    assert!(after.record.updated_at > before.record.updated_at);
    let mut expected = serde_json::to_value(&registry).unwrap();
    expected["accounts"][&record.id]["label"] = serde_json::json!(after.record.label);
    expected["accounts"][&record.id]["updatedAt"] = serde_json::json!(after.record.updated_at);
    assert_eq!(serde_json::to_value(&updated).unwrap(), expected);
    assert_eq!(std::fs::read(&secret_path).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&secret_path).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(
        std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn label_update_preserves_custom_shared_invalid_and_noop_records() {
    let (store, _dir) = store();
    let original = store
        .import(
            "codex",
            "codex",
            "managed",
            &secret("fixture"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let concurrent = AccountStore::new(store.root.clone());
    let custom = concurrent
        .import(
            "codex",
            "Custom label",
            "managed",
            &secret("replacement"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let shared = store
        .import(
            "claude",
            "claude",
            "shared",
            &secret("fixture"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let default = store
        .import(
            "codex",
            "codex",
            "default",
            &secret("fixture"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let registry_path = store.root.join("registry.json");
    let before = std::fs::read(&registry_path).unwrap();

    assert!(
        !store
            .set_label_if_default(&original, "fixture@example.test")
            .unwrap()
    );
    assert!(
        !store
            .set_label_if_default(&custom, "fixture@example.test")
            .unwrap()
    );
    assert!(
        !store
            .set_label_if_default(&shared, "fixture@example.test")
            .unwrap()
    );
    assert!(!store.set_label_if_default(&default, " codex ").unwrap());
    for invalid in [
        "".to_owned(),
        " ".into(),
        "line\nbreak".into(),
        "a".repeat(257),
    ] {
        assert!(matches!(
            store.set_label_if_default(&default, &invalid),
            Err(AccountError::InvalidAccount)
        ));
    }
    let mut missing = default.clone();
    missing.id = "codex@missing".into();
    assert!(matches!(
        store.set_label_if_default(&missing, "fixture@example.test"),
        Err(AccountError::NotFound)
    ));
    assert_eq!(std::fs::read(registry_path).unwrap(), before);
}

#[test]
fn parallel_label_updates_change_a_default_only_once() {
    let (store, _dir) = store();
    let record = store
        .import(
            "codex",
            "codex",
            "fixture",
            &secret("fixture"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let barrier = std::sync::Barrier::new(8);
    let changed = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let store = AccountStore::new(store.root.clone());
                let barrier = &barrier;
                let expected = &record;
                scope.spawn(move || {
                    barrier.wait();
                    store
                        .set_label_if_default(expected, &format!("fixture-{index}@example.test"))
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|changed| *changed)
            .count()
    });
    assert_eq!(changed, 1);
}

#[test]
fn label_update_skips_a_snapshot_from_before_credentials_changed() {
    let (store, _dir) = store();
    let original = store
        .import(
            "codex",
            "codex",
            "fixture",
            &secret("old"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let mut registry = store.registry().unwrap();
    registry
        .accounts
        .get_mut(&original.id)
        .unwrap()
        .record
        .updated_at = Utc::now() - chrono::Duration::days(1);
    store.save_registry(&registry).unwrap();
    let stale = store.list().unwrap().pop().unwrap();
    store
        .update_credentials(&original.id, &secret("renewed"))
        .unwrap();
    let registry_path = store.root.join("registry.json");
    let before = std::fs::read(&registry_path).unwrap();
    assert!(
        !store
            .set_label_if_default(&stale, "fixture@example.test")
            .unwrap()
    );
    assert_eq!(std::fs::read(registry_path).unwrap(), before);
    assert_eq!(store.credentials(&original.id).unwrap(), secret("renewed"));
    let current = store.list().unwrap().pop().unwrap();
    assert!(
        store
            .set_label_if_default(&current, "fixture@example.test")
            .unwrap()
    );
}

#[test]
fn round_trip_survives_reopening_store() {
    let (store, _dir) = store();
    assert!(store.list().unwrap().is_empty());
    let record = store
        .import(
            "codex",
            "Personal",
            "stable-user-id",
            &secret("token-one"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let reopened = AccountStore::new(store.root.clone());
    assert_eq!(reopened.list().unwrap(), vec![record.clone()]);
    assert_eq!(
        reopened.credentials(&record.id).unwrap(),
        secret("token-one")
    );
    assert!(record.id.starts_with("codex@"));
    assert_eq!(record.id.len(), "codex@".len() + 64);
}

#[test]
fn duplicate_identity_updates_without_creating_another_account() {
    let (store, _dir) = store();
    let first = store
        .import(
            "codex",
            "Personal",
            "stable-user-id",
            &secret("old-token"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let second = store
        .import(
            "codex",
            "Personal renamed",
            "stable-user-id",
            &secret("new-token"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.connected_at, first.connected_at);
    assert_eq!(second.label, "Personal renamed");
    assert_eq!(second.credential_mode, CredentialMode::ManagedOauth);
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.credentials(&first.id).unwrap(), secret("new-token"));
    assert_eq!(
        std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn shared_cli_import_never_replaces_managed_tokens() {
    let (store, _dir) = store();
    let managed = store
        .import(
            "codex",
            "Managed",
            "stable-user-id",
            &secret("owned-token"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let imported = store
        .import(
            "codex",
            "Updated label",
            "stable-user-id",
            &secret("cli-token"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    assert_eq!(imported.id, managed.id);
    assert_eq!(imported.credential_mode, CredentialMode::ManagedOauth);
    assert_eq!(
        store.credentials(&managed.id).unwrap(),
        secret("owned-token")
    );
}

#[test]
fn simultaneous_accounts_remain_separate_and_listed_without_credentials() {
    let (store, _dir) = store();
    let personal = store
        .import(
            "codex",
            "Personal",
            "personal",
            &secret("personal-token"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let work = store
        .import(
            "codex",
            "Work",
            "work",
            &secret("work-token"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let claude = store
        .import(
            "claude",
            "Claude",
            "personal",
            &secret("claude-token"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    assert_ne!(personal.id, work.id);
    assert_ne!(personal.id, claude.id);
    let registry = store.registry().unwrap();
    std::fs::remove_file(
        store.secret_path(&work.id, registry.accounts[&work.id].credential_revision),
    )
    .unwrap();
    std::fs::write(
        store.secret_path(
            &claude.id,
            registry.accounts[&claude.id].credential_revision,
        ),
        b"damaged",
    )
    .unwrap();
    assert_eq!(store.list().unwrap().len(), 3);
    assert_eq!(
        store.credentials(&personal.id).unwrap(),
        secret("personal-token")
    );
    assert!(matches!(
        store.credentials(&work.id),
        Err(AccountError::CredentialsUnavailable)
    ));
    assert!(matches!(
        store.credentials(&claude.id),
        Err(AccountError::CredentialsUnavailable)
    ));
    assert_eq!(store.list().unwrap().len(), 3);
}

#[test]
fn metadata_never_contains_credentials_or_raw_identity() {
    let (store, _dir) = store();
    let record = store
        .import(
            "codex",
            "Personal",
            "private-account-identity",
            &secret("secret-access-value"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let wire = serde_json::to_string(&record).unwrap();
    let disk = std::fs::read_to_string(store.root.join("registry.json")).unwrap();
    for text in [&wire, &disk] {
        for forbidden in [
            "private-account-identity",
            "secret-access-value",
            "access_token",
            "refresh_token",
            "test-refresh-secret",
        ] {
            assert!(!text.contains(forbidden));
        }
    }
    let keys = serde_json::to_value(record)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "connectedAt",
            "credentialMode",
            "id",
            "label",
            "provider",
            "updatedAt"
        ]
    );
}

#[test]
fn malformed_registry_is_preserved_when_import_is_attempted() {
    let (store, _dir) = store();
    store.list().unwrap();
    let invalid = b"{not valid JSON";
    std::fs::write(store.root.join("registry.json"), invalid).unwrap();
    assert!(matches!(store.list(), Err(AccountError::InvalidRegistry)));
    let result = store.import(
        "codex",
        "Personal",
        "identity",
        &secret("value"),
        CredentialMode::SharedCli,
    );
    assert!(matches!(result, Err(AccountError::InvalidRegistry)));
    assert_eq!(
        std::fs::read(store.root.join("registry.json")).unwrap(),
        invalid
    );
    assert_eq!(
        std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn malformed_account_path_is_not_followed() {
    let (store, _dir) = store();
    let record = store
        .import(
            "codex",
            "Personal",
            "identity",
            &secret("value"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let mut registry: Value =
        serde_json::from_slice(&std::fs::read(store.root.join("registry.json")).unwrap()).unwrap();
    let mut entry = registry["accounts"]
        .as_object_mut()
        .unwrap()
        .remove(&record.id)
        .unwrap();
    entry["id"] = "../other".into();
    registry["accounts"]["../other"] = entry;
    std::fs::write(
        store.root.join("registry.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
    assert!(matches!(store.list(), Err(AccountError::InvalidRegistry)));
    assert!(matches!(
        store.credentials("../other"),
        Err(AccountError::InvalidRegistry)
    ));
}

#[test]
fn refresh_updates_only_the_selected_account_and_remove_is_explicit() {
    let (store, _dir) = store();
    let first = store
        .import(
            "codex",
            "First",
            "first",
            &secret("first-token"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let second = store
        .import(
            "codex",
            "Second",
            "second",
            &secret("second-token"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    store
        .update_credentials(&first.id, &secret("refreshed"))
        .unwrap();
    assert_eq!(store.credentials(&first.id).unwrap(), secret("refreshed"));
    assert_eq!(
        store.credentials(&second.id).unwrap(),
        secret("second-token")
    );
    store.remove(&first.id).unwrap();
    assert_eq!(store.list().unwrap(), vec![second]);
    assert!(matches!(
        store.credentials(&first.id),
        Err(AccountError::NotFound)
    ));
    assert_eq!(
        std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn parallel_store_instances_preserve_every_account() {
    let (store, _dir) = store();
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for index in 0..8 {
            let root = store.root.clone();
            let barrier = &barrier;
            scope.spawn(move || {
                let store = AccountStore::new(root);
                barrier.wait();
                store
                    .import(
                        "codex",
                        &format!("Account {index}"),
                        &format!("user-{index}"),
                        &secret("value"),
                        CredentialMode::SharedCli,
                    )
                    .unwrap();
            });
        }
    });
    assert_eq!(store.list().unwrap().len(), 8);
}

#[test]
fn child_process_import() {
    let Ok(root) = std::env::var("UC_ACCOUNTS_TEST_ROOT") else {
        return;
    };
    let identity = std::env::var("UC_ACCOUNTS_TEST_IDENTITY").unwrap();
    let store = AccountStore::new(root.into());
    store
        .import(
            "codex",
            &identity,
            &identity,
            &secret("child-value"),
            CredentialMode::SharedCli,
        )
        .unwrap();
}

#[test]
fn parallel_processes_preserve_every_account() {
    let (store, _dir) = store();
    let executable = std::env::current_exe().unwrap();
    let mut children: Vec<_> = (0..4)
        .map(|index| {
            std::process::Command::new(&executable)
                .args(["--exact", "tests::child_process_import", "--nocapture"])
                .env("UC_ACCOUNTS_TEST_ROOT", &store.root)
                .env("UC_ACCOUNTS_TEST_IDENTITY", format!("child-{index}"))
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(store.list().unwrap().len(), 4);
}

#[cfg(windows)]
#[test]
fn windows_secrets_are_encrypted_and_bound_to_the_account() {
    let (store, _dir) = store();
    let record = store
        .import(
            "codex",
            "Personal",
            "private-id",
            &secret("secret-access-value"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let registry = store.registry().unwrap();
    let bytes = std::fs::read(store.secret_path(
        &record.id,
        registry.accounts[&record.id].credential_revision,
    ))
    .unwrap();
    assert!(serde_json::from_slice::<Value>(&bytes).is_err());
    assert!(!String::from_utf8_lossy(&bytes).contains("secret-access-value"));
    assert!(protection::unprotect(&bytes, b"another-account").is_err());
}

#[cfg(windows)]
#[test]
fn failed_registry_commit_preserves_previous_credentials() {
    let (store, _dir) = store();
    let record = store
        .import(
            "codex",
            "Personal",
            "identity",
            &secret("old-token"),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let registry_path = store.root.join("registry.json");
    let original = std::fs::read(&registry_path).unwrap();
    let original_permissions = std::fs::metadata(&registry_path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&registry_path, readonly).unwrap();
    let result = store.update_credentials(&record.id, &secret("new-token"));
    std::fs::set_permissions(&registry_path, original_permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&registry_path).unwrap(), original);
    assert_eq!(store.credentials(&record.id).unwrap(), secret("old-token"));
    assert_eq!(
        std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn unix_storage_is_owner_only_and_rejects_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (store, dir) = store();
    let record = store
        .import(
            "codex",
            "Personal",
            "id",
            &secret("value"),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let registry = store.registry().unwrap();
    for directory in [&store.root, &store.root.join("credentials")] {
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for file in [
        store.root.join("registry.json"),
        store.root.join("registry.lock"),
        store.secret_path(
            &record.id,
            registry.accounts[&record.id].credential_revision,
        ),
    ] {
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let link = dir.path().join("linked-accounts");
    symlink(&store.root, &link).unwrap();
    assert!(matches!(
        AccountStore::new(link).list(),
        Err(AccountError::UnsafePath)
    ));
    std::fs::remove_file(store.root.join("registry.json")).unwrap();
    symlink(
        dir.path().join("external.json"),
        store.root.join("registry.json"),
    )
    .unwrap();
    assert!(matches!(store.list(), Err(AccountError::UnsafePath)));
}

#[test]
fn renewal_lock_holds_off_a_second_holder_until_released() {
    let (store, _dir) = store();
    let first = store.renewal_lock("codex@abc").unwrap();
    let (acquired, receiver) = std::sync::mpsc::channel();
    let waiter = {
        let store = store.clone();
        std::thread::spawn(move || {
            let _second = store.renewal_lock("codex@abc").unwrap();
            acquired.send(()).unwrap();
        })
    };
    assert!(
        receiver
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err()
    );
    let _other_account = store.renewal_lock("claude@abc").unwrap();
    drop(first);
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    waiter.join().unwrap();
    for invalid in ["", "../registry", "codex@abc/../x", "codex@abc\\x"] {
        assert!(matches!(
            store.renewal_lock(invalid),
            Err(AccountError::InvalidAccount)
        ));
    }
}
