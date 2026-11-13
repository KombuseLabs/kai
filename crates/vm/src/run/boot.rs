//! Steps 2 to 4: the drives made, `vm.json` written, and Firecracker run
//! on it until the guest resets the VM, or killed past the time limit.

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use helpers::fs::Part;
use helpers::process;
use tokio::process::Command;

use super::drives;
use super::ways_out::{VSOCK, WaysOut};
use super::{GRACE, Launch, RunSpec, absolute};

/// The kernel's command line: the serial console, a reset through the
/// keyboard controller that ends Firecracker, a reset one second after a
/// panic, no PCI, and only errors and worse on the console.
const BOOT_ARGS: &str = "console=ttyS0 reboot=k panic=1 pci=off loglevel=4";

/// The most of Firecracker's standard error kept.
const MAX_STDERR: usize = 64 * 1024;

/// The guest's vsock context id; the host is 2.
const GUEST_CID: u32 = 3;

/// How Firecracker ended, and how long it ran.
pub(super) struct Ended {
    /// `Some`: it ended by itself, with this status; `None`: it was killed
    /// past the time limit.
    pub(super) status: Option<ExitStatus>,
    /// From its start to its end, on the host: the drives made before and
    /// what is read back after are not in it.
    pub(super) wall: Duration,
}

/// `Ok`: Firecracker ran, and how it ended; `Err`: it did not start, with
/// the reason.
pub(super) async fn boot(
    launch: &Launch,
    spec: &RunSpec,
    disk_mib: u64,
    upper: Option<&Part>,
    ways_out: &WaysOut,
) -> Result<Ended, String> {
    let dir = absolute(&spec.run_dir)?;
    let image = absolute(&spec.image)?;
    if !image.is_file() {
        return Err(format!("no image file {}", image.display()));
    }
    // Every path of the host from here on is absolute: mke2fs and
    // Firecracker start in the run directory, where a relative one names
    // another file.
    let upper = upper.map(|upper| absolute(upper.path())).transpose()?;
    let layers = spec
        .layers
        .iter()
        .map(|layer| absolute(layer))
        .collect::<Result<Vec<_>, _>>()?;
    let (firecracker, kernel, initramfs) = (
        absolute(&launch.firecracker)?,
        absolute(&launch.kernel)?,
        absolute(&launch.initramfs)?,
    );
    drives::job_drive(spec, &dir, disk_mib, ways_out).await?;
    if let Some(upper) = &upper {
        drives::mke2fs(&dir, None, upper, disk_mib).await?;
    }
    let files = Files {
        kernel: &kernel,
        initramfs: &initramfs,
        image: &image,
        dir: &dir,
        layers: &layers,
        upper: upper.as_deref(),
    };
    let config = vm_config(&files, !ways_out.ports.is_empty(), spec);
    let text = serde_json::to_vec_pretty(&config).map_err(|err| err.to_string())?;
    std::fs::write(dir.join("vm.json"), text).map_err(|err| format!("write vm.json: {err}"))?;

    // Firecracker's logger writes to firecracker.log once it is set up;
    // before that, its first line goes to the console. Its last word on a
    // failure goes to standard error, which is added to firecracker.log
    // after it ended: the logger does not append, so the two cannot share
    // the file while it runs.
    let console = std::fs::File::create(dir.join("console.log"))
        .map_err(|err| format!("create console.log: {err}"))?;
    std::fs::File::create(dir.join("firecracker.log"))
        .map_err(|err| format!("create firecracker.log: {err}"))?;
    let started = Instant::now();
    let mut child = Command::new(&firecracker)
        .args(["--no-api", "--config-file", "vm.json"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(console)
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("cannot start {}: {err}", firecracker.display()))?;
    // Its standard error is read to the end, so that Firecracker never
    // blocks on a full pipe; the first MAX_STDERR bytes are kept.
    let stderr = child
        .stderr
        .take()
        .map(|stderr| process::keep_start(stderr, MAX_STDERR));
    let limit = Duration::from_secs(spec.timeout_s) + GRACE;
    let ended = process::wait_or_kill(&mut child, limit)
        .await
        .map(|status| Ended {
            status,
            wall: started.elapsed(),
        })
        .map_err(|err| format!("cannot wait for Firecracker: {err}"));
    if let Some(stderr) = stderr
        && let Ok(kept) = stderr.await
        && !kept.bytes.is_empty()
    {
        use std::io::Write as _;
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("firecracker.log"))
        {
            let _ = log.write_all(&kept.bytes);
        }
    }
    ended
}

