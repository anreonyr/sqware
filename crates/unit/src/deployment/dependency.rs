use super::Bootstrap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dependency {
    Unit(&'static str),
    Target(&'static str),
    Bootstrap(Bootstrap),
}
