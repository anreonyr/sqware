//! 协议的**号**：一张表的坐标，以及它在线上那 8 字节。
//! 三个号空间（`EntryId` / `PrincipalId` / `CoalitionId`）共用**一条规则**：
//! > **号只增、不重用**：一枚铸过的号永远指着它当初那一格（或那一格的墓碑），不会悄悄指到
//! > 后铸的那一格身上。故"这枚号铸过没有"的判据是**水位**。
//! **三个容器长得不一样，因为差别只有一维：删。**
//! ```text
//!   EntryId      Vec<Option<Slot>>   删了置墓碑（不 pop）     水位 = slots.len()
//!   PrincipalId  Vec<Node>           没有删（只增不删）       水位 = tree.len()
//!   CoalitionId  两张边表            删边、号留着             水位 = next（自持：容器会缩）
//! ```

/// 一枚协议层的号：由裸号造、取裸号、与线上那 8 字节往返。
/// **线上形状只有这一处**（8 字节小端）：三种号各写一遍 `to_bytes` 曾是这个仓的形状。
pub trait Id: Copy {
    /// 由裸号造一个（**线上解码面**：表外的号从这里进来）。
    fn new(raw: usize) -> Self;

    /// 裸号。
    fn get(self) -> usize;

    /// **交出**：号变回线上的 8 字节（小端）——`new` 的对偶面。
    fn to_bytes(self) -> [u8; 8] {
        (self.get() as u64).to_le_bytes()
    }

    /// **收号**：8 字节小端解回裸号，再造一个。
    fn from_bytes(bytes: [u8; 8]) -> Self {
        Self::new(u64::from_le_bytes(bytes) as usize)
    }
}
