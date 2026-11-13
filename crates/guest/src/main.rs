//! kai's init: PID 1 of every microVM. The same program for every image, so
//! that no image has to be prepared for kai. In order it:
//!
//! 1. mounts `devtmpfs`, the image (`/dev/vda`, squashfs else ext4)
//!    read-only at `/img` and the job drive (`/dev/vdb`, ext4) at `/j`;
//! 2. reads `/j/job.json` and refuses a format it does not know;
//! 3. mounts the job's layers (`/dev/vdc` on, squashfs else ext4 without
//!    replaying a journal) read-only, and lays an overlay over them and the
//!    image at `/root`, whose upper layer is on the job drive, or on a drive
//!    of its own when the job asks for one, so that what the command writes
//!    outside its tree lands there and not in memory; binds the job drive
//!    to `/root/job`; mounts `/proc`, `/sys`, `/dev` and a `/tmp` there;
//! 4. with ports, brings `lo` up and, for each, listens on
//!    `127.0.0.1:<port>` and passes every connection on to vsock, the host,
//!    the same port;
//! 5. starts the command in a process group of its own, changed into
//!    `/root` and the job's directory, as the job's user, with the job's
//!    environment, standard input closed and its output going to
//!    `/job/out`;
//! 6. reaps every process that ends and waits for the command; at the time
//!    limit it kills the command's process group;
//! 7. kills what is left, writes `out/status.json`, unmounts everything and
//!    resets the VM, which ends Firecracker.
//!
//! When a step before the command fails, it prints one line naming the step
//! and the error on the console and resets the VM. The launcher then finds
//! no status and reports the run as `guest-failed`.

#![allow(
    clippy::print_stderr,
    reason = "the console is where PID 1 says what went wrong"
)]

use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use guest::{
    Ending, JOB_FILE, JOB_FORMAT, Job, OUT_DIR, STATUS_FILE, STATUS_FORMAT, STDERR_FILE,
    STDOUT_FILE, Status,
};

/// Where the image is mounted.
const IMAGE: &str = "/img";
/// Where the job drive is mounted.
const DRIVE: &str = "/j";
/// Where the upper drive is mounted, when the job asks for one.
const UPPER: &str = "/u";
/// The command's root: the overlay.
const ROOT: &str = "/root";
/// The largest `job.json` read.
const MAX_JOB: u64 = 1 << 20;

fn main() {
    if let Err(err) = run() {
        eprintln!("kai-init: {err}");
    }
    shutdown();
}

