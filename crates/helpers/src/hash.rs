//! SHA-256, the one hash there is here: of bytes, of a file, or of bytes
//! that arrive in pieces, always as lower-case hex.
//!
//! ```
//! use helpers::hash::{Sha256, sha256_bytes};
//!
//! assert_eq!(
//!     sha256_bytes(b"abc"),
//!     "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
//! );
//! let mut pieces = Sha256::new();
//! pieces.update(b"a");
//! pieces.update(b"bc");
//! assert_eq!(pieces.finish(), sha256_bytes(b"abc"));
//! ```

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use ring::digest;

/// The SHA-256 of `bytes`.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, bytes).as_ref())
}

/// The SHA-256 of the file at `path`, read in pieces.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finish())
}

/// [`sha256_file`] on one of the runtime's blocking threads, so that a
/// large file holds up no other task.
pub async fn sha256_file_async(path: PathBuf) -> io::Result<String> {
    tokio::task::spawn_blocking(move || sha256_file(&path))
        .await
        .unwrap_or_else(|join| Err(io::Error::other(join)))
}

/// The SHA-256 of bytes that arrive in pieces, such as a download.
pub struct Sha256(digest::Context);

impl Sha256 {
    /// Nothing hashed yet.
    pub fn new() -> Sha256 {
        Sha256(digest::Context::new(&digest::SHA256))
    }

    /// Adds the next piece.
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The SHA-256 of all pieces.
    pub fn finish(self) -> String {
        hex(self.0.finish().as_ref())
    }
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn nothing_hashes_to_the_known_value() {
        assert_eq!(sha256_bytes(b""), EMPTY);
        assert_eq!(Sha256::default().finish(), EMPTY);
    }

    #[tokio::test]
    async fn a_file_hashes_as_its_bytes_and_a_missing_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("helpers-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // More than one buffer, and not a multiple of it.
        let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        let file = dir.join("file");
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(sha256_file(&file).unwrap(), sha256_bytes(&bytes));
        assert_eq!(
            sha256_file_async(file.clone()).await.unwrap(),
            sha256_bytes(&bytes)
        );
        let missing = sha256_file_async(dir.join("missing")).await.unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
