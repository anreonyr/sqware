pub const DDT: usize = 0;
pub const COMMANDS: usize = 1;
pub const FAULTS: usize = 2;
pub const FENCE: usize = 3;
pub const PAGES: usize = 4 + 3 * super::context::CAPACITY;

pub fn tables(requester: u16) -> Option<(usize, usize, usize)> {
    super::context::offset(requester)?;
    let root = 4 + 3 * usize::from(requester);
    Some((root, root + 1, root + 2))
}
