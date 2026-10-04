//! Systems without a source backend yet: only the simulator and replays drive modes.

use super::{ActiveSource, EventSender, SourceOptions};
use crate::config::SourceChoice;

pub struct Sources;

impl Sources {
    pub fn new() -> Self {
        Self
    }

    pub fn start(&self, choice: SourceChoice, _opts: &SourceOptions, _tx: EventSender) -> anyhow::Result<Box<dyn ActiveSource>> {
        anyhow::bail!("the {choice:?} source is not available on this system")
    }

    pub fn shutdown(&self) {}
}
