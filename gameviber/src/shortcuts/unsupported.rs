//! Systems without a shortcuts backend yet.

use super::{Action, Status};

pub struct Shortcuts;

impl Shortcuts {
    pub fn start() -> Self {
        Self
    }

    pub fn poll(&self) -> Vec<Action> {
        Vec::new()
    }

    pub fn status(&self) -> Status {
        Status::Failed("not available on this system yet".into())
    }

    pub fn configure(&self) {}
}
