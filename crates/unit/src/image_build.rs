/// One executable image and the real Cargo target that produces it.
///
/// All coordinates are caller-supplied facts. `manifest` is relative to the
/// caller's build root; `image`, `package`, `bin`, `target` and `profile` are
/// passed to Cargo unchanged.
#[derive(Clone, Copy)]
pub struct ImageBuild {
    pub image: &'static str,
    pub manifest: &'static str,
    pub package: &'static str,
    pub bin: &'static str,
    pub features: &'static [&'static str],
    pub target: &'static str,
    pub profile: &'static str,
    pub default_features: bool,
}
