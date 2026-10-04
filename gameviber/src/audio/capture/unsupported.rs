//! Systems without a sound capture backend yet: no application is heard.

use std::sync::mpsc;

use super::{Stream, Target};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub streams: Vec<Stream>,
}

impl Graph {
    pub fn read() -> Self {
        Self::default()
    }
}

pub struct Capture {
    pub target: Target,
    pub samples: mpsc::Receiver<Vec<f32>>,
}

impl Capture {
    pub fn start(_target: Target) -> anyhow::Result<Self> {
        anyhow::bail!("not available on this system yet")
    }

    pub fn update(&mut self, _graph: &Graph, _target: &Target) -> bool {
        false
    }

    pub fn ended(&mut self) -> bool {
        true
    }
}
