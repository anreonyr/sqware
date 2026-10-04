pub const MAGIC: [u8; 8] = *b"SQBOOT01";
pub const PAGE: usize = 4096;
pub const HEADER: usize = 32;
pub const RECORD: usize = 40;

#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub va: usize,
    pub pages: usize,
    pub data_pages: usize,
    pub payload: usize,
    pub flags: u64,
}

pub struct Capsule<'a> {
    bytes: &'a [u8],
    pub entry: usize,
    pub count: usize,
}

fn number(bytes: &[u8], at: usize, len: usize) -> Option<usize> {
    let mut raw = [0u8; 8];
    raw[..len].copy_from_slice(bytes.get(at..at.checked_add(len)?)?);
    usize::try_from(u64::from_le_bytes(raw)).ok()
}

impl<'a> Capsule<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.get(..8)? != MAGIC
            || number(bytes, 8, 2)? != 1
            || bytes.get(10..12)? != [12, 0]
            || number(bytes, 24, 8)? != bytes.len()
        {
            return None;
        }
        let count = number(bytes, 12, 4)?;
        let table = HEADER.checked_add(count.checked_mul(RECORD)?)?;
        if count == 0 || table > bytes.len() {
            return None;
        }
        let capsule = Self {
            bytes,
            entry: number(bytes, 16, 8)?,
            count,
        };
        let mut entry = false;
        for i in 0..count {
            let region = capsule.region(i)?;
            let size = region.pages.checked_mul(PAGE)?;
            let end = region.va.checked_add(size)?;
            let data = region.data_pages.checked_mul(PAGE)?;
            if region.pages == 0
                || region.va == 0
                || !region.va.is_multiple_of(PAGE)
                || region.data_pages > region.pages
                || region.flags & 2 == 0
                || region.flags & 12 == 12
                || region.flags & !14 != 0
            {
                return None;
            }
            if data == 0 {
                if region.payload != 0 {
                    return None;
                }
            } else if !region.payload.is_multiple_of(PAGE)
                || region.payload < table
                || region.payload.checked_add(data)? > bytes.len()
            {
                return None;
            }
            if region.flags & 8 != 0 {
                if region.data_pages != region.pages {
                    return None;
                }
                entry |= capsule.entry >= region.va && capsule.entry.checked_add(2)? <= end;
            }
            for j in 0..i {
                let other = capsule.region(j)?;
                if region.va < other.va.checked_add(other.pages.checked_mul(PAGE)?)?
                    && other.va < end
                {
                    return None;
                }
            }
        }
        (entry && capsule.entry.is_multiple_of(2)).then_some(capsule)
    }

    pub fn region(&self, index: usize) -> Option<Region> {
        if index >= self.count {
            return None;
        }
        let at = HEADER + index * RECORD;
        let permissions = number(self.bytes, at + 32, 4)?;
        if number(self.bytes, at + 36, 4)? != 0 || permissions & !7 != 0 {
            return None;
        }
        Some(Region {
            va: number(self.bytes, at, 8)?,
            pages: number(self.bytes, at + 8, 8)?,
            data_pages: number(self.bytes, at + 16, 8)?,
            payload: number(self.bytes, at + 24, 8)?,
            flags: (permissions << 1) as u64,
        })
    }
}
