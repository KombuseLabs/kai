//! Infrastructure the other crates share. It holds nothing about what kai
//! does: [`logging`] decides where log lines go, [`config`] finds and reads
//! settings files, [`http`] starts HTTP clients, [`process`] runs programs,
//! [`fs`] puts files in place whole, [`hash`] is SHA-256, [`lines`] bounds
//! local protocol frames without losing partial input on cancellation.

pub mod config;
pub mod fs;
pub mod hash;
pub mod http;
pub mod lines;
pub mod logging;
pub mod process;
