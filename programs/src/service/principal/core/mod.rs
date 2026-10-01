//! principal::core — **名册 ＋ 谱系那本账**：两张表与一把钥匙。
//! 三份：本文件（这本账是什么 ＋ 立账 ＋ **跨两张表那一手** `adopt`）· [`roster`]（名册）·
//! [`lineage`]（谱系）。

use alloc::vec::Vec;

use env::TaskId;

use protocol::service::principal::{Fail, PrincipalId};

use lineage::Node;
use roster::Bound;

mod lineage;
mod roster;

/// 身份服务的权威状态：**两张表 + 一把钥匙**。
pub struct Principal {
    /// 唯一能写名册的那一枚——Server 启动时把它自己的 `Sire` 注入进来。
    assembler: TaskId,
    /// 谱系：只增不删，下标即号。
    tree: Vec<Node>,
    /// 名册：一 TID 一格，可增可删。
    roster: Vec<Bound>,
}

impl Principal {
    /// 立一份账：**自带根**（零号节点，没有父）。
    /// 根立不起来就没有身份可言，故备不下时如实报 [`Fail::Full`]（不 panic）。
    pub fn new(assembler: TaskId) -> Result<Principal, Fail> {
        let mut tree = Vec::new();
        tree.try_reserve(1).map_err(|_| Fail::Full)?;
        tree.push(Node { parent: None });
        Ok(Principal {
            assembler,
            tree,
            roster: Vec::new(),
        })
    }

    /// 转换 · 领：把**自己**当前的号换成 `q`。
    /// **同名反义**：内核那一侧的 `Task::adopt(child)` 是把一枚子域**收进来**，与这里"把**自己**
    /// 换到下面"方向正好相反（见正文"层"那一张表）。
    /// 三格前置各自不同一件事：
    /// - `from` 那一格在名册里，取 `p = current`；否则 [`Fail::Unknown`]——与 [`Principal::unbind`]
    ///   撞空同一格："那一格不存在"，调用方接下来都是"先让装配者给我绑"；
    /// - `q` 在树里；否则 [`Fail::Unknown`]（这个号是假的）；
    /// - **`heir(p, q)`**；否则 [`Fail::Denied`]——向上、跨支都不在**你自己那一支**里。
    /// `q == p` 时幂等。不动树、不动 `origin`（故 `origin ≼ current` 仍成立）。
    pub fn adopt(&mut self, from: TaskId, q: PrincipalId) -> Result<(), Fail> {
        // 先把三格判据问完（都是读），再去动那一格——借还清楚了，规矩也一眼看得见。
        let Some(p) = self.resolve(from) else {
            return Err(Fail::Unknown);
        };
        if self.node(q).is_none() {
            return Err(Fail::Unknown);
        }
        if !self.heir(p, q)? {
            return Err(Fail::Denied);
        }
        let row = self
            .roster
            .iter_mut()
            .find(|r| r.tid == from)
            .ok_or(Fail::Unknown)?;
        row.current = q;
        Ok(())
    }
}
