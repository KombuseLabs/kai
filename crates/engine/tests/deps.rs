//! `deps` with stand-ins for Firecracker and fence: what deps does with what
//! a run left, without `/dev/kvm` and without the network. The Firecracker
//! stand-in writes the guest's status and output onto the job drive with
//! `debugfs`, as kai-init would, and ends; it can also take away the upper
//! drive a run would keep. Needs `cp` and e2fsprogs, as every run does.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use engine::deps::{self, Error, Layers, RUST, Request};
use helpers::testing::TestDir;
use types::RepoName;
use vm::Launch;
use vm::image::ImageInfo;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("engine-deps-{test}"))
}

fn script(path: &Path, text: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{text}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A launch whose Firecracker ends at once with the guest's status 0 and,
/// for a fetch run, the commit's line on standard output. For the runs of
/// the kinds in `lose`, it removes the upper drive first, so that kai
/// cannot keep it.
fn launch(dir: &Path, lose: &[&str]) -> Launch {
    let lose = lose.join(" ");
    let firecracker = dir.join("firecracker");
    script(
        &firecracker,
        &format!(
            r#"set -eu
case "$PWD" in *-fetch*) kind=fetch ;; *) kind=build ;; esac
printf '{{"format":1,"outcome":"exited","exit_code":0,"command_ms":1}}' > status.json
if [ "$kind" = fetch ]; then echo "kai-head {SHA} {tree}" > stdout.txt; else : > stdout.txt; fi
: > stderr.txt
printf 'mkdir out\ncd out\nwrite status.json status.json\nwrite stdout.txt stdout\nwrite stderr.txt stderr\n' > requests
debugfs -w -f requests job.ext4 > /dev/null 2>&1
for lost in {lose}; do
  if [ "$lost" = "$kind" ]; then
    rm "$(sed -n '/"drive_id": "upper"/,/}}/s/.*"path_on_host": "\(.*\)".*/\1/p' vm.json)"
  fi
done
"#,
            tree = "f".repeat(40),
        ),
    );
    for name in ["vmlinux", "initramfs.cpio"] {
        std::fs::write(dir.join(name), name).unwrap();
    }
    Launch {
        firecracker,
        firecracker_sha256: "0".repeat(64),
        kernel: dir.join("vmlinux"),
        kernel_sha256: "1".repeat(64),
        initramfs: dir.join("initramfs.cpio"),
        initramfs_sha256: "2".repeat(64),
    }
}

/// A fence that says it listens, keeps an empty ledger and ends when its
/// input closes.
fn fence(dir: &Path) -> PathBuf {
    let fence = dir.join("fence");
    script(
        &fence,
        "socket=${2#--listen=unix:}\n: > proxy.jsonl\necho \"listening unix:$socket\"\ncat > /dev/null\n",
    );
    fence
}

fn info() -> ImageInfo {
    ImageInfo {
        format: vm::image::IMAGE_FORMAT.to_owned(),
        recipe: "rust".to_owned(),
        id: format!("sha256:{}", "1".repeat(64)),
        file: format!("sha256:{}", "2".repeat(64)),
        docker_image: format!("sha256:{}", "3".repeat(64)),
        env: vec![],
        working_dir: None,
        user: None,
    }
}

/// Runs `deps` in `dir` with the stand-ins.
async fn run(dir: &Path, lose: &[&str]) -> Result<Layers, Error> {
    let repo: RepoName = "example-org/example-code".parse().unwrap();
    let image = dir.join("image.sqfs");
    std::fs::write(&image, "not read").unwrap();
    let info = info();
    let request = Request {
        ecosystem: &RUST,
        repo: &repo,
        sha: SHA,
        image: (&image, &info),
        cache: &dir.join("cache"),
        fence: Some(fence(dir)),
    };
    deps::deps(&launch(dir, lose), &request).await
}

