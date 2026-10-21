use crate::{PieToken, ReferenceRange, state};

#[path = "reference.rs"]
mod reference;
#[path = "view.rs"]
mod view;

pub use reference::Reference;
pub use view::View;

pub fn traverse(
    retention: PieToken,
    _range: ReferenceRange,
    physical: &mut [u64],
) -> abi::MemoryResult<()> {
    assert_eq!(retention, PieToken::mint(7));
    state::step("traverse").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    for (index, page) in physical.iter_mut().enumerate() {
        *page = ((index + 1) * abi::PAGE_SIZE) as u64;
    }
    Ok(())
}
