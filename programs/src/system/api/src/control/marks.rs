//! Stable marks used by Control sessions and publication services.

use env::{Mark, marks::Definition};

pub const LINK: &str = "control";

macro_rules! channels {
    ($($symbol:ident => $name:expr;)+) => {
        $(pub const $symbol: Mark = Mark::of($name);)+
        pub const DECLARATIONS: [Definition; [$(stringify!($symbol)),+].len()] = [
            $(Definition { name: $name, mark: $symbol },)+
        ];
    };
}

channels! {
    ASK_MARK => "control-ask";
    BACK => "control-back";
    LINK_MARK => LINK;
    ACCOUNT_ENTRY => "control-account";
    ACCOUNT_BACK => "control-account-back";
    PUBLICATION_ENTRY => "control-publication";
    PUBLICATION_BACK => "control-publication-back";
    IDENTITY_REF => "control-identity-ref";
}

const _: () = assert!(env::marks::conflict(&[&DECLARATIONS]).is_none());
