//! Device recovery effects and their original failure codes.

use wire::OK;

pub const ATTACH: u8 = 1;
pub const STOP: u8 = 2;
pub const DETACH: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, env::WireCodes)]
#[wire(fallback = Bad)]
pub enum Fail {
    #[code(1)]
    Unknown,
    #[code(2)]
    Conflict,
    #[code(3)]
    Denied,
    #[code(4)]
    Unavailable,
    #[code(5)]
    Bad,
}
