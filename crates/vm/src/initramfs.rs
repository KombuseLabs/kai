//! The initramfs: kai's init alone, as `/init`, in the `newc` format the
//! kernel unpacks. The kernel's own built-in initramfs, unpacked first,
//! brings `/dev/console`, so the init's lines reach the serial console.

use std::path::{Path, PathBuf};

use helpers::fs::write_whole;
use helpers::hash::{sha256_bytes, sha256_file};

use crate::Error;

/// kai's init, built for x86_64-unknown-linux-musl into kai by the build
/// script. Empty when that build failed; [`INIT_ERROR`] then says why.
pub const INIT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kai-init"));

/// Why [`INIT`] is empty; empty when it is not.
pub const INIT_ERROR: &str = env!("KAI_INIT_ERROR");

/// One file of a `newc` archive.
#[derive(Clone, Copy, Debug)]
pub struct Entry<'a> {
    /// Its path in the archive, without a leading `/`.
    pub name: &'a str,
    /// Its mode, file type included (`0o100755` for a program).
    pub mode: u32,
    /// Its bytes.
    pub data: &'a [u8],
}

/// `entries` as a `newc` archive, the format the kernel takes as an
/// initramfs: owned by root, dated 1970, so that the same entries give the
/// same bytes.
pub fn newc(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut archive = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let inode = u32::try_from(index + 1).unwrap_or(u32::MAX);
        header(&mut archive, inode, entry.mode, entry.name, entry.data);
    }
    header(&mut archive, 0, 0, "TRAILER!!!", &[]);
    archive
}

fn header(archive: &mut Vec<u8>, inode: u32, mode: u32, name: &str, data: &[u8]) {
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let name_size = u32::try_from(name.len() + 1).unwrap_or(u32::MAX);
    // ino, mode, uid, gid, nlink, mtime, filesize, devmajor, devminor,
    // rdevmajor, rdevminor, namesize, check
    let fields = [inode, mode, 0, 0, 1, 0, size, 0, 0, 0, 0, name_size, 0];
    archive.extend_from_slice(b"070701");
    for field in fields {
        archive.extend_from_slice(format!("{field:08X}").as_bytes());
    }
    archive.extend_from_slice(name.as_bytes());
    archive.push(0);
    pad(archive);
    archive.extend_from_slice(data);
    pad(archive);
}

/// Pads to a multiple of four bytes, as `newc` aligns names and data.
fn pad(archive: &mut Vec<u8>) {
    while !archive.len().is_multiple_of(4) {
        archive.push(0);
    }
}

/// The initramfs with [`INIT`] in the directory `cache`, under a name made
/// from its hash; returns its path and its SHA-256. It is written there
/// when it is missing, and again when the file of that name holds anything
/// else, so that the hash returned is always the file's.
pub fn initramfs(cache: &Path) -> Result<(PathBuf, String), Error> {
    if INIT.is_empty() {
        return Err(Error::NoInit(INIT_ERROR.to_owned()));
    }
    let archive = newc(&[Entry {
        name: "init",
        mode: 0o100_755,
        data: INIT,
    }]);
    let hash = sha256_bytes(&archive);
    let path = cache.join(format!("initramfs-{}.cpio", &hash[..16]));
    let io = |source| Error::Io {
        path: path.clone(),
        source,
    };
    let found = match sha256_file(&path) {
        Ok(found) => Some(found),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(io(source)),
    };
    if found.as_deref() != Some(hash.as_str()) {
        if found.is_some() {
            tracing::warn!(file = %path.display(), "not kai's initramfs; writing it again");
        }
        std::fs::create_dir_all(cache).map_err(io)?;
        write_whole(&path, &archive).map_err(io)?;
    }
    Ok((path, hash))
}

#[cfg(test)]
mod tests {
    use helpers::hash::sha256_file;

    use super::{Entry, INIT, INIT_ERROR, initramfs, newc};

    #[test]
    fn an_initramfs_in_the_cache_that_is_not_this_one_is_written_again() {
        let cache = std::env::temp_dir().join(format!("kai-vm-initramfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let (path, hash) = initramfs(&cache).unwrap();
        assert_eq!(sha256_file(&path).unwrap(), hash);

        // Cut short, or changed by anyone: the name says this archive, the
        // contents are another.
        std::fs::write(&path, b"cut short").unwrap();
        let (again, hash_again) = initramfs(&cache).unwrap();
        assert_eq!((&again, &hash_again), (&path, &hash));
        assert_eq!(sha256_file(&path).unwrap(), hash);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    /// A kai without its init builds, but runs no VM; CI must not miss it.
    #[test]
    fn the_init_is_built_into_kai() {
        assert!(!INIT.is_empty(), "{INIT_ERROR}");
        assert_eq!(&INIT[..4], b"\x7fELF");
    }

    #[test]
    fn newc_headers_name_sizes_and_alignment() {
        let archive = newc(&[Entry {
            name: "init",
            mode: 0o100_755,
            data: b"abcde",
        }]);
        let text = String::from_utf8_lossy(&archive[..110]);
        assert!(text.starts_with("070701"));
        // mode, filesize and namesize, as 8 hex digits each
        assert_eq!(&text[14..22], "000081ED");
        assert_eq!(&text[54..62], "00000005");
        assert_eq!(&text[94..102], "00000005");
        // header 110 + "init\0" 5 = 115, padded to 116; then 5 bytes of
        // data padded to 8
        assert_eq!(&archive[110..115], b"init\0");
        assert_eq!(&archive[116..121], b"abcde");
        assert_eq!(&archive[124..130], b"070701");
        assert!(archive.windows(10).any(|window| window == b"TRAILER!!!"));
        assert!(archive.len().is_multiple_of(4));
    }
}
