//! Provider-owned channel marks and their registry.
use env::{Mark, marks::Definition};
pub const LANE: &str = "line";

macro_rules! channels {
    ($($symbol:ident => $name:expr;)+) => {
        $(pub const $symbol: Mark = Mark::of($name);)+
        pub const DECLARATIONS: &[Definition] = &[
            $(Definition { name: $name, mark: $symbol },)+
        ];
    };
}
channels! {
    ENTRY_MARK => "entry";
    LINE_BACK => "line-back";
    LINE_MARK => LANE;
}
const _: () = assert!(env::marks::conflict(&[DECLARATIONS]).is_none());
