//! What a run is given: what runs a VM on this machine ([`Launch`]), and
//! what to run in it, with how much ([`RunSpec`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fetch::hash_file;
use crate::{Error, FIRECRACKER, KERNEL, initramfs};

/// A VM's processors when the caller names no number.
pub const DEFAULT_CPUS: u32 = 2;

/// A VM's memory in MiB when the caller names none.
pub const DEFAULT_MEMORY_MIB: u32 = 2048;

/// Seconds a command may run when the caller names no limit.
pub const DEFAULT_TIMEOUT_S: u64 = 600;

/// `PATH` for a command when the caller names none.
pub const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The command's directory when the caller names none: the work tree.
pub const DEFAULT_CWD: &str = "/job/tree";

/// What runs a VM on this machine: Firecracker, the kernel and the
/// initramfs, each checked.
#[derive(Clone, Debug)]
pub struct Launch {
    /// The Firecracker program.
    pub firecracker: PathBuf,
    /// Its SHA-256.
    pub firecracker_sha256: String,
    /// The guest kernel, an uncompressed `vmlinux`.
    pub kernel: PathBuf,
    /// Its SHA-256.
    pub kernel_sha256: String,
    /// The initramfs with kai's init.
    pub initramfs: PathBuf,
    /// Its SHA-256.
    pub initramfs_sha256: String,
}

impl Launch {
    /// From the cache `kai vm fetch` fills: Firecracker and the kernel are
    /// checked against their pins, and the initramfs is written there when
    /// it is missing.
    pub async fn from_cache(cache: &Path) -> Result<Launch, Error> {
        let mut found = Vec::new();
        for artefact in [FIRECRACKER, KERNEL] {
            let path = cache.join(artefact.file);
            match hash_file(path.clone()).await {
                Ok(hash) if hash == artefact.sha256 => found.push((path, hash)),
                Ok(_) => {
                    return Err(Error::Artefact {
                        name: artefact.name,
                        problem: format!(
                            "{} is not the pinned file; `kai vm fetch` replaces it",
                            path.display()
                        ),
                    });
                }
                Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Error::Artefact {
                        name: artefact.name,
                        problem: format!("not in {}; `kai vm fetch` downloads it", cache.display()),
                    });
                }
                Err(err) => return Err(err),
            }
        }
        let (initramfs, initramfs_sha256) = initramfs::initramfs(cache)?;
        let [(firecracker, firecracker_sha256), (kernel, kernel_sha256)]: [(PathBuf, String); 2] =
            found.try_into().expect("two artefacts were checked");
        Ok(Launch {
            firecracker,
            firecracker_sha256,
            kernel,
            kernel_sha256,
            initramfs,
            initramfs_sha256,
        })
    }
}

/// What to run, in what, and with how much.
#[derive(Clone, Debug)]
pub struct RunSpec {
    /// The image: a squashfs or ext4 file, the guest's file system.
    pub image: PathBuf,
    /// Where the image came from, when it was made from a recipe.
    pub origin: Option<Origin>,
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// The command's whole environment.
    pub env: BTreeMap<String, String>,
    /// The command's directory in the guest.
    pub cwd: String,
    /// Processors of the VM.
    pub cpus: u32,
    /// Memory of the VM, in MiB.
    pub memory_mib: u32,
    /// Seconds the command may run; the guest ends it then.
    pub timeout_s: u64,
    /// The user id the command runs as.
    pub uid: u32,
    /// The group id the command runs as.
    pub gid: u32,
    /// Size of the job drive, and of the upper drive, in MiB; the files are
    /// sparse. `None`: twice the tree and the files, plus 2048.
    pub disk_mib: Option<u64>,
    /// A directory of the host copied in as the work tree, `/job/tree`;
    /// never changed on the host. `None`: an empty tree.
    pub tree: Option<PathBuf>,
    /// Where the tree is written after the run; must not exist yet.
    pub tree_out: Option<PathBuf>,
    /// Files or directories of the host copied onto the job drive, each to
    /// a path under `/job`.
    pub files: Vec<JobFile>,
    /// Read-only layers over the image, the last on top: files an earlier
    /// run kept with [`RunSpec::upper_out`], or file systems made by hand.
    pub layers: Vec<PathBuf>,
    /// Where the overlay's upper layer is kept as a layer when the command
    /// exits with 0; must not exist yet.
    pub upper_out: Option<PathBuf>,
    /// Ways out: for each, the guest's `127.0.0.1:<guest port>` reaches a
    /// socket or address of the host.
    pub ports: Vec<Port>,
    /// A way out through an `Egress` (crates/proxy, `fence proxy` behind
    /// it) on the guest's `127.0.0.1:3128`, with the proxy variables in the
    /// command's environment; the policy says which hosts it lets through
    /// and which `fence` runs.
    pub proxy: Option<proxy::Policy>,
    /// The run directory; made by the run, so it must not exist yet, or be
    /// empty.
    pub run_dir: PathBuf,
}

/// A way out of the guest: its `127.0.0.1:<guest>` reaches `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// The port the command connects to in the guest.
    pub guest: u16,
    /// What it reaches on the host.
    pub to: PortTarget,
}

/// What a port reaches on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortTarget {
    /// A Unix socket.
    Unix(PathBuf),
    /// A TCP address, `host:port`.
    Tcp(String),
}

impl RunSpec {
    /// A run of `argv` on `image` in `run_dir`, with kai's defaults: only
    /// `PATH` set, the work tree as the directory, [`DEFAULT_CPUS`],
    /// [`DEFAULT_MEMORY_MIB`], [`DEFAULT_TIMEOUT_S`], root, an empty tree,
    /// no layer.
    pub fn new(image: PathBuf, argv: Vec<String>, run_dir: PathBuf) -> RunSpec {
        RunSpec {
            image,
            origin: None,
            argv,
            env: BTreeMap::from([("PATH".to_owned(), DEFAULT_PATH.to_owned())]),
            cwd: DEFAULT_CWD.to_owned(),
            cpus: DEFAULT_CPUS,
            memory_mib: DEFAULT_MEMORY_MIB,
            timeout_s: DEFAULT_TIMEOUT_S,
            uid: 0,
            gid: 0,
            disk_mib: None,
            tree: None,
            tree_out: None,
            files: Vec::new(),
            layers: Vec::new(),
            upper_out: None,
            ports: Vec::new(),
            proxy: None,
            run_dir,
        }
    }
}

/// A file or directory of the host for the job drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobFile {
    /// The file or directory on the host.
    pub from: PathBuf,
    /// Where it goes, relative to `/job`: plain names joined by `/`, not
    /// `job.json` and not under `out/`.
    pub to: String,
}

/// The recipe an image was made from, and the image's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The recipe's name.
    pub recipe: String,
    /// The image's id, `sha256:<hex>`.
    pub id: String,
}
