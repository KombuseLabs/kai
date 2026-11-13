//! Runs in real microVMs. They need `/dev/kvm` and the artefacts
//! `kai vm fetch` downloads, so they are ignored by default; run them with
//!
//! ```sh
//! KAI_VM_CACHE=<the cache> cargo test -p vm --test run -- --ignored
//! ```
//!
//! (without `KAI_VM_CACHE`: `~/.cache/kai/vm`). The proxy's test needs
//! `fence` 0.3.0 or newer: on `PATH`, or named by `KAI_FENCE`. No network:
//! the image is made here, a directory with the static test program `probe`
//! that the build script builds with kai's init, packed with `mksquashfs`,
//! and the only hosts reached are servers of the test on this machine.

use std::path::{Path, PathBuf};
use std::process::Command;

use helpers::testing::TestDir;
use vm::{JobFile, Launch, Outcome, Port, PortTarget, RunResult, RunSpec};

/// The test program, built for musl by the build script.
const PROBE: &str = concat!(env!("OUT_DIR"), "/probe");

fn cache() -> PathBuf {
    match std::env::var_os("KAI_VM_CACHE") {
        Some(cache) => PathBuf::from(cache),
        None => PathBuf::from(std::env::var_os("HOME").expect("HOME is set")).join(".cache/kai/vm"),
    }
}

fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("vm-run-{test}"))
}

