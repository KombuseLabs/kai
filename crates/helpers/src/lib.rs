//! Infrastructure the other crates share. It holds nothing about what kai
//! does: [`logging`] decides where log lines go, [`config`] finds and reads
//! settings files, [`http`] starts HTTP clients, [`process`] runs programs.

pub mod config;
pub mod http;
pub mod logging;
pub mod process;