/// Steps 1 to 7; an error names the step.
fn run() -> Result<(), String> {
    mount(Some("devtmpfs"), "/dev", Some("devtmpfs"), 0, None)
        .or_else(|err| {
            if err.raw_os_error() == Some(libc::EBUSY) {
                Ok(())
            } else {
                Err(err)
            }
        })
        .map_err(|err| format!("mount /dev: {err}"))?;
    mkdir(IMAGE)?;
    mount(
        Some("/dev/vda"),
        IMAGE,
        Some("squashfs"),
        libc::MS_RDONLY,
        None,
    )
    .or_else(|_| mount(Some("/dev/vda"), IMAGE, Some("ext4"), libc::MS_RDONLY, None))
    .map_err(|err| format!("mount the image /dev/vda: {err}"))?;
    mkdir(DRIVE)?;
    mount(Some("/dev/vdb"), DRIVE, Some("ext4"), 0, None)
        .map_err(|err| format!("mount the job drive /dev/vdb: {err}"))?;

    let job = read_job()?;
    if job.layers > guest::MAX_LAYERS {
        return Err(format!(
            "the job names {} layers; at most {} are taken",
            job.layers,
            guest::MAX_LAYERS
        ));
    }
    mkdir(&format!("{DRIVE}/{OUT_DIR}"))?;

    // The lower layers, top first: the layers in reverse, then the image.
    let mut lower = Vec::new();
    for index in 0..job.layers {
        let device = guest::drive(2 + index);
        let target = format!("/layer{index}");
        mkdir(&target)?;
        mount(
            Some(&device),
            &target,
            Some("squashfs"),
            libc::MS_RDONLY,
            None,
        )
        .or_else(|_| {
            mount(
                Some(&device),
                &target,
                Some("ext4"),
                libc::MS_RDONLY,
                Some("noload"),
            )
        })
        .map_err(|err| format!("mount the layer {device}: {err}"))?;
        // A layer an earlier run kept holds its upper layer in `upper/`; a
        // file system made by hand is a layer as it is.
        let upper = format!("{target}/upper");
        lower.push(if fs::metadata(&upper).is_ok_and(|meta| meta.is_dir()) {
            upper
        } else {
            target
        });
    }
    lower.reverse();
    lower.push(IMAGE.to_owned());

    let writes = if job.upper_out {
        let device = guest::drive(2 + job.layers);
        mkdir(UPPER)?;
        mount(Some(&device), UPPER, Some("ext4"), 0, None)
            .map_err(|err| format!("mount the upper drive {device}: {err}"))?;
        UPPER
    } else {
        DRIVE
    };
    for dir in ["upper", "work"] {
        mkdir(&format!("{writes}/{dir}"))?;
    }
    mkdir(ROOT)?;
    let layers = format!(
        "lowerdir={},upperdir={writes}/upper,workdir={writes}/work",
        lower.join(":")
    );
    mount(Some("overlay"), ROOT, Some("overlay"), 0, Some(&layers))
        .map_err(|err| format!("mount the overlay: {err}"))?;
    let inside = |path: &str| format!("{ROOT}{path}");
    for (source, target, fstype, data) in [
        (Some(DRIVE), "/job", None, None),
        (Some("proc"), "/proc", Some("proc"), None),
        (Some("sysfs"), "/sys", Some("sysfs"), None),
        (Some("devtmpfs"), "/dev", Some("devtmpfs"), None),
        (Some("tmpfs"), "/tmp", Some("tmpfs"), Some("mode=1777")),
    ] {
        let target = inside(target);
        mkdir(&target)?;
        let flags = if fstype.is_none() { libc::MS_BIND } else { 0 };
        mount(source, &target, fstype, flags, data)
            .map_err(|err| format!("mount {target}: {err}"))?;
    }

    if !job.ports.is_empty() {
        loopback_up().map_err(|err| format!("bring lo up: {err}"))?;
        for port in &job.ports {
            forward(*port).map_err(|err| format!("listen on 127.0.0.1:{port}: {err}"))?;
        }
    }

    let ending = command(&job)?;
    let status = Status {
        format: STATUS_FORMAT,
        ending: ending.0,
        command_ms: ending.1,
    };
    write_status(&status).map_err(|err| format!("write {OUT_DIR}/{STATUS_FILE}: {err}"))
}

/// Reads and checks `job.json` on the job drive.
fn read_job() -> Result<Job, String> {
    let path = format!("{DRIVE}/{JOB_FILE}");
    let size = fs::metadata(&path)
        .map_err(|err| format!("read {path}: {err}"))?
        .len();
    if size > MAX_JOB {
        return Err(format!("{path} has {size} bytes, more than {MAX_JOB}"));
    }
    let text = fs::read(&path).map_err(|err| format!("read {path}: {err}"))?;
    let job: Job = serde_json::from_slice(&text).map_err(|err| format!("read {path}: {err}"))?;
    if job.format != JOB_FORMAT {
        return Err(format!(
            "{path} has format {}; this init reads {JOB_FORMAT}",
            job.format
        ));
    }
    if job.argv.is_empty() {
        return Err(format!("{path} names no program"));
    }
    Ok(job)
}

