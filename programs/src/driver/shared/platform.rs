//! 受限平台窗的接收侧：从本域表里认出装配方经声明供给交来的那一段。
//!
//! 装配方（System）在可信供给里登记这些来源，UnitFile 的声明绑定把它们定向交到本台。
//! 接收侧核三件事：描述符确实来自 sire、描述形状与几何合法、本域表里那一枚窗的记号
//! 与权限与声明一致。来源那一半由内核给出的 `vestor` 核（是装配方交的），不靠报文自报。

use env::{Permission, PieKind, PieToken, TaskId, Wait};
use resource::raw::Hole;
use wire::Message;

use crate::unit::{Binding, SupplyFrame};

/// 一扇交进来的平台窗：本域那一枚门闩 ＋ 它声明的几何（物理地址，不是本域 VA）。
pub struct Window {
    pub token: PieToken,
    pub base: u64,
    pub size: u64,
}

/// 收一扇平台窗。
pub fn take(
    system: TaskId,
    binding: Binding,
    permission: Permission,
    wait: Wait,
) -> Result<Window, &'static str> {
    let (length, description) = descriptor(system, binding.slot, wait)?;
    let length = usize::from(length);
    if length != 8 && length != 16 {
        return Err("platform: description shape");
    }
    let base = u64::from_le_bytes(
        description[..8]
            .try_into()
            .map_err(|_| "platform: window base")?,
    );
    // 长度那一格缺省为零：接收方从自己那份映射里量。
    let size = if length == 16 {
        u64::from_le_bytes(
            description[8..16]
                .try_into()
                .map_err(|_| "platform: window size")?,
        )
    } else {
        0
    };
    if base == 0 || (length == 16 && (size == 0 || !size.is_multiple_of(env::PAGE_SIZE as u64))) {
        return Err("platform: window geometry");
    }
    let token = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.kind == PieKind::Pole
                && info.vestor == system
                && info.mark == binding.slot
                && info.permission == permission
        })
        .map(|info| info.token)
        .ok_or("platform: window authority")?;
    Ok(Window { token, base, size })
}

/// 收一帧声明供给的描述（发送者必须是 sire；形状读不懂当场拒）。
pub fn descriptor(
    system: TaskId,
    slot: env::Mark,
    wait: Wait,
) -> Result<(u16, [u8; system_api::supply::MAX_DESCRIPTION]), &'static str> {
    let descriptor = ipc::session::establish::claim(
        system,
        crate::unit::descriptor_mark(slot),
        wait,
    )
    .map_err(|_| "platform: descriptor channel")?;
    let mut bytes = SupplyFrame::EMPTY;
    let (len, from) = Hole::from_raw(descriptor)
        .pull(&mut bytes, wait)
        .map_err(|_| "platform: description")?;
    if from != system {
        return Err("platform: descriptor sender");
    }
    let frame = SupplyFrame::fetch(&bytes[..len]).ok_or("platform: description frame")?;
    if usize::from(frame.length) > frame.description.len() {
        return Err("platform: description shape");
    }
    Ok((frame.length, frame.description))
}
