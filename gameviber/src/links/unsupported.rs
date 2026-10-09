//! Systems without a single instance yet: every start runs.

use super::Claim;

pub struct Instance;

pub fn claim(_link: Option<&str>) -> Claim {
    Claim::First(Instance)
}

impl Instance {
    pub fn listen(self, _on_link: impl Fn(String) + Send + 'static) {}
}