/// A squashfs image holding `/probe`, `/etc/motd` and `/data/a` and `b`.
fn image(dir: &Path) -> PathBuf {
    let root = dir.join("image-root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::copy(PROBE, root.join("probe")).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("etc/motd"), "from the image").unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("data/a"), "a").unwrap();
    std::fs::write(root.join("data/b"), "b").unwrap();
    let image = dir.join("test.sqfs");
    let status = Command::new("mksquashfs")
        .arg(&root)
        .arg(&image)
        .args(["-quiet", "-no-progress", "-all-root"])
        .status()
        .unwrap();
    assert!(status.success());
    image
}

fn spec(image: &Path, run_dir: PathBuf, argv: &[&str]) -> RunSpec {
    let mut spec = RunSpec::new(
        image.to_path_buf(),
        argv.iter().map(|arg| (*arg).to_owned()).collect(),
        run_dir,
    );
    spec.cpus = 1;
    spec.memory_mib = 256;
    spec.timeout_s = 60;
    spec.disk_mib = Some(64);
    spec
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

async fn launch() -> Launch {
    Launch::from_cache(&cache())
        .await
        .expect("the artefacts: run `kai vm fetch` first")
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_command_exits_with_its_status_and_both_streams_come_back() {
    let dir = dir("exit");
    let image = image(&dir);
    let result = vm::run(
        &launch().await,
        &spec(&image, dir.join("run"), &["/probe", "exit", "7"]),
    )
    .await
    .unwrap();

    assert_eq!(result.outcome, Outcome::Exited, "{result:?}");
    assert_eq!(result.exit_code, Some(7));
    assert_eq!(result.output_complete, Some(true));
    assert_eq!(read(dir.join("run/out/stdout")), "out\n");
    assert_eq!(read(dir.join("run/out/stderr")), "err\n");
    let written: RunResult = serde_json::from_str(&read(dir.join("run/result.json"))).unwrap();
    assert_eq!(written, result);
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_command_past_its_limit_is_a_timeout_with_its_output_complete() {
    let dir = dir("timeout");
    let image = image(&dir);
    let mut spec = spec(&image, dir.join("run"), &["/probe", "sleep"]);
    spec.timeout_s = 2;
    let result = vm::run(&launch().await, &spec).await.unwrap();

    assert_eq!(result.outcome, Outcome::Timeout, "{result:?}");
    assert_eq!(result.output_complete, Some(true));
    assert_eq!(result.exit_code, None);
    assert_eq!(read(dir.join("run/out/stdout")), "before the hang\n");
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_program_that_is_not_there_exits_with_127_as_in_a_shell() {
    let dir = dir("missing");
    let image = image(&dir);
    let result = vm::run(
        &launch().await,
        &spec(&image, dir.join("run"), &["nothere"]),
    )
    .await
    .unwrap();

    assert_eq!(result.outcome, Outcome::Exited, "{result:?}");
    assert_eq!(result.exit_code, Some(127));
    assert!(read(dir.join("run/out/stderr")).contains("nothere: not found"));
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn an_image_that_is_no_file_system_fails_the_guest_and_the_console_says_so() {
    let dir = dir("junk");
    let image = dir.join("junk.img");
    std::fs::write(&image, vec![0x5a; 1 << 20]).unwrap();
    let result = vm::run(
        &launch().await,
        &spec(&image, dir.join("run"), &["/probe", "exit", "0"]),
    )
    .await
    .unwrap();

    assert_eq!(result.outcome, Outcome::GuestFailed, "{result:?}");
    assert_eq!(result.output_complete, Some(false));
    assert!(read(dir.join("run/console.log")).contains("kai-init: mount the image /dev/vda"));
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_missing_kernel_fails_the_launch() {
    let dir = dir("no-kernel");
    let image = image(&dir);
    let mut launch = launch().await;
    launch.kernel = dir.join("no-such-vmlinux");
    let result = vm::run(
        &launch,
        &spec(&image, dir.join("run"), &["/probe", "exit", "0"]),
    )
    .await
    .unwrap();

    assert_eq!(result.outcome, Outcome::LaunchFailed, "{result:?}");
    assert!(
        result.reason.as_deref().unwrap_or("").contains("kernel"),
        "{result:?}"
    );
    assert_eq!(result.output_complete, None);
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn eight_runs_at_once_give_eight_results() {
    let dir = dir("eight");
    let image = image(&dir);
    let launch = launch().await;
    let mut runs = tokio::task::JoinSet::new();
    for n in 1..=8 {
        let code = n.to_string();
        let spec = spec(
            &image,
            dir.join(format!("run-{n}")),
            &["/probe", "exit", &code],
        );
        let launch = launch.clone();
        runs.spawn(async move { (n, vm::run(&launch, &spec).await) });
    }
    let mut seen = Vec::new();
    while let Some(joined) = runs.join_next().await {
        let (n, result) = joined.unwrap();
        let result = result.unwrap();
        assert_eq!(result.outcome, Outcome::Exited, "{result:?}");
        assert_eq!(result.exit_code, Some(n));
        seen.push(n);
    }
    seen.sort_unstable();
    assert_eq!(seen, (1..=8).collect::<Vec<_>>());
}

/// `probe do` with its steps, `;` between them.
fn steps(steps: &[&[&str]]) -> Vec<String> {
    let mut argv = vec!["/probe".to_owned(), "do".to_owned()];
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            argv.push(";".to_owned());
        }
        argv.extend(step.iter().map(|word| (*word).to_owned()));
    }
    argv
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn an_upper_layer_kept_by_one_run_is_a_layer_of_the_next() {
    let dir = dir("layer");
    let image = image(&dir);
    let launch = launch().await;
    let layer = dir.join("layer.ext4");

    let mut first = spec(&image, dir.join("run-1"), &[]);
    first.argv = steps(&[
        &["write", "/opt/a", "hello"],
        &["remove", "/etc/motd"],
        &["reset-dir", "/data"],
    ]);
    first.upper_out = Some(layer.clone());
    let result = vm::run(&launch, &first).await.unwrap();
    assert_eq!(result.outcome, Outcome::Exited, "{result:?}");
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}",
        read(dir.join("run-1/out/stderr"))
    );
    assert_eq!(result.upper_out, Some(true));
    let hash = helpers::hash::sha256_file(&layer).unwrap();

    let mut second = spec(&image, dir.join("run-2"), &[]);
    second.argv = steps(&[&["cat", "/opt/a"], &["cat", "/etc/motd"], &["ls", "/data"]]);
    second.layers = vec![layer.clone()];
    let result = vm::run(&launch, &second).await.unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}",
        read(dir.join("run-2/out/stderr"))
    );
    // The file written, the file removed (a whiteout), the directory
    // emptied (an opaque directory): as the first run left them.
    assert_eq!(
        read(dir.join("run-2/out/stdout")),
        "hello\nmissing /etc/motd\nnew\n"
    );
    assert_eq!(result.layers, [format!("sha256:{hash}")]);
    assert_eq!(
        helpers::hash::sha256_file(&layer).unwrap(),
        hash,
        "the layer is unchanged"
    );
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn an_upper_layer_is_kept_only_when_the_command_exits_with_0() {
    let dir = dir("no-layer");
    let image = image(&dir);
    let launch = launch().await;
    for (name, argv, timeout) in [
        ("exit-3", vec!["/probe", "exit", "3"], 60),
        ("timeout", vec!["/probe", "sleep"], 1),
    ] {
        let layer = dir.join(format!("{name}.ext4"));
        let mut spec = spec(&image, dir.join(name), &argv);
        spec.timeout_s = timeout;
        spec.upper_out = Some(layer.clone());
        let result = vm::run(&launch, &spec).await.unwrap();
        assert_eq!(result.upper_out, Some(false), "{name}: {result:?}");
        assert!(!layer.exists(), "{name}");
    }
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_tree_goes_in_and_comes_out_changed_and_the_hosts_tree_is_not() {
    let dir = dir("tree");
    let image = image(&dir);
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("changed"), "before").unwrap();
    std::fs::write(tree.join("removed"), "here").unwrap();
    std::fs::write(tree.join("sub/kept"), "kept").unwrap();
    std::fs::hard_link(tree.join("sub/kept"), tree.join("sub/link")).unwrap();
    std::os::unix::fs::symlink("sub/kept", tree.join("symlink")).unwrap();
    std::fs::write(tree.join("with\nnewline"), "odd").unwrap();
    std::fs::create_dir_all(tree.join(".git")).unwrap();
    std::fs::write(tree.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    let out = dir.join("tree-out");
    let mut spec = spec(&image, dir.join("run"), &[]);
    spec.argv = steps(&[
        &["write", "changed", "after"],
        &["remove", "removed"],
        &["write", "made", "new"],
        &["cat", "/job/hidden/secret"],
    ]);
    spec.tree = Some(tree.clone());
    spec.disk_mib = None;
    spec.tree_out = Some(out.clone());
    spec.files = vec![JobFile {
        from: tree.join("sub/kept"),
        to: "hidden/secret".to_owned(),
    }];
    let result = vm::run(&launch().await, &spec).await.unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}",
        read(dir.join("run/out/stderr"))
    );
    assert_eq!(result.tree_out, Some(true));
    assert_eq!(read(dir.join("run/out/stdout")), "kept\n");

    assert_eq!(read(out.join("changed")), "after");
    assert!(!out.join("removed").exists());
    assert_eq!(read(out.join("made")), "new");
    assert_eq!(read(out.join("sub/link")), "kept");
    assert_eq!(read(out.join("with\nnewline")), "odd");
    assert_eq!(read(out.join(".git/HEAD")), "ref: refs/heads/main\n");
    assert_eq!(
        std::fs::read_link(out.join("symlink")).unwrap(),
        PathBuf::from("sub/kept")
    );
    assert!(
        !out.join("hidden").exists(),
        "files beside the tree stay out of it"
    );
    // The host's tree is as it was.
    assert_eq!(read(tree.join("changed")), "before");
    assert_eq!(read(tree.join("removed")), "here");
    assert!(!tree.join("made").exists());
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_file_never_follows_a_link_of_the_tree_out_of_the_job_drive() {
    let dir = dir("tree-links");
    let image = image(&dir);
    // A tree whose links point at the host, as a repository's may.
    let host = dir.join("host");
    std::fs::create_dir_all(&host).unwrap();
    std::fs::write(host.join("hidden.rs"), "the host's").unwrap();
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::os::unix::fs::symlink(&host, tree.join("tests")).unwrap();
    std::os::unix::fs::symlink(host.join("hidden.rs"), tree.join("hidden.rs")).unwrap();
    let file = dir.join("hidden.rs");
    std::fs::write(&file, "the job's").unwrap();
    let launch = launch().await;

    // Through a linked directory: refused, and the host's file untouched.
    let mut through = spec(&image, dir.join("run-through"), &["/probe", "exit", "0"]);
    through.tree = Some(tree.clone());
    through.disk_mib = None;
    through.files = vec![JobFile {
        from: file.clone(),
        to: "tree/tests/hidden.rs".to_owned(),
    }];
    let result = vm::run(&launch, &through).await.unwrap();
    assert_eq!(result.outcome, Outcome::LaunchFailed, "{result:?}");
    assert!(
        result
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("tree/tests"),
        "{result:?}"
    );
    assert_eq!(read(host.join("hidden.rs")), "the host's");

    // Onto a link: the link is replaced in the job drive, not written
    // through.
    let mut onto = spec(&image, dir.join("run-onto"), &[]);
    onto.argv = steps(&[&["cat", "/job/tree/hidden.rs"]]);
    onto.tree = Some(tree.clone());
    onto.disk_mib = None;
    onto.files = vec![JobFile {
        from: file.clone(),
        to: "tree/hidden.rs".to_owned(),
    }];
    let result = vm::run(&launch, &onto).await.unwrap();
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert_eq!(read(dir.join("run-onto/out/stdout")), "the job's\n");
    assert_eq!(read(host.join("hidden.rs")), "the host's");
    assert!(
        tree.join("hidden.rs").is_symlink(),
        "the host's tree as it was"
    );
}

/// `path` relative to the current directory: `..` for each name of the
/// current directory below what the two share, then the rest of `path`.
/// From another directory it names another file.
fn from_here(path: &Path) -> PathBuf {
    let here = std::env::current_dir().unwrap();
    let shared = here
        .components()
        .zip(path.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative = PathBuf::new();
    for _ in shared..here.components().count() {
        relative.push("..");
    }
    relative.extend(path.components().skip(shared));
    relative
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn relative_paths_and_paths_with_spaces_are_taken_as_given() {
    // Beside the current directory, so that a path relative to it names
    // another file when taken from the run directory.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("kai-vm-run-{}-with spaces", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image = image(&dir);
    let launch = Launch::from_cache(&from_here(&cache())).await.unwrap();
    assert!(launch.firecracker.is_relative(), "{launch:?}");
    let tree = dir.join("a tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("in"), "in").unwrap();

    let mut spec = spec(&image, from_here(&dir.join("the run")), &[]);
    spec.argv = steps(&[&["write", "/opt/up", "up"], &["write", "made", "made"]]);
    spec.tree = Some(from_here(&tree));
    spec.disk_mib = None;
    spec.tree_out = Some(from_here(&dir.join("tree out")));
    spec.upper_out = Some(from_here(&dir.join("upper layer.ext4")));
    let result = vm::run(&launch, &spec).await.unwrap();
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert_eq!(result.tree_out, Some(true), "{result:?}");
    assert_eq!(result.upper_out, Some(true), "{result:?}");
    assert_eq!(read(dir.join("tree out/in")), "in");
    assert_eq!(read(dir.join("tree out/made")), "made");
    assert!(dir.join("upper layer.ext4").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn the_command_runs_as_the_user_it_is_given() {
    let dir = dir("user");
    let image = image(&dir);
    let mut spec = spec(&image, dir.join("run"), &["/probe", "do", "id"]);
    spec.uid = 1000;
    spec.gid = 1000;
    let result = vm::run(&launch().await, &spec).await.unwrap();
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert_eq!(read(dir.join("run/out/stdout")), "1000:1000\n");
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_user_that_is_not_root_gets_its_tree_and_a_home_and_nothing_else() {
    use std::os::unix::fs::symlink;

    let dir = dir("user-owns");
    let image = image(&dir);
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a"), "a").unwrap();
    std::fs::write(tree.join("sub/b"), "b").unwrap();
    // A link out of the tree, to a file of the image: the link is given to
    // the user, what it points to is not.
    symlink("/etc/motd", tree.join("link")).unwrap();
    let steps = [
        "write /job/tree/new n ; write /job/tree/sub/c c ; write /job/home/h h",
        "owner /job/tree ; owner /job/tree/a ; owner /job/tree/sub/b ; owner /job/tree/link",
        "owner /etc/motd ; owner /job/home ; owner /job/out ; id",
    ]
    .join(" ; ");
    let mut argv = vec!["/probe", "do"];
    argv.extend(steps.split(' '));
    // Not this machine's user: the ids the job drive was made with do not
    // matter.
    let mut spec = spec(&image, dir.join("run"), &argv);
    spec.uid = 4321;
    spec.gid = 4322;
    spec.tree = Some(tree);
    spec.disk_mib = Some(128);
    let result = vm::run(&launch().await, &spec).await.unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{result:?} {}",
        read(dir.join("run/out/stderr"))
    );
    let stdout = read(dir.join("run/out/stdout"));
    // Owners, and the home's mode; the other modes are the host's umask.
    let owners: Vec<&str> = stdout
        .lines()
        .map(|line| line.split(' ').next().unwrap_or_default())
        .collect();
    assert_eq!(
        owners,
        [
            "4321:4322", // the tree
            "4321:4322", // a file of the tree
            "4321:4322", // a file in a directory of the tree
            "4321:4322", // the link
            "0:0",       // what the link points to
            "4321:4322", // the home
            "0:0",       // out/
            "4321:4322", // the command's ids
        ],
        "{stdout}"
    );
    assert_eq!(stdout.lines().nth(5), Some("4321:4322 700"), "{stdout}");

    // The image stays root's.
    let mut image_spec = self::spec(
        &image,
        dir.join("run-image"),
        &["/probe", "do", "write", "/data/c", "c"],
    );
    image_spec.uid = 4321;
    image_spec.gid = 4322;
    let result = vm::run(&launch().await, &image_spec).await.unwrap();
    assert_eq!(result.exit_code, Some(1), "{result:?}");
    assert!(
        read(dir.join("run-image/out/stderr")).contains("ermission denied"),
        "{}",
        read(dir.join("run-image/out/stderr"))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn what_becomes_roots_loses_setuid_and_setgid_and_keeps_its_other_bits() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let dir = dir("setid");
    let image = image(&dir);
    // The host's own ids, as in the test below: then the job drive's files
    // are the command's user's until the init makes them root's.
    let host = std::fs::metadata(&dir).unwrap();
    let (uid, gid) = match (host.uid(), host.gid()) {
        (0, _) => (1000, 1000),
        ids => ids,
    };
    let mode = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    // Harmless files with the bits set; nothing here is run.
    let given = dir.join("given");
    std::fs::create_dir(&given).unwrap();
    let files = [
        ("setuid", 0o4755),
        ("setgid", 0o2755),
        ("both", 0o6755),
        ("setuid-not-executable", 0o4644),
        ("executable", 0o755),
        ("plain", 0o644),
        ("writable", 0o666),
    ];
    for (name, bits) in files {
        let file = given.join(name);
        std::fs::write(&file, name).unwrap();
        mode(&file, bits);
    }
    std::fs::create_dir(given.join("setgid-dir")).unwrap();
    mode(&given.join("setgid-dir"), 0o2775);
    mode(&given, 0o755);
    let tree = dir.join("tree");
    std::fs::create_dir(&tree).unwrap();
    for (name, bits) in [("setuid", 0o4755), ("executable", 0o755)] {
        std::fs::write(tree.join(name), name).unwrap();
        mode(&tree.join(name), bits);
    }
    // The host kept the bits: what the guest gets comes from these.
    assert_eq!(
        std::fs::metadata(given.join("both")).unwrap().mode() & 0o7777,
        0o6755
    );

    let mut steps: Vec<String> = files
        .iter()
        .map(|(name, _)| format!("owner /job/given/{name}"))
        .collect();
    steps.extend(
        [
            "owner /job/given/setgid-dir",
            "owner /job/given",
            "owner /job/tree/setuid",
            "owner /job/tree/executable",
            "can-write /job/job.json",
            "can-write /job/tree/executable",
            "can-create /job/home",
        ]
        .map(str::to_owned),
    );
    let steps = steps.join(" ; ");
    let mut argv = vec!["/probe", "do"];
    argv.extend(steps.split(' '));
    let mut spec = spec(&image, dir.join("run"), &argv);
    spec.uid = uid;
    spec.gid = gid;
    spec.tree = Some(tree);
    spec.files = vec![JobFile {
        from: given,
        to: "given".to_owned(),
    }];
    spec.disk_mib = Some(128);
    let result = vm::run(&launch().await, &spec).await.unwrap();
    let stdout = read(dir.join("run/out/stdout"));
    assert_eq!(
        result.exit_code,
        Some(0),
        "{result:?} {stdout} {}",
        read(dir.join("run/out/stderr"))
    );
    let lines: Vec<&str> = stdout.lines().collect();
    let user = format!("{uid}:{gid}");
    assert_eq!(
        lines,
        [
            "0:0 755",              // setuid: the bit gone, the rest kept
            "0:0 755",              // setgid
            "0:0 755",              // both
            "0:0 644",              // setuid on a file that is not executable
            "0:0 755",              // an ordinary executable stays executable
            "0:0 644",              // a plain file stays readable
            "0:0 644",              // group's and others' write gone
            "0:0 755",              // a setgid directory
            "0:0 755",              // the given directory
            lines[9],               // the tree's setuid file: checked below
            &format!("{user} 755"), // the tree's executable, the user's
            "no /job/job.json",
            "yes /job/tree/executable",
            "yes /job/home",
        ],
        "{stdout}"
    );
    // The tree is the user's: whatever its bits, none of it is root's.
    let (owner, bits) = lines[9].split_once(' ').unwrap();
    assert_eq!(owner, user, "{stdout}");
    assert_eq!(u32::from_str_radix(bits, 8).unwrap() & 0o777, 0o755);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn a_user_with_the_hosts_own_uid_can_write_its_tree_and_home_and_nothing_else() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let dir = dir("host-uid");
    let image = image(&dir);
    // The job drive keeps the uids of the files it is made from, the
    // host user's; the command runs as that same user, as the agent of an
    // agent run does on a host whose user is 1000. Root on the host stages
    // root's files, so the case is then 1000.
    let host = std::fs::metadata(&dir).unwrap();
    let (uid, gid) = match (host.uid(), host.gid()) {
        (0, _) => (1000, 1000),
        ids => ids,
    };
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a"), "a").unwrap();
    // Given files, writable by everyone on the host.
    let given = dir.join("given");
    std::fs::write(&given, "given").unwrap();
    let given_dir = dir.join("given-dir");
    std::fs::create_dir(&given_dir).unwrap();
    std::fs::write(given_dir.join("inner"), "inner").unwrap();
    for path in [&given, &given_dir.join("inner")] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
    }
    std::fs::set_permissions(&given_dir, std::fs::Permissions::from_mode(0o777)).unwrap();

    let steps = [
        "can-write /job/tree/a ; can-create /job/tree ; can-create /job/tree/sub",
        "can-create /job/home ; can-write /job/job.json ; can-write /job/given",
        "can-write /job/given-dir/inner ; can-create /job ; can-create /job/given-dir",
        "can-create /job/out ; cat /job/given ; owner /job/job.json ; owner /job/given",
        "owner /job/given-dir/inner ; id",
    ]
    .join(" ; ");
    let mut argv = vec!["/probe", "do"];
    argv.extend(steps.split(' '));
    let mut spec = spec(&image, dir.join("run"), &argv);
    spec.uid = uid;
    spec.gid = gid;
    spec.tree = Some(tree);
    spec.files = vec![
        JobFile {
            from: given,
            to: "given".to_owned(),
        },
        JobFile {
            from: given_dir,
            to: "given-dir".to_owned(),
        },
    ];
    spec.disk_mib = Some(128);
    let result = vm::run(&launch().await, &spec).await.unwrap();
    let stdout = read(dir.join("run/out/stdout"));
    assert_eq!(
        result.exit_code,
        Some(0),
        "{result:?} {stdout} {}",
        read(dir.join("run/out/stderr"))
    );
    let lines: Vec<&str> = stdout.lines().collect();
    // What the command can do, tried, not read from the owners.
    assert_eq!(
        lines[..10],
        [
            "yes /job/tree/a",
            "yes /job/tree",
            "yes /job/tree/sub",
            "yes /job/home",
            "no /job/job.json",
            "no /job/given",
            "no /job/given-dir/inner",
            "no /job",
            "no /job/given-dir",
            "no /job/out",
        ],
        "{stdout}"
    );
    // What it was given it can still read.
    assert_eq!(lines[10], "given", "{stdout}");
    for (line, what) in lines[11..14].iter().zip(["job.json", "given", "inner"]) {
        let (owner, mode) = line.split_once(' ').unwrap();
        assert_eq!(owner, "0:0", "{what}: {stdout}");
        let mode = u32::from_str_radix(mode, 8).unwrap();
        assert_eq!(
            mode & 0o022,
            0,
            "{what} is writable by group or others: {stdout}"
        );
    }
    assert_eq!(lines[14], format!("{uid}:{gid}"), "{stdout}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn eight_runs_at_once_on_one_layer() {
    let dir = dir("eight-layer");
    let image = image(&dir);
    let launch = launch().await;
    let layer = dir.join("layer.ext4");
    let mut first = spec(&image, dir.join("first"), &[]);
    first.argv = steps(&[&["write", "/opt/shared", "shared"]]);
    first.upper_out = Some(layer.clone());
    assert_eq!(
        vm::run(&launch, &first).await.unwrap().upper_out,
        Some(true)
    );

    let mut runs = tokio::task::JoinSet::new();
    for n in 1..=8 {
        let mut spec = spec(
            &image,
            dir.join(format!("run-{n}")),
            &["/probe", "do", "cat", "/opt/shared"],
        );
        spec.layers = vec![layer.clone()];
        let launch = launch.clone();
        runs.spawn(async move { (n, vm::run(&launch, &spec).await) });
    }
    while let Some(joined) = runs.join_next().await {
        let (n, result) = joined.unwrap();
        assert_eq!(result.unwrap().exit_code, Some(0));
        assert_eq!(read(dir.join(format!("run-{n}/out/stdout"))), "shared\n");
    }
}

/// Answers each connection's first line with `pong from <tag>: <line>`.
fn answer(stream: impl std::io::Read + std::io::Write, tag: &str) {
    use std::io::BufRead as _;
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_ok() {
        let _ = writeln!(reader.get_mut(), "pong from {tag}: {}", line.trim_end());
    }
}

fn unix_server(path: &Path, tag: &'static str) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream, tag));
        }
    });
}

fn tcp_server(tag: &'static str) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream, tag));
        }
    });
    port
}

/// The run directory's sockets; none is left after a run.
fn sockets(dir: &Path) -> Vec<String> {
    use std::os::unix::fs::FileTypeExt as _;
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_socket()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[tokio::test]
#[ignore = "needs /dev/kvm and the artefacts of `kai vm fetch`"]
async fn ports_reach_the_hosts_servers_and_nothing_else() {
    let dir = dir("ports");
    let image = image(&dir);
    unix_server(&dir.join("a.sock"), "A");
    let tcp = tcp_server("B");
    let mut spec = spec(&image, dir.join("run"), &[]);
    spec.argv = steps(&[
        &["connect", "8080", "hello\\r\\n"],
        &["connect", "9090", "hi\\r\\n"],
        &["connect", "7070", "nobody\\r\\n"],
    ]);
    spec.ports = vec![
        Port {
            guest: 8080,
            to: PortTarget::Unix(dir.join("a.sock")),
        },
        Port {
            guest: 9090,
            to: PortTarget::Tcp(format!("127.0.0.1:{tcp}")),
        },
    ];
    let result = vm::run(&launch().await, &spec).await.unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}",
        read(dir.join("run/out/stderr"))
    );
    assert_eq!(
        read(dir.join("run/out/stdout")),
        "pong from A: hello\npong from B: hi\ncannot connect\n"
    );
    assert!(
        sockets(&dir.join("run")).is_empty(),
        "{:?}",
        sockets(&dir.join("run"))
    );
}

#[tokio::test]
#[ignore = "needs /dev/kvm, the artefacts of `kai vm fetch` and fence 0.3.0"]
async fn the_proxy_tunnels_what_its_policy_allows_and_refuses_the_rest() {
    let dir = dir("proxy");
    let image = image(&dir);
    let tcp = tcp_server("C");
    let mut policy = proxy::Policy::new([format!("127.0.0.1:{tcp}")]);
    if let Some(fence) = std::env::var_os("KAI_FENCE") {
        policy.fence = PathBuf::from(fence);
    }
    let mut spec = spec(&image, dir.join("run"), &[]);
    let allowed = format!("CONNECT 127.0.0.1:{tcp} HTTP/1.1\\r\\n\\r\\n");
    spec.argv = steps(&[
        &["connect", "3128", &allowed],
        &[
            "connect",
            "3128",
            "CONNECT example.com:443 HTTP/1.1\\r\\n\\r\\n",
        ],
    ]);
    spec.proxy = Some(policy);
    let result = vm::run(&launch().await, &spec).await.unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "{result:?} {}",
        read(dir.join("run/out/stderr"))
    );
    assert_eq!(
        read(dir.join("run/out/stdout")),
        "HTTP/1.1 200 Connection Established\nHTTP/1.1 403 Forbidden\n"
    );
    let proxy = result.proxy.unwrap();
    assert_eq!((proxy.allowed, proxy.refused), (1, 1));
    assert_eq!(proxy.refused_targets["example.com:443"], 1);
    assert!(read(dir.join("run/proxy.jsonl")).contains("\"decision\":\"allow\""));
    assert!(
        sockets(&dir.join("run")).is_empty(),
        "{:?}",
        sockets(&dir.join("run"))
    );
}
