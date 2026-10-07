//! Stable marks used by Operator sessions and bootstrap messages.

use env::{Mark, marks::Definition};

pub const LINK: &str = "operator";

macro_rules! channels {
    ($($symbol:ident => $name:expr;)+) => {
        $(pub const $symbol: Mark = Mark::of($name);)+
        pub const DECLARATIONS: [Definition; [$(stringify!($symbol)),+].len()] = [
            $(Definition { name: $name, mark: $symbol },)+
        ];
    };
}

channels! {
    ASK_MARK => "operator-ask";
    TIP_MARK => "tip";
    TIP_BACK => "operator-tip-back";
    LINK_MARK => LINK;
    WATCH_MARK => "operator-watch";
}

const _: () = assert!(env::marks::conflict(&[&DECLARATIONS]).is_none());
