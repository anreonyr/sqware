#![allow(dead_code, unused_imports)]

extern crate alloc;

mod fixture;

mod core {
    pub mod work {
        pub use crate::fixture::commit;
        pub mod unit {
            pub mod copies {
                pub fn check_block_heldout() {}
            }
        }
    }
}

mod messenger {
    pub use crate::fixture::take_doomed;

    mod wait {
        use crate::fixture::*;

        mod block {
            include!("../../src/core/work/room/messenger/wait/block.rs");
        }

        #[cfg(test)]
        mod tests {
            include!("cases.rs");
        }
    }
}
