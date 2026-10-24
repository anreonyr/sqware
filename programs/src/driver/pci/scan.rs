//! Bounded generic enumeration over a mapped ECAM window.

use super::end::End;
use super::function::Function;
use super::page::Page;
use super::reader::{Reader, MAX_FUNCTIONS};

/// Maximum BDFs probed by one call. A quiet window returns `WorkLimit`, not a
/// false EOF; the caller advances with `Page::next`.
pub const WORK: u32 = 256;

pub fn scan(reader: &impl Reader, from: u32, out: &mut [Function]) -> Option<Page> {
    let limit = reader.limit();
    if limit > MAX_FUNCTIONS || from > limit {
        return None;
    }
    let mut next = from;
    let mut count = 0usize;
    let mut examined = 0u32;

    if next >= limit {
        return Some(Page {
            next: limit,
            count,
            end: End::Eof,
        });
    }
    if out.is_empty() {
        return Some(Page {
            next,
            count,
            end: End::Capacity,
        });
    }

    while next < limit && examined < WORK {
        if count == out.len() {
            return Some(Page {
                next,
                count,
                end: End::Capacity,
            });
        }
        let requester = next as u16;
        examined += 1;

        let vendor = read16(reader, requester, 0)?;
        if vendor == 0 || vendor == 0xFFFF {
            next += 1;
            continue;
        }
        let device = read16(reader, requester, 2)?;
        let class = class(reader, requester)?;

        out[count] = Function {
            requester,
            vendor,
            device,
            class,
        };
        count += 1;
        next += 1;
    }

    let end = if next >= limit {
        End::Eof
    } else if count == out.len() {
        End::Capacity
    } else {
        End::WorkLimit
    };
    Some(Page { next, count, end })
}

fn read16(reader: &impl Reader, requester: u16, register: u16) -> Option<u16> {
    Some(u16::from_le_bytes([
        reader.read8(requester, register)?,
        reader.read8(requester, register.checked_add(1)?)?,
    ]))
}

fn class(reader: &impl Reader, requester: u16) -> Option<u32> {
    Some(
        ((reader.read8(requester, 11)? as u32) << 16)
            | ((reader.read8(requester, 10)? as u32) << 8)
            | reader.read8(requester, 9)? as u32,
    )
}
