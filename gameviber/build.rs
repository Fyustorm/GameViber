use aya_build::{build_ebpf, Package, Toolchain};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SKIP_EBPF_BUILD=1: build without the eBPF toolchain (nightly + bpf-linker);
    // the ebpf source then fails at load time.
    // Other systems have no eBPF source (AGENTS.md, Platforms).
    println!("cargo:rerun-if-env-changed=SKIP_EBPF_BUILD");
    let linux = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux");
    if !linux || std::env::var_os("SKIP_EBPF_BUILD").is_some() {
        let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
        std::fs::write(out_dir.join("gameviber-ebpf"), [])?;
        return Ok(());
    }
    build_ebpf(
        [Package {
            name: "gameviber-ebpf",
            root_dir: "../gameviber-ebpf",
            no_default_features: false,
            features: &[],
        }],
        Toolchain::default(),
    )?;
    Ok(())
}
