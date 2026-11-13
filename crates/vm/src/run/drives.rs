//! The run's drives on the host. kai makes them and reads them, and never
//! mounts one: the job drive, made with `mke2fs -d` (step 2), repaired after
//! a kill and its tree written out with `debugfs rdump` (step 5); the upper
//! drive, made empty as a part beside its target, kept as a layer or
//! removed.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use guest::{JOB_FILE, JOB_FORMAT, Job, TREE_DIR};
use helpers::fs::PartDir;
use helpers::process::{self, Failed};
use tokio::process::Command;

use super::ways_out::WaysOut;
use super::{RunSpec, absolute};

/// Step 2: `job.json`, the tree and the files, made into `job.ext4`.
pub(super) async fn job_drive(
    spec: &RunSpec,
    dir: &Path,
    disk_mib: u64,
    ways_out: &WaysOut,
) -> Result<(), String> {
    let stage = dir.join("job");
    std::fs::create_dir_all(&stage).map_err(|err| format!("make job/: {err}"))?;
    match &spec.tree {
        // The tree's contents, links, modes and holes kept; a copy, so the
        // host's tree is never changed.
        Some(tree) => copy(&tree.join("."), &stage.join(TREE_DIR)).await?,
        None => std::fs::create_dir(stage.join(TREE_DIR))
            .map_err(|err| format!("make job/tree: {err}"))?,
    }
    for file in &spec.files {
        let to = place(&stage, &file.to)?;
        copy(&file.from, &to).await?;
    }
    let job = Job {
        format: JOB_FORMAT,
        argv: spec.argv.clone(),
        env: ways_out.env.clone(),
        cwd: spec.cwd.clone(),
        uid: spec.uid,
        gid: spec.gid,
        timeout_s: spec.timeout_s,
        layers: u32::try_from(spec.layers.len()).unwrap_or(u32::MAX),
        upper_out: spec.upper_out.is_some(),
        ports: ways_out.ports.clone(),
    };
    let text = serde_json::to_vec_pretty(&job).map_err(|err| err.to_string())?;
    std::fs::write(stage.join(JOB_FILE), text).map_err(|err| format!("write job.json: {err}"))?;
    mke2fs(dir, Some(&stage), &dir.join("job.ext4"), disk_mib).await?;
    std::fs::remove_dir_all(&stage).map_err(|err| format!("remove job/: {err}"))
}

/// Where the job file `to` goes in `stage`, made ready for the copy. The
/// tree and earlier files may have put symbolic links there, pointing
/// anywhere on the host: every directory on the way must be a directory of
/// the stage, made when it is missing, never a link, and whatever lies at
/// the place itself goes, so that the file replaces it and nothing is
/// written through a link. `to` is a path `check` let through: plain
/// names joined by `/`.
fn place(stage: &Path, to: &str) -> Result<PathBuf, String> {
    let names: Vec<&str> = to.split('/').collect();
    let Some((last, directories)) = names.split_last() else {
        return Err("a job file without a path".to_owned());
    };
    let mut path = stage.to_path_buf();
    for (depth, name) in directories.iter().enumerate() {
        path.push(name);
        let not_a_directory = || {
            format!(
                "--file {to}: {} is a link or a file in the job drive, not a directory",
                names[..=depth].join("/")
            )
        };
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(not_a_directory()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&path)
                    .map_err(|err| format!("make {}: {err}", path.display()))?;
            }
            Err(err) => return Err(format!("{}: {err}", path.display())),
        }
    }
    path.push(last);
    let cleared = match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
        Ok(_) => std::fs::remove_file(&path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    };
    cleared.map_err(|err| format!("clear {}: {err}", path.display()))?;
    Ok(path)
}

/// `cp -a --reflink=auto`: the contents, links, modes, times and holes
/// kept, and on a file system that can, without copying the bytes.
async fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let mut command = Command::new("cp");
    command
        .args(["-a", "--reflink=auto", "--"])
        .arg(from)
        .arg(to)
        .stdin(Stdio::null());
    process::run(&mut command)
        .await
        .map(drop)
        .map_err(|failed| match failed {
            Failed::Start { source, .. } => format!("cannot run cp: {source}"),
            Failed::Status { stderr, .. } => format!("copy {}: {stderr}", from.display()),
        })
}

