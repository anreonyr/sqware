use crate::{
    AcquireSource, FlatReference, PieToken, Placement, TeamId, ViewControl, memory::View, state,
};

pub struct Reference {
    live: bool,
}

impl Reference {
    pub fn acquire(source: AcquireSource, retention: PieToken) -> abi::MemoryResult<Self> {
        assert!(
            matches!(source, AcquireSource::Source { token, offset: 0, count }
            if token == PieToken::mint(11) && count > 0)
        );
        assert_eq!(retention, PieToken::mint(7));
        state::step("acquire").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
        state::STATE.with_borrow_mut(|state| state.references += 1);
        Ok(Self { live: true })
    }

    pub fn handle(&self) -> abi::MemoryResult<FlatReference> {
        assert!(self.live);
        state::step("handle").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
        Ok(FlatReference::from_bytes(&1usize.to_le_bytes()).unwrap())
    }

    pub fn attach(
        &self,
        _team: TeamId,
        offset: usize,
        count: usize,
        _placement: Placement,
        access: u64,
        _control: ViewControl,
    ) -> abi::MemoryResult<View> {
        assert!(self.live);
        assert_eq!(offset, 0);
        assert_eq!(access, 6);
        state::step("attach").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
        state::STATE.with_borrow_mut(|state| state.views += 1);
        Ok(View::new(count))
    }

    pub fn release(&mut self) -> abi::MemoryResult<()> {
        assert!(self.live, "duplicate reference release");
        state::step("release reference").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
        state::STATE.with_borrow_mut(|state| {
            assert_eq!(state.views, 0, "reference released before view");
            state.references -= 1;
        });
        self.live = false;
        Ok(())
    }
}
