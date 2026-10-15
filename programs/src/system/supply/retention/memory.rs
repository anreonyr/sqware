use env::FlatReference;

pub(super) struct Memory {
    pub id: usize,
    pub reference: FlatReference,
    pub published: bool,
}
