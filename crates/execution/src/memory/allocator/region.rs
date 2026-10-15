use env::TaskId;

#[derive(Clone, Copy)]
pub(super) struct Region {
    pub(super) address: usize,
    pub(super) span: usize,
    pub(super) view: usize,
    pub(super) pending: Option<(TaskId, usize)>,
}
