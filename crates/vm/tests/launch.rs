//! `vm::run` with a stand-in for Firecracker: what a run does on the host
//! around the VM, without `/dev/kvm` and without the artefacts of
//! `kai vm fetch`. Needs `cp` and e2fsprogs, as every run does.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use vm::{Launch, Outcome, RunSpec};

fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-vm-launch-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A launch whose Firecracker is the shell script `script`, which runs in
/// the run directory; the kernel and the initramfs are files nothing reads.
fn launch(dir: &Path, script: &str) -> Launch {
    let firecracker = dir.join("firecracker");
    std::fs::write(&firecracker, format!("#!/bin/sh\n{script}")).unwrap();
    std::fs::set_permissions(&firecracker, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("vmlinux"), "no kernel").unwrap();
    std::fs::write(dir.join("initramfs.cpio"), "no initramfs").unwrap();
    Launch {
        firecracker,
        firecracker_sha256: "0".repeat(64),
        kernel: dir.join("vmlinux"),
        kernel_sha256: "1".repeat(64),
        initramfs: dir.join("initramfs.cpio"),
        initramfs_sha256: "2".repeat(64),
    }
}

fn nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// The nanoseconds since 1970 that `date +%s%N` wrote to `path`.
fn nanos_in(path: &Path) -> u128 {
    let text = std::fs::read_to_string(path).unwrap();
    text.trim().parse().unwrap()
}

#[tokio::test]
async fn wall_ms_is_firecrackers_time_and_not_the_making_of_the_drives() {
    let dir = dir("wall");
    // 3,000 files, so that copying the tree and making the job drive take a
    // time that shows.
    let tree = dir.join("tree");
    for n in 0..3000 {
        let sub = tree.join(format!("d{}", n / 100));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f{n}")), n.to_string()).unwrap();
    }
    let image = dir.join("image.sqfs");
    std::fs::write(&image, "not read").unwrap();
    let launch = launch(
        &dir,
        "date +%s%N > started\nsleep 0.2\ndate +%s%N > ended\n",
    );
    let mut spec = RunSpec::new(image, vec!["true".to_owned()], dir.join("run"));
    spec.tree = Some(tree);

    let before = nanos_now();
    let result = vm::run(&launch, &spec).await.unwrap();

    // The stand-in ends with 0 and leaves no status on the job drive.
    assert_eq!(result.outcome, Outcome::GuestFailed, "{result:?}");
    let started = nanos_in(&dir.join("run/started"));
    let ended = nanos_in(&dir.join("run/ended"));
    let preparation_ms = (started - before) / 1_000_000;
    let ran_ms = (ended - started) / 1_000_000;
    let wall_ms = u128::from(result.wall_ms.expect("Firecracker started"));
    assert!(
        wall_ms >= ran_ms,
        "wall_ms {wall_ms}, the stand-in ran {ran_ms} ms"
    );
    assert!(
        wall_ms < ran_ms + preparation_ms / 2,
        "wall_ms {wall_ms}, the stand-in ran {ran_ms} ms after {preparation_ms} ms of preparation"
    );
}
