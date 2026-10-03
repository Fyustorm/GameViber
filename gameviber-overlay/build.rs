//! Compiles the overlay shaders (WGSL) to SPIR-V with naga, so building needs
//! no Vulkan SDK.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/overlay.wgsl");
    let source = std::fs::read_to_string("src/overlay.wgsl")?;
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| e.emit_to_string(&source))?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::IMMEDIATES)
        .validate(&module)?;
    let words = naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)?;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR")?).join("overlay.spv"), bytes)?;
    Ok(())
}
