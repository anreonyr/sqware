use crate::{state, PieToken};

pub const COMMAND: u16 = 0x04;
pub const COMMAND_MEMORY: u16 = 1 << 1;
pub const COMMAND_MASTER: u16 = 1 << 2;
pub const CAPABILITY: u16 = 0x34;
pub const CAP_VENDOR: u8 = 0x09;

const BAR: u16 = 0x10;

pub struct Ecam {
    base: usize,
    requester: u16,
}

impl Ecam {
    pub const fn new(base: usize, requester: u16) -> Self {
        Self { base, requester }
    }

    pub const fn requester(&self) -> u16 {
        self.requester
    }

    fn at(&self, register: u16) -> usize {
        self.base + usize::from(self.requester) * 4096 + register as usize
    }

    pub fn read8(&self, register: u16) -> u8 {
        state::read(self.at(register))
    }

    pub fn read16(&self, register: u16) -> u16 {
        let at = self.at(register);
        u16::from_le_bytes([state::read(at), state::read(at + 1)])
    }

    pub fn read32(&self, register: u16) -> u32 {
        let at = self.at(register);
        u32::from_le_bytes([
            state::read(at),
            state::read(at + 1),
            state::read(at + 2),
            state::read(at + 3),
        ])
    }

    pub fn write16(&self, register: u16, value: u16) {
        let at = self.at(register);
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            state::write(at + offset, byte);
        }
    }

    pub fn write32(&self, register: u16, value: u32) {
        let at = self.at(register);
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            state::write(at + offset, byte);
        }
    }

    pub fn command_or(&self, set: u16) {
        self.write16(COMMAND, self.read16(COMMAND) | set);
    }

    pub fn command_and(&self, clear: u16) {
        self.write16(COMMAND, self.read16(COMMAND) & !clear);
    }

    pub fn bar_size(&self, index: u8) -> Option<(u64, bool)> {
        matches!(index, 0 | 4).then_some((state::bar_size(self.requester), true))
    }

    pub fn place_bar(&self, index: u8, address: u64, wide: bool) {
        assert!(matches!(index, 0 | 4));
        assert!(wide);
        state::place_bar(self.requester, address);
    }
}

pub fn find(_: usize, _: u16, _: &[u16]) -> Option<u16> {
    Some(state::requester())
}

pub fn _unused(_: PieToken) {}
