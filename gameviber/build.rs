use aya_build::{build_ebpf, Package, Toolchain};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SKIP_EBPF_BUILD=1: build without the eBPF toolchain (nightly + bpf-linker);
    // the ebpf source then fails at load time.
    // Other systems have no eBPF source (AGENTS.md, Platforms).
    println!("cargo:rerun-if-env-changed=SKIP_EBPF_BUILD");
    // Windows: ONNX Runtime's prebuilt library links DirectML, which GameViber
    // never uses; loaded on first use only, it is not needed to start (Windows
    // before 1903, Wine).
    if std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:DirectML.dll");
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
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
