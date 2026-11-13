//! Infrastructure the other crates share. It holds nothing about what kai
//! does: [`logging`] decides where log lines go, [`config`] finds and reads
//! settings files, [`http`] starts HTTP clients, [`process`] runs programs,
//! [`fs`] puts files in place whole, [`hash`] is SHA-256, [`canonical`] is
//! a JSON value's one text and its id, [`lines`] bounds local protocol
//! frames without losing partial input on cancellation. `testing`,
//! compiled only for tests (the feature `test-support`), gives a test a
//! directory of its own that is removed when the test ends.

pub mod canonical;
pub mod config;
pub mod fs;
pub mod hash;
pub mod http;
pub mod lines;
pub mod logging;
pub mod process;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
