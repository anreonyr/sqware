use env::{Mark, PieToken};

use super::Ask;

/// A trusted-publisher query: namespace mark, resource mark and return token.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 25)]
pub struct PublisherAsk {
    pub op: u8,
    pub namespace: Mark,
    pub mark: Mark,
    pub back: PieToken,
}

const _: () = assert!(
    PublisherAsk::LEN == 1 + Mark::WIDTH + Mark::WIDTH + PieToken::WIDTH
);
const _: () = assert!(PublisherAsk::LEN <= Ask::LEN);
