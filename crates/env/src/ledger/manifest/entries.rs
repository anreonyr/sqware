use super::{Entry, HEAD, MAGIC, MAX_NAME, MAX_PROGRAMS, Malformed, PREAMBLE, read_one, u32le};

/// 带部署选择的清单游标；引导项始终按原清单下标读取，不受游标位置影响。
pub struct Entries<'a> {
    blob: &'a [u8],
    at: usize,
    left: usize,
    deployment: &'a str,
    entry_at: usize,
}

impl<'a> Entries<'a> {
    pub fn new(blob: &'a [u8]) -> Option<Self> {
        if blob.get(PREAMBLE..PREAMBLE + 4)? != MAGIC {
            return None;
        }
        let left = u32le(blob, PREAMBLE + 4)? as usize;
        let entry_at = u32le(blob, PREAMBLE + 8)? as usize;
        let len = u32le(blob, PREAMBLE + 12)? as usize;
        if left == 0 || left > MAX_PROGRAMS || entry_at >= left || len == 0 || len > MAX_NAME {
            return None;
        }
        let deployment = core::str::from_utf8(blob.get(HEAD..HEAD + len)?).ok()?;
        Some(Self {
            blob,
            at: HEAD + len,
            left,
            deployment,
            entry_at,
        })
    }

    pub fn deployment(&self) -> &'a str {
        self.deployment
    }

    pub fn entry(&self) -> Result<Entry<'a>, Malformed> {
        Self::new(self.blob).ok_or(Malformed)?
            .nth(self.entry_at).ok_or(Malformed)?
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<Entry<'a>, Malformed>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        Some(read_one(self.blob, &mut self.at))
    }
}