/// The names in the cache's `layers/`, sorted.
fn layers(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("cache/layers"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn runs(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("cache/runs")).map_or(0, Iterator::count)
}

#[tokio::test]
async fn both_layers_are_made_then_taken_from_the_cache() {
    let dir = dir("made");
    let first = run(&dir, &[]).await.unwrap();
    assert!(!first.fetch.cached && !first.build.cached, "{first:?}");
    assert!(first.fetch.file.is_file() && first.build.file.is_file());
    assert_eq!(layers(&dir).len(), 4, "{:?}", layers(&dir));
    let second = run(&dir, &[]).await.unwrap();
    assert!(second.fetch.cached && second.build.cached, "{second:?}");
    assert_eq!(runs(&dir), 2);
}

#[tokio::test]
async fn a_commit_counts_as_fetched_once_its_fetch_layer_is_in_the_cache() {
    // The stand-in tells the runs apart by "fetch" in their directory.
    let dir = dir("commit-there");
    let cache = dir.join("cache");
    let repo: RepoName = "example-org/example-code".parse().unwrap();
    let other: RepoName = "example-org/other-code".parse().unwrap();
    assert!(!deps::fetched(&cache, &repo, SHA));
    run(&dir, &["build"]).await.unwrap_err();
    // The fetch layer was kept, though the build's was not.
    assert!(deps::fetched(&cache, &repo, SHA));
    assert!(deps::fetched(&cache, &repo, &SHA.to_ascii_uppercase()));
    assert!(!deps::fetched(&cache, &other, SHA));
    assert!(!deps::fetched(&cache, &repo, &"9".repeat(40)));
}

#[tokio::test]
async fn a_fetch_run_that_kept_nothing_does_not_count_as_fetched() {
    let dir = dir("nothing-kept");
    run(&dir, &["fetch"]).await.unwrap_err();
    let repo: RepoName = "example-org/example-code".parse().unwrap();
    assert!(!deps::fetched(&dir.join("cache"), &repo, SHA));
}

#[tokio::test]
async fn a_fetch_layer_its_run_did_not_keep_is_an_error_every_time() {
    let dir = dir("fetch-lost");
    for _ in 0..2 {
        let result = run(&dir, &["fetch"]).await;
        assert!(
            matches!(result, Err(Error::NotKept { kind: "fetch", .. })),
            "{result:?}"
        );
        assert_eq!(layers(&dir), Vec::<String>::new());
    }
}

#[tokio::test]
async fn a_build_layer_its_run_did_not_keep_is_an_error_every_time() {
    let dir = dir("build-lost");
    for _ in 0..2 {
        let result = run(&dir, &["build"]).await;
        assert!(
            matches!(result, Err(Error::NotKept { kind: "build", .. })),
            "{result:?}"
        );
        // The fetch layer and its JSON, nothing of the build.
        assert_eq!(layers(&dir).len(), 2, "{:?}", layers(&dir));
    }
}

#[tokio::test]
async fn a_layer_without_its_json_is_made_again_and_then_taken() {
    let dir = dir("orphans");
    // What a kai stopped between keeping a layer and writing its JSON
    // leaves: the files, without JSON.
    let made = run(&dir, &[]).await.unwrap();
    for layer in [&made.fetch, &made.build] {
        std::fs::remove_file(layer.file.with_extension("json")).unwrap();
        std::fs::write(&layer.file, "cut short").unwrap();
    }
    let again = run(&dir, &[]).await.unwrap();
    assert!(!again.fetch.cached && !again.build.cached, "{again:?}");
    for layer in [&again.fetch, &again.build] {
        assert_ne!(std::fs::read(&layer.file).unwrap(), b"cut short");
        assert!(layer.file.with_extension("json").is_file());
    }
    let cached = run(&dir, &[]).await.unwrap();
    assert!(cached.fetch.cached && cached.build.cached, "{cached:?}");
    assert_eq!(runs(&dir), 4);
    assert_eq!(layers(&dir).len(), 4, "{:?}", layers(&dir));
}
