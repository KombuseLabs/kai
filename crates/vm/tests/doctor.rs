//! `vm::doctor` on caches made here. What the machine has (`/dev/kvm`, the
//! programs) differs between machines, so only the artefacts' checks are
//! asserted.

use std::path::PathBuf;

use helpers::hash::sha256_file;
use vm::doctor::{self, Check};
use vm::{ARTEFACTS, Artefact};

fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-vm-doctor-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn invented(sha256: &'static str) -> Artefact {
    Artefact {
        name: "invented kernel",
        file: "vmlinux-invented",
        url: "https://example.com/vmlinux-invented",
        download_sha256: sha256,
        member: None,
        sha256,
        executable: false,
    }
}

#[test]
fn with_an_empty_cache_every_artefact_is_unmet_and_kai_vm_fetch_is_named() {
    let cache = dir("empty");
    let checks = doctor::check(&cache);
    for artefact in ARTEFACTS {
        let check: &Check = checks
            .iter()
            .find(|check| check.need == artefact.name)
            .unwrap();
        assert!(!check.met, "{check:?}");
        assert!(check.detail.contains("`kai vm fetch`"), "{check:?}");
    }
}

#[test]
fn an_artefact_with_its_pinned_hash_is_met() {
    let cache = dir("pinned");
    let file = cache.join("vmlinux-invented");
    std::fs::write(&file, b"an invented kernel").unwrap();
    let hash: &'static str = Box::leak(sha256_file(&file).unwrap().into_boxed_str());

    let check = doctor::artefact(&invented(hash), &cache);
    assert!(check.met, "{check:?}");
}

#[test]
fn an_artefact_with_another_hash_is_unmet_and_fetch_replaces_it() {
    let cache = dir("other");
    std::fs::write(cache.join("vmlinux-invented"), b"something else").unwrap();

    let check = doctor::artefact(
        &invented("0000000000000000000000000000000000000000000000000000000000000000"),
        &cache,
    );
    assert!(!check.met, "{check:?}");
    assert!(check.detail.contains("not the pinned file"), "{check:?}");
    assert!(
        check.detail.contains("`kai vm fetch` replaces it"),
        "{check:?}"
    );
}
