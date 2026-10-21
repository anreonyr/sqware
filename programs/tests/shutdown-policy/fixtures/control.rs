use super::instance::Instance;

pub struct Control {
    pub services: usize,
    pub instances: Vec<Instance>,
}

impl Control {
    pub fn living_count(&self) -> usize {
        self.services
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    pub fn walking(&self) -> bool {
        true
    }
}
