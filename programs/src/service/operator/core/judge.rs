//! operator::core::judge — **判据**（纯，不碰账）：`Facts` 那四个问句 ＋ 判一格 `judge`
//! （谁在问 / 这一支里吗 / 这一盟里吗 / 这一格谁开着）。裁决与翻码在 [`super::gate`]。

use env::TaskId;

use protocol::service::coalition::CoalitionId;
use protocol::service::operator::{EntryId, Permit, Ruling};
use protocol::service::principal::PrincipalId;

/// 名册 / 谱系 / 盟册 / 树——**判一格要问的全部事实**，四条边一个出口。
pub trait Facts {
    fn who(&self, tid: TaskId) -> Result<Option<PrincipalId>, ()>;
    fn heir(&self, a: PrincipalId, b: PrincipalId) -> Result<bool, ()>;
    fn amid(&self, me: PrincipalId, at: CoalitionId) -> Result<bool, ()>;
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()>;
}

/// 判一格。`who` 是内核在 `Push` 那一刻盖的章；`permit` 是那一格自己那一句话。
pub fn judge(f: &impl Facts, who: TaskId, permit: Permit) -> Ruling {
    let Some(me) = (match f.who(who) {
        Ok(found) => found,
        Err(()) => return Ruling::Unjudged,
    }) else {
        return Ruling::Deny;
    };
    // 二、照那一句话问那一条边。**三种"答不是"都是 `Ok(false)`，不是失败**。
    match permit {
        Permit::Unset => Ruling::Allow,
        Permit::Trunk(p) => allow(me == p),
        Permit::Bough(p) => match f.heir(p, me) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        Permit::Among(c) => match f.amid(me, c) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        // 三、**两问**：先问树"那一格谁开着"，再问名册"那位此刻代表谁"。两问的失败域各自落格，
        //    与上面几条同一分法：**"没有那一位"是判不了**（三因同落，其中两因永久，见 [`Facts::opens`]），
        //    **"那一位没身份"是终态拒**。
        Permit::Opener(at) => match f.opens(at) {
            Ok(Some(that)) => match f.who(that) {
                Ok(Some(theirs)) => allow(me == theirs),
                Ok(None) => Ruling::Deny,
                Err(()) => Ruling::Unjudged,
            },
            Ok(None) => Ruling::Unjudged,
            Err(()) => Ruling::Unjudged,
        },
    }
}

const fn allow(ok: bool) -> Ruling {
    if ok { Ruling::Allow } else { Ruling::Deny }
}
