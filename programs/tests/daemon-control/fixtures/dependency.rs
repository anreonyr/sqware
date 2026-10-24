#[derive(Clone, Copy)]
pub enum Dependency {
    Unit(&'static str),
    Target(&'static str),
    Bootstrap(()),
}
