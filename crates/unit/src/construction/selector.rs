#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selector {
    Unit(&'static str),
    Image,
}
