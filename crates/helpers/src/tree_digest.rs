//! Content identity of a tree, compatible with eval-reporter.
//! Sorted depth-first paths, executable bits, file hashes and link targets; never follows links.
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::hash::sha256_bytes;

fn sha256_hex(bytes: &[u8]) -> String {
    sha256_bytes(bytes)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            out.push(
                path.strip_prefix(root)
                    .expect("below the root")
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

/// The tree digest of a directory, lowercase hex without a prefix.
pub fn tree(dir: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let mut manifest = Vec::new();
    for rel in files {
        let path = dir.join(&rel);
        let meta = fs::symlink_metadata(&path)?;
        manifest.extend_from_slice(rel.as_os_str().as_bytes());
        manifest.push(0);
        if meta.file_type().is_symlink() {
            manifest.extend_from_slice(b"link:");
            for &byte in fs::read_link(&path)?.as_os_str().as_bytes() {
                match byte {
                    b'\\' => manifest.extend_from_slice(b"\\\\"),
                    b'\n' => manifest.extend_from_slice(b"\\n"),
                    _ => manifest.push(byte),
                }
            }
        } else {
            let exec = meta.permissions().mode() & 0o111 != 0;
            manifest.extend_from_slice(if exec { b"x:" } else { b"-:" });
            manifest.extend_from_slice(sha256_hex(&fs::read(&path)?).as_bytes());
        }
        manifest.push(b'\n');
    }
    Ok(sha256_hex(&manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(build: impl FnOnce(&Path)) -> String {
        let dir = crate::testing::TestDir::new("tree-digest");
        build(&dir);
        tree(&dir).unwrap()
    }

    // Shared vectors with eval-reporter's tree digest.
    #[test]
    fn agrees_with_kais_vectors() {
        assert_eq!(
            tree_of(|d| fs::write(d.join("a.txt"), "hello\n").unwrap()),
            "2173f24c0000669c3eacc15203b94ab9ee39be281c7dabb1f3adabff832fc2af"
        );
        assert_eq!(
            tree_of(|d| {
                let p = d.join("run.sh");
                fs::write(&p, "#!/bin/sh\necho hi\n").unwrap();
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            }),
            "330f57d15f467157d3ea14360185dcc19e7733fdfe1befa2bd2a4f3af2298eb5"
        );
        assert_eq!(
            tree_of(|d| {
                fs::create_dir_all(d.join("src/lib")).unwrap();
                fs::write(d.join("src/lib/mod.txt"), "nested").unwrap();
            }),
            "790b6e2eb8c70dd16b901c897e0f6ef7593bc24c9a78ff9b6e0a0ad5e45b164d"
        );
        assert_eq!(
            tree_of(|d| fs::write(d.join("empty.txt"), "").unwrap()),
            "3ebad82cdbf4e756fe72d7dd5c21f3d8ca6cb4525b693270533c5baf6e1dd048"
        );
        assert_eq!(
            tree_of(|d| fs::create_dir_all(d.join("x/y")).unwrap()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            tree_of(|d| std::os::unix::fs::symlink("target/x", d.join("l")).unwrap()),
            "b7c41d4372075afcd4a5721bc3fa851104a60af4274770671926a90803fd286e"
        );
    }

    #[test]
    fn names_sort_per_directory_by_bytes() {
        // `a/b` comes before `a-c`: entries of the root are `a` and `a-c`,
        // and `a` < `a-c`
        let one = tree_of(|d| {
            fs::create_dir(d.join("a")).unwrap();
            fs::write(d.join("a/b"), "1").unwrap();
            fs::write(d.join("a-c"), "2").unwrap();
        });
        let manifest = format!("a/b\0-:{}\na-c\0-:{}\n", sha256_hex(b"1"), sha256_hex(b"2"));
        assert_eq!(one, sha256_hex(manifest.as_bytes()));
    }
}
