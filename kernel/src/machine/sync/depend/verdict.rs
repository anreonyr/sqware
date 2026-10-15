//! 锁序判据：只做判断，不取锁、不分配、不 panic。
//!
//! 取锁路径拿它做决定，判错才交给 `report`；自检也直接喂合成持有集核这几条规则。

use super::level::Level;

/// 普通获取的判据：不许递归（同一把锁再取），且新锁级必须严格大于已持最大级。
pub(crate) fn strict(
    held: impl IntoIterator<Item = (usize, Option<Level>)>,
    addr: usize,
    level: Option<Level>,
) -> Result<(), &'static str> {
    let mut max: Option<Level> = None;
    for (at, held_level) in held {
        if at == addr {
            return Err("recursive acquisition");
        }
        if let Some(held_level) = held_level {
            max = Some(max.map_or(held_level, |max| max.max(held_level)));
        }
    }
    match (level, max) {
        (Some(level), Some(max)) if level <= max => Err("lock-order level violation"),
        _ => Ok(()),
    }
}

/// 有序同级对的判据：只声明给 `Level::Flat`；先取的那把必须是已持的那一把、地址更小，
/// 同级只此一对（成对不成链），其上也不得压着更高一级的锁。
pub(crate) fn peer(
    held: impl IntoIterator<Item = (usize, Option<Level>)>,
    addr: usize,
    lower: usize,
    level: Option<Level>,
) -> Result<(), &'static str> {
    if level != Some(Level::Flat) {
        return Err("ordered pair is only declared for Level::Flat");
    }
    let mut max: Option<Level> = None;
    let mut same = 0usize;
    let mut predecessor = false;
    for (at, held_level) in held {
        if at == addr {
            return Err("recursive acquisition");
        }
        if let Some(held_level) = held_level {
            max = Some(max.map_or(held_level, |max| max.max(held_level)));
        }
        if held_level == Some(Level::Flat) {
            same += 1;
            predecessor = at == lower;
        }
    }
    if max.is_some_and(|max| max > Level::Flat) {
        return Err("ordered pair under a higher level");
    }
    if same != 1 || !predecessor || lower >= addr {
        return Err("the held same-level lock is not the declared predecessor");
    }
    Ok(())
}
