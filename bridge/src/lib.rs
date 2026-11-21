//! The Spellink bridge and relay (ADR 0018: the Rust port of the Go bridge).
//! One module per Go package; the state of the port is in docs/notes/progress/rust-port.md.

pub mod app;
pub mod cli;
pub mod config;
pub mod connector;
pub mod devicecli;
pub mod devices;
pub mod glyphbuild;
pub mod glyphs;
pub mod handwriting;
pub mod history;
pub mod lantls;
pub mod layout;
pub mod localguard;
pub mod measure;
pub mod names;
pub mod namesproto;
pub mod probe;
pub mod protocol;
pub mod provider;
pub mod random;
pub mod relay;
pub mod securechan;
pub mod server;
pub mod service;
pub mod timestamp;
pub mod ui;
pub mod util;
