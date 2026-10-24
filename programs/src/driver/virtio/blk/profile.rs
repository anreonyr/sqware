/// Device-protocol selection from a read-only configuration snapshot.
#[derive(Clone, Copy)]
pub struct Profile {
    pub key: u64,
    pub requester: u16,
    pub bar: u8,
    pub caps: (u32, u32, u32, u32),
}
