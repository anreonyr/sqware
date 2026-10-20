use super::boundary::Boundary;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Before(Boundary),
    Reply(Boundary),
}
