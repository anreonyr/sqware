#[derive(Debug, PartialEq, Eq)]
pub enum Blocked {
    Resume,
    Switch(Option<usize>),
    Quit(usize),
}
