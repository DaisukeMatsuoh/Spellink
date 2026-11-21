//! The Spellink bridge and relay (ADR 0018: the Rust port of the Go bridge).
//! One module per Go package; the state of the port is in docs/notes/progress/rust-port.md.

pub mod config;
pub mod devices;
pub mod glyphs;
pub mod handwriting;
pub mod history;
pub mod lantls;
pub mod localguard;
pub mod layout;
pub mod measure;
pub mod protocol;
pub mod provider;
pub mod random;
pub mod securechan;
pub mod util;
pub mod relay;
pub mod server;
pub mod timestamp;
