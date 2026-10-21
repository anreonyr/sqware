#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    Missing,
    Channel,
    Frame,
    Source,
}
