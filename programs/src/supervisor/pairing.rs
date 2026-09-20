//! pairing — **这台机器的转交**：把需求单翻成单子上的一条、把配件领回来再投给客人。
//!
//! 固件面的帧与两个角色住在 [`protocol::firmware`]；boot 给的两块账住在
//! [`crate::supervisor::boot`]。本文件只剩"这台机器怎么用那一面"。

use protocol::firmware::call::{WANT_MAX, Want};

use super::needs::Need;

/// 需求单 → 单子上的几条。返 `(那几条, 条数)`。
///
/// 这条转换把"收方的需求"（带 `slot` 的常量表）变成"请求里要的那样"（不带 `slot`）；
/// 名字装不下 ⇒ `None`（**不 panic**）。
pub fn wants_of(needs: &[Need]) -> Option<([Want; WANT_MAX], usize)> {
    if needs.len() > WANT_MAX {
        return None;
    }
    let mut wants = [Want::NONE; WANT_MAX];
    for (i, need) in needs.iter().enumerate() {
        wants[i] = need.want()?;
    }
    Some((wants, needs.len()))
}
