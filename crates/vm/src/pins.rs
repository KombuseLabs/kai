//! What kai runs VMs with, pinned: where each file comes from and its
//! SHA-256. A new version is a change to this file, and nothing else
//! decides which version kai runs.

/// One file kai downloads and keeps in its cache.
#[derive(Clone, Copy, Debug)]
pub struct Artefact {
    /// What it is, for people: `Firecracker v1.18.0`.
    pub name: &'static str,
    /// Its file name in the cache.
    pub file: &'static str,
    /// Where it is downloaded from: `https://`, or plain `http://` to this
    /// machine (tests).
    pub url: &'static str,
    /// SHA-256 of the download, in lower-case hex.
    pub download_sha256: &'static str,
    /// For a `.tgz`, the one member kept, as `tar` names it. `None` when the
    /// download is the file itself.
    pub member: Option<&'static str>,
    /// SHA-256 of the file in the cache: the member's, or the download's
    /// when there is no member.
    pub sha256: &'static str,
    /// Whether the file is a program (mode 0755; else 0644).
    pub executable: bool,
}

/// Firecracker v1.18.0, the program that runs a microVM, from its GitHub
/// release. The archive's hash is the one the release publishes beside it
/// (`firecracker-v1.18.0-x86_64.tgz.sha256.txt`); the program's is that of
/// the member unpacked from it.
pub const FIRECRACKER: Artefact = Artefact {
    name: "Firecracker v1.18.0",
    file: "firecracker-v1.18.0",
    url: "https://github.com/firecracker-microvm/firecracker/releases/download/v1.18.0/firecracker-v1.18.0-x86_64.tgz",
    download_sha256: "1ac8eec866dd5eea14efecf6323e22b23eea24054545338a5b34bc5bca6623cd",
    member: Some("release-v1.18.0-x86_64/firecracker-v1.18.0-x86_64"),
    sha256: "b33f46cd6d88834cf6a4f8d184f08f219809c636236bffd286d233a60602a2ba",
    executable: true,
};

/// The guest kernel, Linux 6.1.155 as an uncompressed `vmlinux`, built by
/// Firecracker's CI for its v1.15 release and kept in its public bucket.
/// No hash is published for it; the pin is the hash of the first download.
pub const KERNEL: Artefact = Artefact {
    name: "guest kernel 6.1.155",
    file: "vmlinux-6.1.155",
    url: "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.15/x86_64/vmlinux-6.1.155",
    download_sha256: "e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2",
    member: None,
    sha256: "e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2",
    executable: false,
};

/// Every file `kai vm fetch` downloads, in the order it does.
pub const ARTEFACTS: [Artefact; 2] = [FIRECRACKER, KERNEL];
