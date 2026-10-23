#[derive(Clone, Copy)]
pub(crate) struct Scene {
    pub root: &'static str,
    pub target: &'static str,
    pub controller: &'static str,
    pub request: env::Mark,
    pub done: &'static str,
    pub helper: bool,
}
