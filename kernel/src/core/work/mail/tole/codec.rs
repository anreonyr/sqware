use alloc::sync::Weak;

use env::{PieFail, PieToken, ToleEventFail, ToleEventSource, ToleFail};

use crate::core::work::mail::{hole, nole, pole};
use crate::core::work::pend::{Need, PieSnapshot};
use crate::core::work::room::messenger::WakeKey;
use crate::core::work::unit::life::Life;

/// 权柄轴的失败折成组这一域的操作词汇。
pub(crate) fn pie_fail(error: PieFail) -> ToleFail {
    match error {
        PieFail::Denied | PieFail::NotAligned => ToleFail::Denied,
        PieFail::Dead => ToleFail::Dead,
        PieFail::OoM => ToleFail::OoM,
        PieFail::HandedOver => ToleFail::HandedOver,
        PieFail::Busy => ToleFail::Busy,
    }
}

/// 事件面上的权柄失败：与操作面分开，失权、死亡与交接各自保留一格。
pub(crate) fn event_fail(error: PieFail) -> ToleEventFail {
    match error {
        PieFail::Denied | PieFail::NotAligned => ToleEventFail::Denied,
        PieFail::Dead => ToleEventFail::Dead,
        PieFail::OoM => ToleEventFail::OoM,
        PieFail::HandedOver => ToleEventFail::HandedOver,
        PieFail::Busy => ToleEventFail::Busy,
    }
}

/// 这份 source 看的是哪一枚、要哪一种许可；组内那两种非资源 source 没有资源面。
pub(crate) fn source_watch(source: ToleEventSource) -> Option<(PieToken, Need)> {
    match source {
        ToleEventSource::Hole { token, condition } => Some((token, hole::need(condition))),
        ToleEventSource::Pole { token, .. } | ToleEventSource::Nole { token } => {
            Some((token, Need::Fetch))
        }
        ToleEventSource::Join { .. } | ToleEventSource::Inspect { .. } => None,
    }
}

/// 这份 source 落在哪个资源的哪条通知位上；不是这一族的资源就没有键。
pub(crate) fn source_key(
    pie: &PieSnapshot,
    source: ToleEventSource,
) -> Option<(WakeKey, Weak<Life>)> {
    let mail = pie.mail();
    match source {
        ToleEventSource::Hole { condition, .. } => {
            let meta = mail.hole()?;
            Some((hole::key(&meta, condition), meta.life()))
        }
        ToleEventSource::Pole { bit, .. } => {
            let meta = mail.pole()?;
            Some((pole::key(&meta, bit), meta.life()))
        }
        ToleEventSource::Nole { .. } => {
            let meta = mail.nole()?;
            Some((nole::key(&meta), meta.life()))
        }
        ToleEventSource::Join { .. } | ToleEventSource::Inspect { .. } => None,
    }
}

/// 这份 source 此刻是否已就绪：问它自己的资源机制。
pub(crate) fn source_ready(pie: &PieSnapshot, source: ToleEventSource) -> Option<bool> {
    let mail = pie.mail();
    match source {
        ToleEventSource::Hole { condition, .. } => Some(mail.hole()?.ready(condition)),
        ToleEventSource::Pole { bit, .. } => Some(mail.pole()?.ready(bit)),
        ToleEventSource::Nole { .. } => Some(mail.nole()?.ready()),
        ToleEventSource::Join { .. } | ToleEventSource::Inspect { .. } => None,
    }
}
