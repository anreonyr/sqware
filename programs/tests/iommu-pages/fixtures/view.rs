use crate::state;

pub struct View {
    memory: Box<[u64]>,
    live: bool,
}

impl View {
    pub fn new(pages: usize) -> Self {
        let view = Self {
            memory: vec![u64::MAX; pages * abi::PAGE_SIZE / 8].into_boxed_slice(),
            live: true,
        };
        state::STATE.with_borrow_mut(|state| state.view_address = view.address());
        view
    }

    pub fn address(&self) -> usize {
        assert!(self.live, "access after View close");
        self.memory.as_ptr() as usize
    }

    pub fn close(&mut self) -> abi::MemoryResult<()> {
        assert!(self.live, "duplicate View close");
        state::step("close view").map_err(|()| abi::make_fail(abi::MemoryFail::Busy))?;
        state::STATE.with_borrow_mut(|state| {
            state.views -= 1;
            state.view_address = 0;
        });
        self.live = false;
        Ok(())
    }
}
