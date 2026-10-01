use super::*;
use ourstick_core::secrets::MemorySecretStore;
use ourstick_core::store::MemoryStore;
use std::sync::Arc;

fn setup(data_dir: &Path) -> setup::Setup {
    setup::Setup {
        key_file_lock: None,
        data_dir: data_dir.to_path_buf(),
        store: Arc::new(MemoryStore::new()),
        secrets: Box::new(MemorySecretStore::new()),
    }
}

#[test]
fn a_folder_gets_one_stick_and_never_the_apps() {
    let dir = tempfile::tempdir().unwrap();
    let setup = setup(dir.path());
    let first = stick_for(&setup, Path::new("/photos"), None).unwrap();
    assert_eq!(
        stick_for(&setup, Path::new("/photos"), None).unwrap(),
        first
    );
    let stick = setup.store.owned_stick(&first).unwrap().unwrap();
    assert_eq!(
        (stick.name.as_str(), stick.kind),
        ("photos", StickKind::Cli)
    );

    device::create_stick(
        setup.store.as_ref(),
        setup.secrets.as_ref(),
        Path::new("/drive"),
        "OurStick",
        StickKind::App,
    )
    .unwrap();
    let refused = stick_for(&setup, Path::new("/drive"), None).unwrap_err();
    assert!(refused.to_string().contains("the app's stick"), "{refused}");
}

#[test]
fn a_stick_is_named_by_name_or_folder_and_renamed_by_a_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let setup = setup(dir.path());
    let id = stick_for(&setup, Path::new("/photos"), Some("For Maria")).unwrap();
    let name = |id| setup.store.owned_stick(&id).unwrap().unwrap().name;
    assert_eq!(name(id), "For Maria");
    // Shared again without a name: kept.
    stick_for(&setup, Path::new("/photos"), None).unwrap();
    assert_eq!(name(id), "For Maria");
    stick_for(&setup, Path::new("/photos"), Some("Family")).unwrap();
    assert_eq!(name(id), "Family");
}

#[test]
fn a_stick_is_not_found_or_made_while_another_program_sets_up() {
    let dir = tempfile::tempdir().unwrap();
    let setup = setup(dir.path());
    let _held = locks::setup_lock(dir.path()).unwrap();
    let refused = stick_for(&setup, Path::new("/photos"), None).unwrap_err();
    assert!(refused.downcast_ref::<locks::LockError>().is_some());
    assert!(setup.store.owned_sticks().unwrap().is_empty());
}
