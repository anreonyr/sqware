//! 配对块 —— boot 交给 root 的**门闩账**（一条 = 一枚我造的门闩）。
//!
//! 这不是 envcall 载荷，而是**启动期借映块**的线格式：内核 boot 扫一次设备树，把**每个
//! (节点, `reg` 段)**、以及它自己造的那两枚（设备树本体 / 门铃）各做成一枚门闩，再把
//! 「坐标 + 我持它的号」写成定长记录、只读借映进 root 的空间（与 initrd 清单视图同一套机制）。
//!
//! ```text
//! [0..16)  key    [u8; KEY_LEN]   坐标（判别号 + 那个数，见 [`Key`]）
//! [16..24) token  usize LE        该门闩**在 root 表里**的句柄
//! ```
//!
//! 为什么格式定义在 `env::wire`（而不是内核与 root 各写一遍）：两个字段都是本模块已有的类型
//! （[`Key`] 与 [`PieToken`]），两边共用一份定义即无第二份账——内核只写不读、root 只读不写，
//! **各自都不解释设备语义**。
//!
//! `token = 0` 是无效哨兵（`PieToken` 的约定），故有效记录恒有非零 token；坐标的判别号
//! 不认识 ⇒ [`Pair::key`] 答 `None`（记录判废）。

use crate::key::{KEY_LEN, Key};
use env::PieToken;
use env::wire::Field;

/// 一条记录的字面字节数（`KEY_LEN` + 8）。
pub const PAIR_LEN: usize = KEY_LEN + size_of::<usize>();

/// 配对块的一条：坐标 + 我持它的号。
///
/// `repr(C)` + 两个定长字段 ⇒ 尺寸即 [`PAIR_LEN`]（编译期断言锁死），内核可直接把记录数组
/// 写进借映块、root 直接按块读，不需要序列化步骤。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pair {
    key: Key,
    token: PieToken,
}

/// 尺寸即线格式（`PAIR_LEN` 是 root 侧的步长，写错即整块错位）。
const _: () = assert!(size_of::<Pair>() == PAIR_LEN);

impl Pair {
    /// 空的一条：填数组用（坐标那一格是判废的判别号 ⇒ [`Pair::key`] 一律答 `None`，
    /// 故 `token` 那一格是什么都不影响读侧——与 `Want::NONE` 同一条口径）。
    pub const NONE: Pair = Pair {
        key: Key::NONE,
        token: PieToken::NONE,
    };

    /// 造一条（root 侧）：坐标已定，号已到手。
    pub fn new(key: Key, token: PieToken) -> Self {
        Self { key, token }
    }

    /// 造一条的**字节**（内核侧）：`token` 是**裸号**——内核持的就是它表里的号，而
    /// [`PieToken`] 是"收号的人"才该有的类型（见 [`env::wire::handle`]）。
    /// 故内核这一侧根本不经手句柄类型：它写字节，root 那边读成 [`Pair`]。
    ///
    /// 两个造法**对偶**：内核 [`bytes`](Pair::bytes)（写）、域 [`new`](Pair::new)（读）。
    ///
    /// [`env::wire::handle`]: env::wire::handle
    pub fn bytes(key: Key, token: usize) -> [u8; PAIR_LEN] {
        let mut out = [0u8; PAIR_LEN];
        out[..KEY_LEN].copy_from_slice(&key.bytes());
        out[KEY_LEN..].copy_from_slice(&(token as u64).to_le_bytes());
        out
    }

    /// 读一条（root 侧）：**判别号不认识 ⇒ `None`**（就地判废，由调用方决定跳过还是拒启，
    /// 与本仓「非法输入落在返回值上」一致）。
    pub fn key(&self) -> Option<Key> {
        Key::from_bytes(self.key.bytes())
    }

    /// 该门闩在**本任务**表里的句柄。
    pub const fn token(&self) -> PieToken {
        self.token
    }
}

/// 记录的那一格（回单那一段尾巴要 `T: Field`，见 `contract::driver::supply::frame`）。
///
/// **照实记（为什么可以整条按字节搬）**：[`Pair`] 是 `repr(C)`、尺寸由上面那条编译期断言钉死、
/// 字段全是 POD ⇒ 按字节写满、按字节读回都合法。这一手从前散在三处（`programs` 那侧的
/// `pair_bytes` 只读视图、`grant::each` 与 `supply::client::pick` 里的 `read_unaligned`）——
/// 今天收在类型自己身上（"impl 跟着类型走"）。
impl Field for Pair {
    const WIDTH: usize = PAIR_LEN;

    fn store(&self, out: &mut [u8]) {
        // SAFETY: 同上（`repr(C)`、尺寸断言锁死、字段全是 POD）。
        let raw = unsafe { &*(self as *const Pair).cast::<[u8; PAIR_LEN]>() };
        out.copy_from_slice(raw);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..PAIR_LEN)?;
        let mut pair = Pair::NONE;
        // SAFETY: 同 `store`；`copy_nonoverlapping` 把那 `PAIR_LEN` 字节写满。
        unsafe {
            core::ptr::copy_nonoverlapping(
                raw.as_ptr(),
                (&mut pair as *mut Pair).cast::<u8>(),
                PAIR_LEN,
            );
        }
        Some(pair)
    }
}
