pub(crate) fn entry_image(blob: &[u8]) -> Option<&[u8]> {
    let off = u32le(blob, 0)? as usize;
    let len = u32le(blob, 4)? as usize;
    blob.get(off..off.checked_add(len)?)
}

fn u32le(blob: &[u8], at: usize) -> Option<u32> {
    let s = blob.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
