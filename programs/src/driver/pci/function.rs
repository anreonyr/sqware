//! Compact enumeration record for one present function.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Function {
    pub requester: u16,
    pub vendor: u16,
    pub device: u16,
    /// 24-bit class code in the low three bytes.
    pub class: u32,
}
