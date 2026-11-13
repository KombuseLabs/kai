//! Step 0: what a run is given, checked before anything is made.

use std::path::Path;

use guest::{JOB_FILE, OUT_DIR};

use super::RunSpec;
use crate::Error;

/// The most ports a run takes, the proxy's included.
const MAX_PORTS: usize = 8;

/// What the run is given, checked; returns the size of the job drive in
/// MiB.
pub(super) fn check(spec: &RunSpec) -> Result<u64, Error> {
    let refuse = |why: String| Err(Error::Refused(why));
    if spec.argv.is_empty() {
        return refuse("no program to run".to_owned());
    }
    let mut content = 0;
    if let Some(tree) = &spec.tree {
        if !tree.is_dir() {
            return refuse(format!("the tree {} is not a directory", tree.display()));
        }
        content += size_of(tree);
    }
    for file in &spec.files {
        if !job_path_ok(&file.to) {
            return refuse(format!(
                "`{}` is not a path under /job kai may write: plain names, not job.json, not under out/",
                file.to
            ));
        }
        if std::fs::symlink_metadata(&file.from).is_err() {
            return refuse(format!("no file {}", file.from.display()));
        }
        content += size_of(&file.from);
    }
    if spec.layers.len() > guest::MAX_LAYERS as usize {
        return refuse(format!(
            "{} layers; a run takes at most {}",
            spec.layers.len(),
            guest::MAX_LAYERS
        ));
    }
    for layer in &spec.layers {
        if !layer.is_file() {
            return refuse(format!("no layer file {}", layer.display()));
        }
    }
    if spec.upper_out.is_some() && spec.proxy.as_ref().is_some_and(|policy| policy.learn) {
        return refuse(
            "a learn run lets every host through, so what it wrote is never kept as a layer: no --upper-out with --proxy-learn"
                .to_owned(),
        );
    }
    let mut guests: Vec<u16> = spec.ports.iter().map(|port| port.guest).collect();
    if spec.proxy.is_some() {
        guests.push(proxy::GUEST_PORT);
    }
    if guests.len() > MAX_PORTS {
        return refuse(format!(
            "{} ports; a run takes at most {MAX_PORTS}",
            guests.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for guest in guests {
        if guest == 0 || !seen.insert(guest) {
            return refuse(format!(
                "port {guest} is taken twice, or is 0; the proxy takes {}",
                proxy::GUEST_PORT
            ));
        }
    }
    for (what, target) in [
        ("--tree-out", &spec.tree_out),
        ("--upper-out", &spec.upper_out),
    ] {
        if let Some(target) = target
            && std::fs::symlink_metadata(target).is_ok()
        {
            return refuse(format!("{what} {} exists already", target.display()));
        }
    }
    let content_mib = content.div_ceil(1 << 20);
    // ext4's own needs, from its inode tables and journal, with room.
    let needed = content_mib + content_mib / 10 + 64;
    match spec.disk_mib {
        None => Ok(content_mib * 2 + 2048),
        Some(disk) if disk >= needed => Ok(disk),
        Some(disk) => refuse(format!(
            "a job drive of {disk} MiB cannot hold the tree and files of {content_mib} MiB; --disk {needed} would do"
        )),
    }
}

/// Bytes `path` takes on its disk, everything below it included, plus a
/// block for each entry; symbolic links are not followed.
fn size_of(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    let mut total = meta.blocks() * 512 + 4096;
    if meta.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        for entry in entries.flatten() {
            total += size_of(&entry.path());
        }
    }
    total
}

/// A path under `/job` that `--file` may write: plain names joined by
/// `/`, not `job.json` and not under `out/`.
fn job_path_ok(path: &str) -> bool {
    let names: Vec<&str> = path.split('/').collect();
    !path.is_empty()
        && names
            .iter()
            .all(|name| !name.is_empty() && *name != "." && *name != ".." && !name.contains('\0'))
        && path != JOB_FILE
        && names[0] != OUT_DIR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::{dir, spec};

    #[test]
    fn files_go_under_job_and_nowhere_else() {
        assert!(job_path_ok("hidden/tests.rs"));
        assert!(job_path_ok("tree/tests/hidden.rs"));
        assert!(job_path_ok("kai"));
        for bad in [
            "",
            "/etc/x",
            "../x",
            "a/../b",
            "a//b",
            "./a",
            "job.json",
            "out",
            "out/stdout",
        ] {
            assert!(!job_path_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_job_drive_too_small_for_the_tree_is_refused_with_the_size_that_would_do() {
        let tree = dir("small-tree");
        std::fs::write(tree.join("big"), vec![1u8; 3 << 20]).unwrap();
        let mut spec = spec();
        spec.tree = Some(tree.to_path_buf());
        spec.disk_mib = Some(8);
        let err = check(&spec).unwrap_err().to_string();
        assert!(err.contains("--disk"), "{err}");
        spec.disk_mib = None;
        assert!(check(&spec).unwrap() >= 2048 + 6);
    }

    #[test]
    fn targets_that_exist_and_missing_layers_are_refused() {
        let base = dir("targets");
        let mut spec = spec();
        spec.tree_out = Some(base.to_path_buf());
        assert!(check(&spec).unwrap_err().to_string().contains("--tree-out"));
        spec.tree_out = None;
        spec.upper_out = Some(base.to_path_buf());
        assert!(
            check(&spec)
                .unwrap_err()
                .to_string()
                .contains("--upper-out")
        );
        spec.upper_out = None;
        spec.layers = vec![base.join("no-such-layer")];
        assert!(
            check(&spec)
                .unwrap_err()
                .to_string()
                .contains("no layer file")
        );
        spec.layers = vec![base.join("x"); 9];
        assert!(check(&spec).unwrap_err().to_string().contains("at most"));
    }
}
