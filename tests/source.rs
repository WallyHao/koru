//! Filesystem boundary fixtures: no script evaluation occurs during capture.
use koru::{
    error::ErrorCode,
    source::{SourceBundle, SourceLimits, discover},
};
use std::fs;
use tempfile::TempDir;

fn fixture(script: &[u8]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("demo.lua"), script).unwrap();
    dir
}
fn capture(dir: &TempDir) -> koru::error::Result<SourceBundle> {
    SourceBundle::capture(dir.path(), "demo", SourceLimits::default())
}

#[test]
fn discovery_reads_names_without_evaluating_or_parsing_lua() {
    let dir = fixture(b"this is not Lua; os.execute('never')");
    fs::write(dir.path().join("lib/shared.lua"), b"return {}").unwrap();
    assert_eq!(discover(dir.path()).unwrap(), ["demo"]);
}
#[test]
fn discovery_rejects_reserved_names() {
    let dir = fixture(b"");
    fs::write(dir.path().join("check.lua"), b"").unwrap();
    assert_eq!(
        discover(dir.path()).unwrap_err().code(),
        ErrorCode::Validation
    );
}
#[test]
fn captures_transitive_modules_and_freezes_source() {
    let dir = fixture(b"-- koru-module: a\nreturn require('a')");
    fs::write(
        dir.path().join("lib/a.lua"),
        b"-- koru-module: b\nreturn require('b')",
    )
    .unwrap();
    fs::write(dir.path().join("lib/b.lua"), b"return 1").unwrap();
    let bundle = capture(&dir).unwrap();
    assert_eq!(bundle.modules().len(), 2);
    let digest = bundle.digest();
    fs::write(dir.path().join("lib/b.lua"), b"return 2").unwrap();
    assert_eq!(bundle.module("b").unwrap().text(), "return 1");
    assert_ne!(capture(&dir).unwrap().digest(), digest);
    assert_eq!(bundle.entry().dependencies(), ["a"]);
}
#[test]
fn digest_is_deterministic_and_independent_of_install_directory() {
    let first = fixture(b"return {}");
    let second = fixture(b"return {}");
    assert_eq!(
        capture(&first).unwrap().digest(),
        capture(&second).unwrap().digest()
    );
}
#[test]
fn rejects_cycles_missing_modules_and_traversal() {
    let dir = fixture(b"-- koru-module: a\nreturn {}");
    assert!(capture(&dir).is_err());
    fs::write(
        dir.path().join("lib/a.lua"),
        b"-- koru-module: a\nreturn {}",
    )
    .unwrap();
    assert!(capture(&dir).is_err());
    fs::write(
        dir.path().join("demo.lua"),
        b"-- koru-module: ../escape\nreturn {}",
    )
    .unwrap();
    assert!(capture(&dir).is_err());
    assert!(SourceBundle::capture(dir.path(), "../demo", SourceLimits::default()).is_err());
}
#[test]
fn rejects_bytecode_and_invalid_utf8() {
    for source in [
        &b"\x1bLua"[..],
        &b"\xff"[..],
        &b"-- koru-module: \nreturn {}"[..],
    ] {
        assert!(capture(&fixture(source)).is_err());
    }
}
#[test]
fn enforces_per_file_total_depth_and_count_limits() {
    let dir = fixture(b"-- koru-module: a\nreturn {}");
    fs::write(
        dir.path().join("lib/a.lua"),
        b"-- koru-module: b\nreturn {}",
    )
    .unwrap();
    fs::write(dir.path().join("lib/b.lua"), b"return {}").unwrap();
    for limits in [
        SourceLimits {
            max_file_bytes: 8,
            ..SourceLimits::default()
        },
        SourceLimits {
            max_total_bytes: 30,
            ..SourceLimits::default()
        },
        SourceLimits {
            max_modules: 1,
            ..SourceLimits::default()
        },
        SourceLimits {
            max_depth: 1,
            ..SourceLimits::default()
        },
    ] {
        assert_eq!(
            SourceBundle::capture(dir.path(), "demo", limits)
                .unwrap_err()
                .code(),
            ErrorCode::BudgetExhausted
        );
    }
}
#[cfg(unix)]
#[test]
fn rejects_symlinks_and_non_regular_sources() {
    use std::os::unix::fs::symlink;
    let dir = fixture(b"-- koru-module: linked\nreturn {}");
    let outside = fixture(b"return 'outside'");
    symlink(
        outside.path().join("demo.lua"),
        dir.path().join("lib/linked.lua"),
    )
    .unwrap();
    assert!(capture(&dir).is_err());
    fs::remove_file(dir.path().join("lib/linked.lua")).unwrap();
    fs::create_dir(dir.path().join("lib/linked.lua")).unwrap();
    assert!(capture(&dir).is_err());
    fs::remove_file(dir.path().join("demo.lua")).unwrap();
    symlink(outside.path().join("demo.lua"), dir.path().join("demo.lua")).unwrap();
    assert!(discover(dir.path()).is_err());
}
#[test]
fn directives_inside_block_or_after_code_are_ignored() {
    let dir = fixture(b"--[[\n-- koru-module: hidden\n]]\nreturn {}\n-- koru-module: late");
    assert!(capture(&dir).unwrap().modules().is_empty());
}
