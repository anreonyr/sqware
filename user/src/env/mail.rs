//! Mail 域：`MailCall::*` 转发（port / dock / ring 三族）。

use ubi::{MailCall, UArgs, UResult, Ucall, UcallBuilder};

pub fn port_open() -> UResult<(usize, usize)> {
    let (h, k) = UcallBuilder::new(Ucall::Mail(MailCall::PortOpen)).call()?;
    Ok((h, k))
}

pub fn port_shut(handle: usize) -> UResult<()> {
    let args = UArgs {
        a0: handle,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::PortShut))
        .args(args)
        .call()?;
    Ok(())
}

pub fn port_try_push(handle: usize, msg: *const u8) -> UResult<()> {
    let args = UArgs {
        a0: handle,
        a1: msg as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::PortPush))
        .args(args)
        .call()?;
    Ok(())
}

pub fn port_try_pull(handle: usize, buf: *mut u8) -> UResult<()> {
    let args = UArgs {
        a0: handle,
        a1: buf as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::PortPull))
        .args(args)
        .call()?;
    Ok(())
}

pub fn dock_open(item_len: usize, slots: usize) -> UResult<(usize, usize)> {
    let args = UArgs {
        a0: item_len,
        a1: slots,
        ..UArgs::default()
    };
    let (id, view) = UcallBuilder::new(Ucall::Mail(MailCall::DockOpen))
        .args(args)
        .call()?;
    Ok((id, view))
}

pub fn dock_join(id: usize, side: usize) -> UResult<usize> {
    let args = UArgs {
        a0: id,
        a1: side,
        ..UArgs::default()
    };
    let (view, _) = UcallBuilder::new(Ucall::Mail(MailCall::DockJoin))
        .args(args)
        .call()?;
    Ok(view)
}

pub fn dock_shut(id: usize) -> UResult<()> {
    let args = UArgs {
        a0: id,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::DockShut))
        .args(args)
        .call()?;
    Ok(())
}

pub fn dock_clone(id: usize) -> UResult<()> {
    let args = UArgs {
        a0: id,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::DockClone))
        .args(args)
        .call()?;
    Ok(())
}

pub fn dock_drop(id: usize, side: usize) -> UResult<()> {
    let args = UArgs {
        a0: id,
        a1: side,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::DockDrop))
        .args(args)
        .call()?;
    Ok(())
}

pub fn ring_open(item_len: usize, slots: usize) -> UResult<(usize, usize)> {
    let args = UArgs {
        a0: item_len,
        a1: slots,
        ..UArgs::default()
    };
    let (id, view) = UcallBuilder::new(Ucall::Mail(MailCall::RingOpen))
        .args(args)
        .call()?;
    Ok((id, view))
}

pub fn ring_join(id: usize) -> UResult<usize> {
    let args = UArgs {
        a0: id,
        ..UArgs::default()
    };
    let (view, _) = UcallBuilder::new(Ucall::Mail(MailCall::RingJoin))
        .args(args)
        .call()?;
    Ok(view)
}

pub fn ring_close(id: usize) -> UResult<()> {
    let args = UArgs {
        a0: id,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::RingClose))
        .args(args)
        .call()?;
    Ok(())
}