/// Steps 5 and 6: runs the job's command and says how it ended, and how
/// many milliseconds it ran.
fn command(job: &Job) -> Result<(Ending, u64), String> {
    let out = |name: &str| format!("{DRIVE}/{OUT_DIR}/{name}");
    let stdout = File::create(out(STDOUT_FILE)).map_err(|err| format!("create stdout: {err}"))?;
    let stderr = File::create(out(STDERR_FILE)).map_err(|err| format!("create stderr: {err}"))?;
    let root = CString::new(ROOT).map_err(|err| err.to_string())?;
    let cwd = CString::new(job.cwd.as_str()).map_err(|_| "the job's cwd holds a NUL".to_owned())?;
    let (uid, gid) = (job.uid, job.gid);

    let mut command = Command::new(&job.argv[0]);
    command
        .args(&job.argv[1..])
        .env_clear()
        .envs(&job.env)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .process_group(0);
    // SAFETY: between fork and exec only system calls run, on strings made
    // before the fork; nothing there allocates or takes a lock, so the
    // forwarders' threads do not matter.
    unsafe {
        command.pre_exec(move || {
            check(libc::chroot(root.as_ptr()))?;
            check(libc::chdir(cwd.as_ptr()))?;
            check(libc::setgroups(0, std::ptr::null()))?;
            check(libc::setgid(gid))?;
            check(libc::setuid(uid))?;
            Ok(())
        });
    }

    let started = Instant::now();
    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let (code, why) = match err.kind() {
                io::ErrorKind::NotFound => (127, "not found".to_owned()),
                io::ErrorKind::PermissionDenied => (126, "permission denied".to_owned()),
                _ => (126, err.to_string()),
            };
            let line = format!("kai-init: {}: {why}\n", job.argv[0]);
            OpenOptions::new()
                .append(true)
                .open(out(STDERR_FILE))
                .and_then(|mut file| file.write_all(line.as_bytes()))
                .map_err(|err| format!("write stderr: {err}"))?;
            return Ok((Ending::Exited { exit_code: code }, 0));
        }
    };
    let pid = i32::try_from(child.id()).map_err(|err| err.to_string())?;
    drop(child);

    let timed_out = Arc::new(AtomicBool::new(false));
    std::thread::spawn({
        let timed_out = Arc::clone(&timed_out);
        let limit = Duration::from_secs(job.timeout_s);
        move || {
            std::thread::sleep(limit);
            timed_out.store(true, Ordering::SeqCst);
            // SAFETY: a plain system call.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    });

    let status = wait_for(pid)?;
    let ran = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let ending = if timed_out.load(Ordering::SeqCst) {
        Ending::Timeout
    } else if libc::WIFEXITED(status) {
        Ending::Exited {
            exit_code: libc::WEXITSTATUS(status),
        }
    } else {
        Ending::Signalled {
            signal: libc::WTERMSIG(status),
        }
    };
    // What the command left running, in its group or not, ends now.
    // SAFETY: plain system calls; -1 is every process but the init.
    unsafe { libc::kill(-1, libc::SIGKILL) };
    while wait_any(0).is_some() {}
    Ok((ending, ran))
}

/// Reaps every process that ends until `pid` does, and returns its wait
/// status.
fn wait_for(pid: i32) -> Result<i32, String> {
    loop {
        match wait_any(0) {
            Some((ended, status)) if ended == pid => return Ok(status),
            Some(_) => {}
            None => return Err(format!("the command, process {pid}, vanished")),
        }
    }
}

/// One ended child and its wait status; `None` when there is no child left.
fn wait_any(flags: i32) -> Option<(i32, i32)> {
    loop {
        let mut status = 0;
        // SAFETY: a plain system call with a pointer to a local.
        let pid = unsafe { libc::waitpid(-1, &mut status, flags) };
        if pid > 0 {
            return Some((pid, status));
        }
        if pid < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return None;
    }
}

/// Writes the status where the launcher reads it, and flushes it to the
/// drive.
fn write_status(status: &Status) -> io::Result<()> {
    let path = format!("{DRIVE}/{OUT_DIR}/{STATUS_FILE}");
    let _ = fs::remove_file(&path);
    let mut file = File::create(&path)?;
    serde_json::to_writer(&mut file, status)?;
    file.sync_all()
}

/// Step 7: unmounts what was mounted, innermost first, so that the job
/// drive is clean for the launcher, and resets the VM.
fn shutdown() -> ! {
    let mounted = MOUNTED
        .lock()
        .map(|mounted| mounted.clone())
        .unwrap_or_default();
    for target in mounted.iter().rev() {
        let Ok(target) = CString::new(target.as_str()) else {
            continue;
        };
        // SAFETY: plain system calls; a busy mount is detached.
        unsafe {
            if libc::umount2(target.as_ptr(), 0) != 0 {
                libc::umount2(target.as_ptr(), libc::MNT_DETACH);
            }
        }
    }
    // SAFETY: plain system calls. With `reboot=k` on the kernel's command
    // line the reset ends Firecracker.
    unsafe {
        libc::sync();
        libc::reboot(libc::RB_AUTOBOOT);
    }
    // Only when the reset failed; the kernel panics when PID 1 exits, and
    // `panic=1` resets the VM as well.
    std::process::exit(1)
}

fn mkdir(path: &str) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(format!("mkdir {path}: {err}")),
    }
}

