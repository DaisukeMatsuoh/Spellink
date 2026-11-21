//! The name service of ADR 0017: it gives each bridge a name under the LAN zone and answers DNS
//! for it, and it writes the TXT records that let the bridge get its own certificate.

pub mod dns;
pub mod store;
pub mod api;
