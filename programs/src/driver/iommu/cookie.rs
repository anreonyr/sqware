/// Reserve nonzero completion values without wrapping or consuming a failed request.
pub(crate) fn reserve(next: &mut Option<u32>, count: u32) -> Option<u32> {
    let first = (*next).filter(|first| *first != 0)?;
    let last = first.checked_add(count.checked_sub(1)?)?;
    *next = last.checked_add(1);
    Some(first)
}
