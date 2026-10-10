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
        // The icon Windows shows for gameviber.exe (Explorer, taskbar, links).
        let res = PathBuf::from(std::env::var("OUT_DIR")?).join("icon.res");
        std::fs::write(&res, icon_resource(&std::fs::read("icons/gameviber.ico")?)?)?;
        println!("cargo:rerun-if-changed=icons/gameviber.ico");
        println!("cargo:rustc-link-arg-bins={}", res.display());
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

/// A compiled resource file (.res, which the linker takes like an object)
/// holding an .ico's images as the program's icon: one RT_ICON per image and
/// the RT_GROUP_ICON listing them, number 1 (the first icon, `gameviber.exe,0`).
fn icon_resource(ico: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    fn u16_at(bytes: &[u8], at: usize) -> usize {
        u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize
    }
    fn u32_at(bytes: &[u8], at: usize) -> usize {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
    }
    // Each resource: its header (numbered type and name), then its data aligned on 4 bytes.
    fn resource(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
        res.extend((data.len() as u32).to_le_bytes());
        res.extend(32u32.to_le_bytes());
        for word in [0xffff, kind, 0xffff, id] {
            res.extend(word.to_le_bytes());
        }
        res.extend(0u32.to_le_bytes()); // data version
        res.extend(flags.to_le_bytes());
        res.extend(0x0409u16.to_le_bytes()); // language: en-US
        res.extend([0; 8]); // version, characteristics
        res.extend(data);
        res.resize(res.len().next_multiple_of(4), 0);
    }
    if ico.len() < 6 || u16_at(ico, 2) != 1 {
        return Err("icons/gameviber.ico is not an icon".into());
    }
    let count = u16_at(ico, 4);
    // The empty resource every .res starts with.
    let mut res = vec![0, 0, 0, 0, 32, 0, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0];
    res.resize(32, 0);
    let mut group = ico[..6].to_vec();
    for i in 0..count {
        let entry = ico.get(6 + 16 * i..22 + 16 * i).ok_or("icons/gameviber.ico is cut")?;
        let (size, offset) = (u32_at(entry, 8), u32_at(entry, 12));
        let image = ico.get(offset..offset + size).ok_or("icons/gameviber.ico is cut")?;
        let id = i as u16 + 1;
        resource(&mut res, RT_ICON, id, 0x1010, image);
        // The group's entry: the directory's, the image's id instead of its offset.
        group.extend(&entry[..12]);
        group.extend(id.to_le_bytes());
    }
    resource(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);
    Ok(res)
}
