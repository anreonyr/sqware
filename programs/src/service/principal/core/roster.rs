//! 一格（Bound：一 TID 一格，必有起点、必有当前）
//! 与它的写读（绑定 / 问"此刻代表谁" / 弃回起点 / 删格）。

use env::TaskId;
use protocol::service::principal::{Fail, PrincipalId};

use super::Principal;

/// 名册一格：**一 TID 一格，且必有起点、必有当前**（两格都不是 Option）。
pub(super) struct Bound {
    pub(super) tid: TaskId,
    pub(super) origin: PrincipalId,
    pub(super) current: PrincipalId,
}

impl Principal {
    /// 名册 · 写：把一条 TID 定到一条**已存在**的 PrincipalId 上（只有装配者能写）。
    /// 覆盖 = **换绑**（不是 `Taken`）：同一位写第二次要么是更正，要么是装配表写错了，
    /// 都不是"别人抢了"——与 operator 否掉"名字已被占"同一条理由。
    /// **换绑 = 重定起点**：两格一起写（`origin` 与 `current` 同值）。
    pub fn bind(&mut self, from: TaskId, tid: TaskId, p: PrincipalId) -> Result<(), Fail> {
        if from != self.assembler {
            return Err(Fail::Denied);
        }
        if self.node(p).is_none() {
            return Err(Fail::Unknown);
        }
        if let Some(row) = self.roster.iter_mut().find(|r| r.tid == tid) {
            row.origin = p;
            row.current = p;
            return Ok(());
        }
        self.roster.try_reserve(1).map_err(|_| Fail::Full)?;
        self.roster.push(Bound {
            tid,
            origin: p,
            current: p,
        });
        Ok(())
    }

    /// 名册 · 读：这条 TID **此刻**代表谁。
    /// **没有失败域**：`None`（没绑）是一个诚实的答案，不是错误码——收到它的人自己决定
    /// 怎么对待一条没身份的请求。读的是 `current`（`adopt` 改过的那一格）。
    pub fn resolve(&self, tid: TaskId) -> Option<PrincipalId> {
        self.roster.iter().find(|r| r.tid == tid).map(|r| r.current)
    }

    /// 转换 · 弃：把 `current` 写回 `origin`——**回到装配给我的那一条**。
    pub fn waive(&mut self, from: TaskId) -> Result<(), Fail> {
        let Some(row) = self.roster.iter_mut().find(|r| r.tid == from) else {
            return Err(Fail::Unknown);
        };
        row.current = row.origin;
        Ok(())
    }

    pub fn drop(&mut self, from: TaskId) -> Result<(), Fail> {
        let before = self.roster.len();
        self.roster.retain(|r| r.tid != from);
        if self.roster.len() == before {
            return Err(Fail::Unknown);
        }
        Ok(())
    }
}
