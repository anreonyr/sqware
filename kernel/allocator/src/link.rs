use core::ptr::NonNull;

pub(crate) struct Link {
    pub(crate) prev: Option<NonNull<Link>>,
    pub(crate) next: Option<NonNull<Link>>,
}

impl Link {
    pub(crate) fn new(prev: Option<NonNull<Link>>, next: Option<NonNull<Link>>) -> Self {
        Self { prev, next }
    }
}
