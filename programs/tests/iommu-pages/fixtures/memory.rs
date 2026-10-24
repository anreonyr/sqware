use crate::{PAGE_SIZE, PieToken, ReferenceRange, state};

#[path = "reference.rs"]
mod reference;
#[path = "view.rs"]
mod view;

pub use reference::Reference;
pub use view::View;

pub fn traverse(
    retention: PieToken,
    range: ReferenceRange,
    physical: &mut [u64],
) -> abi::MemoryResult<()> {
    assert_eq!(retention, PieToken::mint(7));
    state::step("traverse").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    if range.reference.get() == 1 {
        for (index, page) in physical.iter_mut().enumerate() {
            // Synthetic physical pages are numbered from one; these are not host pointers.
            *page = ((index + 1) * PAGE_SIZE) as u64;
        }
        return Ok(());
    }
    let (base, pages) = state::source_of(range.reference.get())
        .ok_or(abi::make_fail(abi::MemoryFail::Denied))?;
    if range.offset != 0 || range.count != physical.len() || range.count > pages {
        return Err(abi::make_fail(abi::MemoryFail::Denied));
    }
    for (index, page) in physical.iter_mut().enumerate() {
        *page = base + (index * PAGE_SIZE) as u64;
    }
    Ok(())
}

pub fn acquire(
    source: crate::AcquireSource,
    retention: PieToken,
) -> abi::MemoryResult<crate::FlatReference> {
    assert_eq!(retention, PieToken::mint(7));
    let crate::AcquireSource::Source {
        token,
        offset,
        count,
    } = source
    else {
        return Err(abi::make_fail(abi::MemoryFail::Denied));
    };
    assert_eq!(offset, 0);
    state::step("acquire data").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    let handle = state::adopt(token.get(), count)
        .ok_or(abi::make_fail(abi::MemoryFail::Denied))?;
    Ok(crate::FlatReference::from_bytes(&handle.to_le_bytes()).unwrap())
}

pub fn release(reference: crate::FlatReference) -> abi::MemoryResult<()> {
    state::step("release data").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    if !state::release_reference(reference.get()) {
        return Err(abi::make_fail(abi::MemoryFail::Denied));
    }
    Ok(())
}
