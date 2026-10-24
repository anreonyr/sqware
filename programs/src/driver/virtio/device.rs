//! Live virtio-pci device registers. Configuration parsing stays in `caps`.

use crate::driver::shared::order;

use super::caps::Caps;
use super::registers::*;

pub struct Virtio {
    bar: usize,
    caps: Caps,
}

impl Virtio {
    pub fn open(bar: usize, caps: &Caps) -> Result<Self, &'static str> {
        let mut virtio = Self { bar, caps: *caps };
        virtio.write8(STATUS, 0);
        virtio.write8(STATUS, STATUS_ACKNOWLEDGE);
        virtio.write8(STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        virtio.write32(FEATURE_SELECT_DEVICE, 1);
        let offered = virtio.read32(FEATURE_DEVICE);
        if offered & FEATURE_VERSION_1 == 0 {
            return Err("pci: virtio version 1");
        }
        virtio.write32(FEATURE_SELECT_DRIVER, 0);
        virtio.write32(FEATURE_DRIVER, 0);
        virtio.write32(FEATURE_SELECT_DRIVER, 1);
        virtio.write32(
            FEATURE_DRIVER,
            offered & (FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM),
        );
        virtio.write8(
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        if virtio.read8(STATUS) & STATUS_FEATURES_OK == 0 {
            return Err("pci: virtio features rejected");
        }
        Ok(virtio)
    }

    pub fn setup_queue(&mut self, desc: u64, avail: u64, used: u64) -> Result<u16, &'static str> {
        self.write16(QUEUE_SELECT, 0);
        let size = self.read16(QUEUE_SIZE);
        if size == 0 || !size.is_power_of_two() {
            return Err("pci: virtio queue size");
        }
        self.write16(QUEUE_SIZE, size);
        self.write16(QUEUE_MSIX, NO_VECTOR);
        self.write32(QUEUE_DESC_LOW, desc as u32);
        self.write32(QUEUE_DESC_HIGH, (desc >> 32) as u32);
        self.write32(QUEUE_AVAIL_LOW, avail as u32);
        self.write32(QUEUE_AVAIL_HIGH, (avail >> 32) as u32);
        self.write32(QUEUE_USED_LOW, used as u32);
        self.write32(QUEUE_USED_HIGH, (used >> 32) as u32);
        self.write16(QUEUE_ENABLE, 1);
        self.write8(
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
        if self.read16(QUEUE_ENABLE) != 1 {
            return Err("pci: virtio queue did not come up");
        }
        Ok(size)
    }

    pub fn status(&self) -> u8 {
        self.read8(STATUS)
    }

    pub fn notify(&self) {
        ring(self.bar, &self.caps, 0);
    }

    fn read8(&self, register: u16) -> u8 {
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u8,
            )
        }
    }

    fn read16(&self, register: u16) -> u16 {
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u16,
            )
        }
    }

    fn read32(&self, register: u16) -> u32 {
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u32,
            )
        }
    }

    fn write8(&mut self, register: u16, value: u8) {
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u8,
                value,
            )
        }
    }

    fn write16(&mut self, register: u16, value: u16) {
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u16,
                value,
            )
        }
    }

    fn write32(&mut self, register: u16, value: u32) {
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u32,
                value,
            )
        }
    }
}

pub fn reset(bar: usize, caps: &Caps) -> bool {
    write(bar, caps, STATUS, 0, 1);
    status(bar, caps) == 0 && queue_enable(bar, caps) == 0
}

pub fn status(bar: usize, caps: &Caps) -> u8 {
    read(bar, caps, STATUS, 1) as u8
}

pub fn queue_enable(bar: usize, caps: &Caps) -> u16 {
    read(bar, caps, QUEUE_ENABLE, 2) as u16
}

fn write(bar: usize, caps: &Caps, register: u16, value: u32, width: usize) {
    let at = (bar + caps.common as usize + register as usize) as *mut u8;
    unsafe {
        match width {
            1 => core::ptr::write_volatile(at, value as u8),
            2 => core::ptr::write_volatile(at as *mut u16, value as u16),
            _ => core::ptr::write_volatile(at as *mut u32, value),
        }
    }
}

fn read(bar: usize, caps: &Caps, register: u16, width: usize) -> u32 {
    let at = (bar + caps.common as usize + register as usize) as *const u8;
    unsafe {
        match width {
            1 => u32::from(core::ptr::read_volatile(at)),
            2 => u32::from(core::ptr::read_volatile(at as *const u16)),
            _ => core::ptr::read_volatile(at as *const u32),
        }
    }
}

pub fn notify_offset(bar: usize, caps: &Caps) -> u16 {
    unsafe {
        core::ptr::read_volatile(
            (bar + caps.common as usize + QUEUE_NOTIFY_OFF as usize) as *const u16,
        )
    }
}

pub fn ring(bar: usize, caps: &Caps, queue: u16) {
    order::sync();
    let at =
        bar + caps.notify as usize + notify_offset(bar, caps) as usize * caps.multiplier as usize;
    unsafe { core::ptr::write_volatile(at as *mut u16, queue) };
    order::sync();
}
