// Puts the plugin package (Spellink.snplg) into the binary, so that `device add` needs no setup
// (docs/notes/research/2026-11-20-bridge-app-draft.md). Which file is named by the environment
// variable SPELLINK_BASE_PACKAGE, never by a default path: a package built on a developer's machine
// carries that machine's bridge address and token (the plugin bakes plugin/src/bridges.local.json
// in), and must not end up inside a binary that is handed out. `scripts/build-with-plugin.sh`
// builds a clean package and sets the variable. Without it the binary has no package, and the
// config's pairing.pluginPackage names one.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=SPELLINK_BASE_PACKAGE");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("base.snplg");
    let bytes = match std::env::var_os("SPELLINK_BASE_PACKAGE").filter(|v| !v.is_empty()) {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("SPELLINK_BASE_PACKAGE={}: {e}", PathBuf::from(&path).display()));
            assert!(bytes.starts_with(b"PK"), "SPELLINK_BASE_PACKAGE is not a zip (.snplg)");
            bytes
        }
        None => Vec::new(),
    };
    std::fs::write(&out, bytes).expect("writing the package into OUT_DIR");
}
