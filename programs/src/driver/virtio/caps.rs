//! Layout metadata parsed from an exact read-only PCI configuration snapshot.

use super::registers::{CAP_COMMON, CAP_DEVICE, CAP_NOTIFY, CAP_VENDOR, CONFIG_CAPABILITY, CONFIG_LEN};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caps {
    pub(super) common: u32,
    pub(super) notify: u32,
    pub(super) multiplier: u32,
    pub(super) device: u32,
}

impl Caps {
    pub const fn from_parts(common: u32, notify: u32, multiplier: u32, device: u32, span: usize) -> Option<Self> {
        if !fits(common, 56, span) || !fits(notify, 2, span) || !fits(device, 8, span)
            || common == notify || notify == device || common == device
            || common % 4 != 0 || notify % 2 != 0 || device % 4 != 0
        { return None; }
        Some(Self { common, notify, multiplier, device })
    }

    pub const fn parts(&self) -> (u32, u32, u32, u32) {
        (self.common, self.notify, self.multiplier, self.device)
    }

    /// Region length is independent of its BAR-relative offset. Recipients
    /// check the returned offsets against their actual BAR mapping.
    pub fn read(snapshot: &[u8; CONFIG_LEN]) -> Result<(u8, Self), &'static str> {
        let mut at = usize::from(snapshot[CONFIG_CAPABILITY] & 0xFC);
        let mut visited = [false; CONFIG_LEN];
        let (mut bar, mut common, mut notify, mut device) = (None, None, None, None);
        let mut multiplier = 0;
        while at != 0 {
            if at < 0x40 || at + 2 > CONFIG_LEN { return Err("virtio: caps header"); }
            if visited[at] { return Err("virtio: caps loop"); }
            visited[at] = true;
            let next = usize::from(snapshot[at + 1] & 0xFC);
            if snapshot[at] == CAP_VENDOR {
                let header = snapshot.get(at..at + 4).ok_or("virtio: caps header")?;
                let length = usize::from(header[2]);
                let kind = header[3];
                if matches!(kind, CAP_COMMON | CAP_NOTIFY | CAP_DEVICE) {
                    let minimum = if kind == CAP_NOTIFY { 20 } else { 16 };
                    if length < minimum || at + length > CONFIG_LEN { return Err("virtio: caps length"); }
                    let owner = snapshot[at + 4];
                    if owner > 5 || bar.is_some_and(|prior| prior != owner) { return Err("virtio: caps bar"); }
                    bar = Some(owner);
                    let offset = read32(snapshot, at + 8)?;
                    let region_length = read32(snapshot, at + 12)?;
                    let used = match kind { CAP_COMMON => 56, CAP_NOTIFY => 2, _ => 8 };
                    if region_length < used || offset.checked_add(region_length).is_none() {
                        return Err("virtio: caps region");
                    }
                    let slot = match kind {
                        CAP_COMMON => &mut common,
                        CAP_NOTIFY => { multiplier = read32(snapshot, at + 16)?; &mut notify }
                        _ => &mut device,
                    };
                    if slot.replace(offset).is_some() { return Err("virtio: caps duplicate"); }
                }
            }
            at = next;
        }
        let caps = Self {
            common: common.ok_or("virtio: caps common")?, notify: notify.ok_or("virtio: caps notify")?,
            multiplier, device: device.ok_or("virtio: caps device")?,
        };
        if Self::from_parts(caps.common, caps.notify, caps.multiplier, caps.device, usize::MAX).is_none() {
            return Err("virtio: caps offsets");
        }
        Ok((bar.ok_or("virtio: caps bar")?, caps))
    }
}

const fn fits(offset: u32, length: usize, span: usize) -> bool {
    match (offset as usize).checked_add(length) { Some(end) => end <= span, None => false }
}

fn read32(snapshot: &[u8; CONFIG_LEN], at: usize) -> Result<u32, &'static str> {
    let bytes = snapshot.get(at..at + 4).ok_or("virtio: caps header")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
