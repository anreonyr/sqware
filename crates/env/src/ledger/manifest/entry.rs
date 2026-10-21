use crate::wire::program_kind::ProgramKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry<'a> {
    pub kind: ProgramKind,
    pub name: &'a str,
    pub elf: &'a [u8],
}