/// An ext4 file of `size_mib` at `file`, sparse, holding `content` when
/// one is given, owned by root at its top.
pub(super) async fn mke2fs(
    dir: &Path,
    content: Option<&Path>,
    file: &Path,
    size_mib: u64,
) -> Result<(), String> {
    let mut command = Command::new("mke2fs");
    command.args(["-q", "-t", "ext4", "-E", "root_owner=0:0"]);
    if let Some(content) = content {
        command.arg("-d").arg(content);
    }
    command
        .arg(file)
        .arg(format!("{size_mib}M"))
        .current_dir(dir)
        .stdin(Stdio::null());
    // Failed reads "cannot run mke2fs: …" or "mke2fs exit status: 1: …".
    process::run(&mut command)
        .await
        .map(drop)
        .map_err(|failed| failed.to_string())
}

/// Step 5, `--tree-out`: the job drive's tree, through `debugfs rdump`, into
/// a directory beside `target`, then renamed to it. Names come from the
/// guest; `rdump` writes them below that directory, owned by the caller,
/// modes kept, links as links, device files left out. debugfs splits its
/// request at spaces, so no host path goes into it: `rdump` writes into
/// its working directory, and the drive is an argument of its own.
pub(super) async fn tree_out(dir: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("make {}: {err}", parent.display()))?;
    }
    // Removed when it goes out of scope, also when the run is dropped
    // while debugfs runs.
    let part = PartDir::beside(target)
        .map_err(|err| format!("make a directory beside {}: {err}", target.display()))?;
    // Not process::run: debugfs ends with 0 when `rdump` fails, and says
    // why only on standard error, which run keeps only on a failure.
    let output = Command::new("debugfs")
        .arg("-R")
        .arg(format!("rdump /{TREE_DIR} ."))
        .arg(absolute(&dir.join("job.ext4"))?)
        .current_dir(part.path())
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|err| format!("cannot run debugfs: {err}"))?;
    let dumped = part.path().join(TREE_DIR);
    let renamed = if output.status.success() && dumped.is_dir() {
        std::fs::rename(&dumped, target)
            .map_err(|err| format!("rename to {}: {err}", target.display()))
    } else {
        Err(format!(
            "debugfs rdump: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    };
    drop(part);
    renamed
}

/// After a kill: `e2fsck -fy` replays the job drive's journal, so that what
/// the guest wrote before is read back.
pub(super) async fn repair(dir: &Path) {
    let _ = Command::new("e2fsck")
        .args(["-fy", "job.ext4"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::dir;

    #[test]
    fn a_job_file_is_placed_in_the_stage_and_never_through_a_link() {
        let base = dir("place");
        let host = base.join("host");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(host.join("x"), "the host's").unwrap();
        let stage = base.join("stage");
        std::fs::create_dir_all(stage.join("tree/real")).unwrap();
        std::fs::write(stage.join("tree/file"), "the tree's").unwrap();
        std::os::unix::fs::symlink(&host, stage.join("tree/linked")).unwrap();
        std::os::unix::fs::symlink(host.join("x"), stage.join("tree/x")).unwrap();

        // Missing directories are made.
        assert_eq!(
            place(&stage, "hidden/deep/a").unwrap(),
            stage.join("hidden/deep/a")
        );
        assert!(stage.join("hidden/deep").is_dir());
        // A link or a file on the way is refused.
        let err = place(&stage, "tree/linked/x").unwrap_err();
        assert!(err.contains("tree/linked is a link"), "{err}");
        let err = place(&stage, "tree/file/x").unwrap_err();
        assert!(err.contains("tree/file is a link or a file"), "{err}");
        // What lies at the place goes: a link, a file, a directory.
        for name in ["tree/x", "tree/file", "tree/real"] {
            assert_eq!(place(&stage, name).unwrap(), stage.join(name));
            assert!(
                std::fs::symlink_metadata(stage.join(name)).is_err(),
                "{name}"
            );
        }
        // The host as it was.
        assert_eq!(
            std::fs::read_to_string(host.join("x")).unwrap(),
            "the host's"
        );
        assert_eq!(std::fs::read_dir(&host).unwrap().count(), 1);
    }
}
