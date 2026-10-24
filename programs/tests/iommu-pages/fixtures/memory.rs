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
        // Synthetic physical pages are numbered from one; these are not host pointers.
        *page = ((index + 1) * abi::PAGE_SIZE) as u64;
    }
    Ok(())
}

pub fn acquire(source: crate::AcquireSource, retention: PieToken) -> abi::MemoryResult<crate::FlatReference> {
    assert!(matches!(source, crate::AcquireSource::Source { token, offset: 0, count: 2 }
        if token == PieToken::mint(42)));
    assert_eq!(retention, PieToken::mint(7));
    state::step("acquire data").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    state::STATE.with_borrow_mut(|state| {
        assert!(!state.data_reference, "duplicate data reference acquisition");
        state.references += 1;
        state.data_reference = true;
    });
    Ok(crate::FlatReference::from_bytes(&2usize.to_le_bytes()).unwrap())
}

pub fn release(reference: crate::FlatReference) -> abi::MemoryResult<()> {
    assert_eq!(reference, crate::FlatReference::from_bytes(&2usize.to_le_bytes()).unwrap());
    state::step("release data").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
    state::STATE.with_borrow_mut(|state| {
        assert!(state.data_reference, "duplicate data reference release");
        state.references -= 1;
        state.data_reference = false;
    });
    Ok(())
}