/// The address family of vsock, which the libc crate names for glibc only.
const AF_VSOCK: libc::c_int = 40;

/// Brings the loopback device up, so that the forwarders can listen on
/// `127.0.0.1`.
fn loopback_up() -> io::Result<()> {
    /// `struct ifreq`, as far as `SIOCGIFFLAGS` and `SIOCSIFFLAGS` use it.
    #[repr(C)]
    struct IfReq {
        name: [u8; 16],
        flags: libc::c_short,
        pad: [u8; 22],
    }
    // SAFETY: a plain system call.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor was just made and is owned here alone.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let mut request = IfReq {
        name: [0; 16],
        flags: 0,
        pad: [0; 22],
    };
    request.name[..2].copy_from_slice(b"lo");
    // SAFETY: the request is a properly sized `ifreq` that lives across the
    // calls.
    unsafe {
        check(libc::ioctl(
            fd.as_raw_fd(),
            libc::SIOCGIFFLAGS as _,
            &mut request,
        ))?;
        request.flags |= libc::IFF_UP as libc::c_short;
        check(libc::ioctl(
            fd.as_raw_fd(),
            libc::SIOCSIFFLAGS as _,
            &mut request,
        ))
    }
}

/// Listens on `127.0.0.1:<port>` and passes each connection on to vsock
/// port `port` of the host, in threads of its own, for as long as the VM
/// runs.
fn forward(port: u16) -> io::Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            std::thread::spawn(move || match vsock_connect(port) {
                Ok(host) => relay(client, host),
                Err(err) => eprintln!("kai-init: port {port}: cannot reach the host: {err}"),
            });
        }
    });
    Ok(())
}

/// A stream to vsock port `port` of the host.
fn vsock_connect(port: u16) -> io::Result<std::os::unix::net::UnixStream> {
    // SAFETY: a plain system call.
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor was just made and is owned here alone.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    // SAFETY: an all-zero `sockaddr_vm` is valid; the fields that matter are
    // set below.
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    address.svm_family = AF_VSOCK as libc::sa_family_t;
    address.svm_cid = libc::VMADDR_CID_HOST;
    address.svm_port = u32::from(port);
    let size = std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;
    // SAFETY: the address is a `sockaddr_vm` of the size given.
    check(unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            size,
        )
    })?;
    // A vsock stream reads, writes and shuts down as any stream socket.
    Ok(std::os::unix::net::UnixStream::from(fd))
}

/// Copies both ways until each side is done, closing each direction as its
/// source ends.
fn relay(client: std::net::TcpStream, host: std::os::unix::net::UnixStream) {
    let (Ok(mut client_in), Ok(mut host_out)) = (client.try_clone(), host.try_clone()) else {
        return;
    };
    let upward = std::thread::spawn(move || {
        let _ = io::copy(&mut client_in, &mut host_out);
        let _ = host_out.shutdown(std::net::Shutdown::Write);
    });
    let (mut host_in, mut client_out) = (host, client);
    let _ = io::copy(&mut host_in, &mut client_out);
    let _ = client_out.shutdown(std::net::Shutdown::Write);
    let _ = upward.join();
}

/// Every mount made, in order, so that the shutdown undoes them innermost
/// first.
static MOUNTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// mount(2); a mount that succeeds is remembered for the shutdown.
fn mount(
    source: Option<&str>,
    target: &str,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let text = |text: &str| CString::new(OsStr::new(text).as_bytes()).map_err(io::Error::other);
    let source = source.map(text).transpose()?;
    let target = text(target)?;
    let fstype = fstype.map(text).transpose()?;
    let data = data.map(text).transpose()?;
    let pointer = |text: &Option<CString>| text.as_ref().map_or(std::ptr::null(), |t| t.as_ptr());
    // SAFETY: every pointer is to a NUL-terminated string that lives until
    // the call returns, or null where mount(2) takes null.
    let result = unsafe {
        libc::mount(
            pointer(&source),
            target.as_ptr(),
            pointer(&fstype),
            flags,
            pointer(&data).cast(),
        )
    };
    check(result)?;
    if let Ok(mut mounted) = MOUNTED.lock() {
        mounted.push(target.to_string_lossy().into_owned());
    }
    Ok(())
}

/// A system call's result as an `io::Result`.
fn check(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
