#![no_std]

#[cfg(feature = "loud")]
pub const NOTE: &str = "link-dep-loud-note";
#[cfg(not(feature = "loud"))]
pub const NOTE: &str = "link-dep-plain-note";
