//! `cfg(sessions)`: the detachable sessions, on Unix. Elsewhere, and on
//! Unix with `--cfg legume_tui_plain` (to check that path here), every
//! session runs plain.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(sessions)");
    println!("cargo::rustc-check-cfg=cfg(legume_tui_plain)");
    let unix = std::env::var_os("CARGO_CFG_UNIX").is_some();
    let plain = std::env::var_os("CARGO_CFG_LEGUME_TUI_PLAIN").is_some();
    if unix && !plain {
        println!("cargo::rustc-cfg=sessions");
    }
}
