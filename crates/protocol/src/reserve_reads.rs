// ── 术语与它的那支宏 ───────────────────────────────────────
//
// 那支宏只做一件事：**把一份身体按格／按表铺开**——一处裁决都不放。术语全部从地板取
// （`env::fid::PieCall::Reserve` 的三格名 `vestor` / `owner` / `mark`、线上答话那一格），
// 不自造。住 `reserve_reads.rs` 是因为板、树、线、货四家都要用——**本仓为这一个形状新开了文件**。

/// `Reserve` 的三格：**一个调用的三个事实**，按格展开——一格一个读出。
///
/// 三格的读法只此一份（`mail::reserve` 那一问 + 两个哨兵）：`owner == 0`（引导期那批设备
/// 门闩）不算"谁开的"；记号答不出就报 `None`。调用点只给**格名**（`vestor` / `owner` /
/// `mark`）与**自己那一侧的名字**，故"三格是一组、三个名字等长"在调用处一眼可见——
/// 原先板与树各写一份 `probe`5 / `opened_by`9 / `mark_of`7，不等长本身就是信号。
///
/// `$vis` 那一格是给**共享体与领域名分家**用的：身体住 `session::call`，名字由调用点给。
macro_rules! reserve_reads {
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => vestor $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            runtime::env::mail::reserve($arg)
                .ok()
                .map(|(vestor, _owner, _mark)| vestor)
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => owner $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
                _ => None,
            }
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => mark $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::Mark> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, _owner, mark)) => Some(mark),
                _ => None,
            }
        }
    };
}
