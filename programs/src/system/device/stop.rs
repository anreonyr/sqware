//! 停链走到哪一步。

/// 停链走到哪一步（照 IOMMU 规范那条 NOTE 的次序）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Stop {
    /// ① 撤访问权：`DDTP` 落 Off，等 busy 落并回读确认。
    Revoke,
    /// ② 改动全局可见之后失效（`IOTINVAL.VMA`，一次提交）。
    Invalidate,
    /// ③ `IOFENCE.C` 并等**本次**完成记录。
    Fence,
    /// ④ 端点侧不再发起：清 bus-master、按 virtio 那一手复位（各自回读确认）。
    Drain,
    /// ⑤ 两台立过的队列停机确认（on 落了、busy 也清了）。
    Queues,
    /// ⑥ 回读两个队列的故障位与页请求队列。
    Confirm,
    /// 整条链走完。
    Done,
}
