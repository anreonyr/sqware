//! Stable identity channel and grant marks.

use env::{Mark, marks::Definition};

pub const BACK: Mark = Mark::of("identity-back");
pub const DECLARATIONS: [Definition; 1] = [Definition { name: "identity-back", mark: BACK }];

const _: () = {
    assert!(BACK.get() != Mark::NONE.get(), "reserved empty mark");
    assert!(env::marks::conflict(&[&DECLARATIONS]).is_none(), "duplicate mark");
};