/// What `vm.json` names on the host.
struct Files<'a> {
    kernel: &'a Path,
    initramfs: &'a Path,
    image: &'a Path,
    /// The run directory, where the job drive lies.
    dir: &'a Path,
    layers: &'a [PathBuf],
    upper: Option<&'a Path>,
}

/// `vm.json`: what Firecracker is started with. The drives appear in the
/// guest in this order: the image is `/dev/vda`, the job drive
/// `/dev/vdb`, then the layers, read-only, then the upper drive. No network
/// device; with ports, a vsock device. Firecracker's own log goes to
/// `firecracker.log` in the run directory, its working directory.
fn vm_config(files: &Files<'_>, vsock: bool, spec: &RunSpec) -> serde_json::Value {
    let Files {
        kernel,
        initramfs,
        image,
        dir,
        layers,
        upper,
    } = *files;
    let drive = |id: String, path: &Path, read_only: bool| serde_json::json!({ "drive_id": id, "path_on_host": path, "is_root_device": false, "is_read_only": read_only });
    let mut drives = vec![
        drive("image".to_owned(), image, true),
        drive("job".to_owned(), &dir.join("job.ext4"), false),
    ];
    for (index, layer) in layers.iter().enumerate() {
        drives.push(drive(format!("layer{index}"), layer, true));
    }
    if let Some(upper) = upper {
        drives.push(drive("upper".to_owned(), upper, false));
    }
    let mut config = serde_json::json!({
        "boot-source": {
            "kernel_image_path": kernel,
            "initrd_path": initramfs,
            "boot_args": BOOT_ARGS,
        },
        "drives": drives,
        "machine-config": { "vcpu_count": spec.cpus, "mem_size_mib": spec.memory_mib },
        "logger": { "log_path": "firecracker.log", "level": "Warning" },
    });
    if vsock {
        // A guest's connection to the host's port N reaches the Unix socket
        // `v.sock_N` beside `uds_path`; a relative path, since an absolute
        // one may be too long for a socket address.
        config["vsock"] = serde_json::json!({ "guest_cid": GUEST_CID, "uds_path": VSOCK });
    }
    config
}

/// The console as Firecracker wrote it, without the carriage returns of the
/// serial line.
pub(super) fn strip_carriage_returns(path: &Path) {
    if let Ok(text) = std::fs::read(path)
        && text.contains(&b'\r')
    {
        let text: Vec<u8> = text.into_iter().filter(|byte| *byte != b'\r').collect();
        let _ = std::fs::write(path, text);
    }
}

/// The first line of Firecracker's log that names an error, without its
/// time and thread, or a note that there is none.
pub(super) fn failure_line(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains("error"))
        .map(|line| match line.split_once("] ") {
            Some((_, message)) => message.trim().to_owned(),
            None => line.trim().to_owned(),
        })
        .unwrap_or_else(|| "its log names no error".to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::run::tests::spec;

    #[test]
    fn vm_json_has_the_image_the_job_drive_the_layers_and_the_upper_drive_in_order() {
        let spec = spec();
        let layers = [
            PathBuf::from("/layers/a.ext4"),
            PathBuf::from("/layers/b.ext4"),
        ];
        let files = Files {
            kernel: Path::new("/cache/vmlinux"),
            initramfs: Path::new("/cache/initramfs.cpio"),
            image: &spec.image,
            dir: &spec.run_dir,
            layers: &layers,
            upper: Some(Path::new("/layers/.upper.part")),
        };
        let config = vm_config(&files, true, &spec);
        let drives = config["drives"].as_array().unwrap();
        assert_eq!(drives[0]["path_on_host"], "/images/test.sqfs");
        assert_eq!(drives[0]["is_read_only"], true);
        assert_eq!(drives[1]["path_on_host"], "/runs/1/job.ext4");
        assert_eq!(drives[1]["is_read_only"], false);
        assert_eq!(drives[2]["path_on_host"], "/layers/a.ext4");
        assert_eq!(drives[3]["path_on_host"], "/layers/b.ext4");
        assert_eq!(drives[2]["is_read_only"], true);
        assert_eq!(drives[4]["drive_id"], "upper");
        assert_eq!(drives[4]["is_read_only"], false);
        assert_eq!(drives.len(), 5);
        assert_eq!(config["machine-config"]["vcpu_count"], 2);
        assert_eq!(config["machine-config"]["mem_size_mib"], 2048);
        assert!(config.get("network-interfaces").is_none());
        assert_eq!(config["vsock"]["uds_path"], "v.sock");
        let boot = config["boot-source"]["boot_args"].as_str().unwrap();
        assert!(
            boot.contains("reboot=k") && boot.contains("console=ttyS0"),
            "{boot}"
        );
    }
}
