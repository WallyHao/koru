//! Atomic replacement and scoped lock behavior.
use koru::persist::{FileLock, write_atomic};
use std::{fs, path::PathBuf, thread, time::Duration};

fn lock_path(target: &std::path::Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

#[test]
fn write_atomic_creates_parents_and_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/file.txt");
    write_atomic(&path, b"one").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"one");
    write_atomic(&path, b"two").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"two");
    let files = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1, "unexpected files: {files:?}");
}

#[test]
fn lock_waits_for_the_holder_and_is_released_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("config.toml");
    let lock = FileLock::acquire(&target).unwrap();
    assert!(lock_path(&target).exists());
    let waiter_target = target.clone();
    let waiter = thread::spawn(move || FileLock::acquire(&waiter_target).map(drop));
    thread::sleep(Duration::from_millis(50));
    drop(lock);
    waiter.join().unwrap().unwrap();
    assert!(!lock_path(&target).exists());
}

#[test]
fn lock_can_be_reacquired_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("config.toml");
    let first = FileLock::acquire(&target).unwrap();
    drop(first);
    let second = FileLock::acquire(&target).unwrap();
    drop(second);
    assert!(!lock_path(&target).exists());
}
