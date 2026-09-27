//! driver::device — **一台设备**：领配给 → 开图 → 交出坐标与视图。
//!
//! **照实记（为什么有这一枚）**：三台驱动起手那几步逐字同构——"按本域那张单子的长度收配给"
//! （[`assemble::take`]）、"一条 `Pair` → 一页映射"（`Dock::open(PolePie)`）、"取坐标"
//! （`Pair::key`）。收进来之后，驱动里不再出现 `PolePie` / `Pair::token()` 这类载体细节。
//!
//! **它不含设备语义**：读哪个寄存器、FIFO 怎么排、闸门开哪一位——住各域自己的设备模块
//! （`uart.rs` / `rtc.rs` / `plic.rs`）。本文件只办"从内核手里把那一页接过来"这一件事。
//!
//! **`Nole`（门铃）不走这里**：它没有寄存器页，只有一枚号——路由者直接用那个
//! [`Pair`]（见 `driver/router/adapt/boot.rs`）。

use env::{Key, Pair};
use runtime::core::dock::{Dock, View};
use runtime::env::mail::PolePie;

use crate::driver::assemble;
use crate::driver::fail::Fail;

/// 一台设备的持有者手里那两样：**那一页的映射**（域活多久它活多久）＋ **坐标**。
pub struct Device {
    dock: Dock,
    key: Key,
}

impl Device {
    /// 收配给：`N` = 本域那张单子的长度；**"有几格"只由那张单子说**。
    ///
    /// 死法那句话由本族自己说（`"assemble"`——**步名**，见 [`crate::driver::fail`]），
    /// 号由 [`assemble::take`] 原样带来（`E_UP` / `E_GRANT`）。
    pub fn claim<const N: usize>() -> Result<[Pair; N], Fail> {
        assemble::take::<N>().map_err(|c| Fail::at(c, "assemble"))
    }

    /// 一条记录 → 一页映射 ＋ 坐标（`Pole` 那一类）。
    ///
    /// **失败那一格由调用方命名**（`"device open failed"` / `"docks"`——**步名**，
    /// 见 [`crate::driver::fail`] 那一格裁）——本文件不认识域名，也不该认识。
    pub fn open(pair: Pair) -> Result<Device, ()> {
        let key = pair.key().ok_or(())?;
        let dock = Dock::open(PolePie::from_token(pair.token())).map_err(|_| ())?;
        Ok(Device { dock, key })
    }

    /// 那一页的视图（[`View`] 是 `Copy`：常驻那一圈每醒一次取一份）。
    pub fn view(&self) -> View {
        self.dock.view()
    }

    /// 坐标（登记那条线要用它；本域不写死地址）。
    pub fn key(&self) -> Key {
        self.key
    }
}
