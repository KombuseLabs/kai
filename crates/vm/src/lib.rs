//! microVMs on Firecracker: one command in a fresh VM ([`run()`]), images
//! from the recipes kai ships ([`image`]), the pinned Firecracker and guest
//! kernel ([`fetch()`]), kai's init inside every VM ([`initramfs`]), and
//! what running a VM needs on this machine ([`doctor`]).
//!
//! - **One run, one VM, one directory.** Nothing a run did is there for the
//!   next one. The guest has no network device. What comes back from it
//!   (the command's output, its status, the console) is read as data.
//! - **Pinned.** [`ARTEFACTS`] names each file by its URL and SHA-256. A
//!   download that does not match is refused and leaves no file behind.
//! - **Downloads only when a person asks.** [`fetch()`] is what
//!   `kai vm fetch` calls; nothing else here reaches the network.
//! - **Async** where it waits on the network or on a VM. The program runs
//!   the runtime; this crate never starts one.

pub mod doctor;
mod error;
mod fetch;
pub mod image;
pub mod initramfs;
mod pins;
mod run;

pub use error::Error;
pub use fetch::{Fetched, fetch};
pub use pins::{ARTEFACTS, Artefact, FIRECRACKER, KERNEL};
pub use run::{
    DEFAULT_CPUS, DEFAULT_CWD, DEFAULT_MEMORY_MIB, DEFAULT_PATH, DEFAULT_TIMEOUT_S, GRACE,
    ImageRecord, JobFile, Launch, Origin, Outcome, Port, PortTarget, ProxyRecord, RESULT_FORMAT,
    RunResult, RunSpec, new_run_dir, run,
};
