//! Systems without an update backend yet: new versions are only announced.

use std::path::Path;

use super::Installation;

pub fn installation() -> Installation {
    Installation::Source
}

pub fn apply(_installation: &Installation, _file: &Path) -> anyhow::Result<()> {
    anyhow::bail!("not available on this system yet")
}

pub fn relaunch() -> anyhow::Error {
    anyhow::anyhow!("not available on this system yet")
}
