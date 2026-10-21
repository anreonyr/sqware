pub struct Hold;

impl Hold {
    pub fn retains(&self) -> bool {
        true
    }

    pub fn window(&self) -> ((), std::num::NonZeroUsize) {
        ((), std::num::NonZeroUsize::new(6).unwrap())
    }
}
