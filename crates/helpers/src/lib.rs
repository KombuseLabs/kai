//! Infrastructure the other crates share. It holds nothing about what kai
//! does: [`logging`] decides where log lines go, [`config`] finds and reads
//! settings files.

pub mod config;
pub mod logging;
