use alloc::string::String;
use env::PieToken;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub class: String,
    pub line: u32,
    pub page: PieToken,
    pub door: PieToken,
}
