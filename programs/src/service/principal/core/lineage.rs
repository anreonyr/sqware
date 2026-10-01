//! 一格（Node：只有父，树只 push ⇒ 无环、恰好一个根）
//! 与它的读法（派生 / 问父 / 判"在这一支里"）。

use env::TaskId;
use protocol::service::principal::{Fail, PrincipalId};

use super::Principal;

/// 树上一格：**只有父**。
/// 没有 setter ⇒ 父不可改；树只 `push` ⇒ 无环、恰好一个根（第二枚无父节点写不出来）。
pub(super) struct Node {
    pub(super) parent: Option<PrincipalId>,
}

impl Principal {
    /// 谱系 · 写：由 `p` 派生一枚新节点，返新号。
    /// 钥匙两把，都是内核与名册免费给的：**装配者**，或**当前正好代表 `p` 的那一枚**
    /// （`resolve(from) == Some(p)`）。注意是"正好相等"，不是"`p` 的某个后代"——
    /// 于是"只能沿自己的 lineage 向下"从纪律变成判据，不需要新机制。
    pub fn derive(&mut self, from: TaskId, p: PrincipalId) -> Result<PrincipalId, Fail> {
        if from != self.assembler && self.resolve(from) != Some(p) {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        self.tree.try_reserve(1).map_err(|_| Fail::Full)?;
        let id = PrincipalId::new(self.tree.len());
        self.tree.push(Node { parent: Some(p) });
        Ok(id)
    }

    /// 谱系 · 读：直接父。**三态**——`Ok(Some)` 有父 / `Ok(None)` 只有根 / `Err(Unknown)` 树外。
    pub fn sire(&self, p: PrincipalId) -> Result<Option<PrincipalId>, Fail> {
        self.node(p).map(|n| n.parent).ok_or(Fail::Unknown)
    }

    /// 谱系 · 读：`a ≼ b`（`b` 在 `a` 那一支里，含 `a == b`）。
    pub fn heir(&self, a: PrincipalId, b: PrincipalId) -> Result<bool, Fail> {
        if self.node(a).is_none() || self.node(b).is_none() {
            return Err(Fail::Unknown);
        }
        let mut at = Some(b);
        while let Some(cur) = at {
            if cur == a {
                return Ok(true);
            }
            at = self.node(cur).and_then(|n| n.parent);
        }
        Ok(false)
    }

    /// 树上一格（树外答 `None`）。
    pub(super) fn node(&self, p: PrincipalId) -> Option<&Node> {
        self.tree.get(p.get())
    }
}
