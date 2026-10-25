#[derive(Clone, Copy)]
pub struct Target {
    pub name: &'static str,
    pub members: &'static [&'static str],
}
