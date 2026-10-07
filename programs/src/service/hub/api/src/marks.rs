use env::{Mark, marks::Definition};

macro_rules! channels {
    ($($symbol:ident = $value:literal;)+) => {
        $(pub const $symbol: Mark = Mark::of($value);)+
        pub const DECLARATIONS: &[Definition] = &[
            $(Definition { name: $value, mark: $symbol },)+
        ];
        const _: () = {
            assert!(env::marks::conflict(&[DECLARATIONS]).is_none());
            $(assert!($symbol.get() != Mark::NONE.get());)+
        };
    };
}

channels! {
    BACK_MARK = "hub-back";
    ALIVE_MARK = "hub-alive";
    ACTIVATE_ENTRY = "hub-activate";
    ACTIVATE_BACK = "hub-activate-back";
}
