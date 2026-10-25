use system_api::identity::Subject;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    Caller,
    Selected,
    Exact(Subject),
}
