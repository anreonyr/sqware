//! 保留授权的委托、撤销及真实帧持有回归。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{
    AcquireSource, EnvCall, Mark, MemoryCall, MemoryFail, NoleCall, Permission, PieCall, PieFail,
    PieToken,
};

use crate::core::memory::address::{PageCount, PageOffset};
use crate::core::memory::flat::Frame;
use crate::core::work::pend;
use crate::core::work::room::conductor::tests::with_running;
use crate::core::work::room::scheduler;

use super::super::{Caller, KVirt, PAGE_SIZE, reference};

/// 只借当场的帧写一下：升级不上就是这一段已经不再被持有。
fn write_through(frames: &[Weak<Frame>], value: u8) -> bool {
    for frame in frames {
        let Some(frame) = frame.upgrade() else {
            return false;
        };
        unsafe {
            frame.as_mut_ptr().write_volatile(value);
        }
    }
    true
}

fn token_of(raw: usize) -> PieToken {
    PieToken::from_bytes(&raw.to_le_bytes()).expect("token")
}

pub fn accept() {
    scheduler::init().expect("scheduler init");

    let owner = Caller::new(false); // 源持有方：自己的分配与 Pole
    let holder = Caller::new(false); // 保留授权根持有方（引导交付的那一手）
    let receiver = Caller::new(false); // 接收方：拿子授权新增保留
    let outsider = Caller::new(false); // 未获保留授权的一方

    // 源：owner 自己的一段真实分配 + Pole；帧的弱引用先记下。
    let (_, at, source) = owner.attach_alloc_reference(7 * PAGE_SIZE, 6);
    let seen: Vec<Weak<Frame>> = {
        let view = owner.team.space.nest.probe(KVirt::wrap(at)).expect("view");
        view.frames_in(KVirt::wrap(at), 7 * PAGE_SIZE)
            .expect("frames")
            .iter()
            .map(|(_, frame)| Arc::downgrade(frame))
            .collect()
    };
    assert_eq!(seen.len(), 7, "all seven data frames are observed");
    let root = crate::entry::call::pole::create_pole(&owner.task, source, true).expect("pole");
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );

    let pole = pend::locate(&owner.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("pole meta");
    let authorization = pole.authorization().expect("authorization record");
    let count = PageCount::new(7).expect("seven pages");

    // 装配期那一手：holder 拿内核铸保留授权根；源子授权按既有 Accord 交给各方。
    let cap = crate::entry::boot::resources::grant_retention(&holder.task).expect("retention");
    let source_grant = |dst: &Caller| {
        token_of(
            pend::accord(
                &owner.task,
                root,
                &Arc::downgrade(&dst.task),
                Permission::FETCH | Permission::VEST,
                Mark::NONE,
            )
            .expect("source grant"),
        )
    };
    let receiver_source = source_grant(&receiver);
    let holder_source = source_grant(&holder);
    let outsider_source = source_grant(&outsider);

    // ① 真实现有 `PieCall::Accord`：holder 把保留授权的一枚子授权交给 receiver。
    let child = with_running(holder.task.clone(), || {
        let raw = holder.call(EnvCall::Pie(PieCall::Accord {
            src: cap.token(),
            dst: receiver.task.ident.id,
            subset: Permission::FETCH | Permission::VEST,
            mark: Mark::of("retention-child"),
        }));
        assert!(raw > 0, "accord retention child: {raw}");
        PieToken::mint(raw as usize)
    });
    let child_pie = pend::locate(&receiver.task, child).expect("child authorization");
    assert!(
        child_pie.alive() && child_pie.permission().contains(Permission::VEST),
        "the child is a live VEST authorization in the receiver's own account"
    );
    assert!(
        child_pie.mail().nole().expect("nole").retains(),
        "the child keeps the kernel-minted retention identity"
    );
    assert_eq!(
        child_pie.mail().owner(),
        holder.task.ident.id,
        "delegation does not rewrite the Nole's historical owner"
    );

    // 接收方用**子授权**做真实 Acquire：成功；来源侧只登记这一条精确占用。
    let delegated = receiver.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: receiver_source,
            offset: 0,
            count: 7,
        },
        retention: child,
    }));
    assert!(delegated > 0, "acquire under a delegated cap: {delegated}");
    assert_eq!(authorization.retention_count(), 1);
    {
        let _commit = crate::core::work::commit();
        assert_eq!(
            authorization.check_publish(PageOffset::ZERO, count, None),
            Err(PieFail::Busy),
            "the registered retention keeps this window out of new publication"
        );
    }
    assert!(write_through(&seen, 0x51), "real frames are held");

    // ② 未获授的一方：账里没有这一枚 token。
    assert_eq!(
        outsider.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: outsider_source,
                offset: 0,
                count: 1,
            },
            retention: child,
        })),
        MemoryFail::Denied.code(),
        "a cap in another account authorizes nothing"
    );
    // 别人的父 token 不在本表：号本身不授权。
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: receiver_source,
                offset: 0,
                count: 1,
            },
            retention: cap.token(),
        })),
        MemoryFail::Denied.code(),
        "the parent cap is not in this account"
    );
    // 用户自造 Nole（`Mark` 与子授权相同）：没有内核铸的保留身份。
    let forged = with_running(outsider.task.clone(), || {
        let raw = outsider.call(EnvCall::Nole(NoleCall::Unseal));
        assert!(raw > 0, "unseal: {raw}");
        let self_made = PieToken::mint(raw as usize);
        let raw = outsider.call(EnvCall::Pie(PieCall::Accord {
            src: self_made,
            dst: outsider.task.ident.id,
            subset: Permission::FETCH | Permission::VEST,
            mark: Mark::of("retention-child"),
        }));
        assert!(raw > 0, "a self-made Nole delegates too: {raw}");
        PieToken::mint(raw as usize)
    });
    assert_eq!(
        outsider.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: outsider_source,
                offset: 0,
                count: 1,
            },
            retention: forged,
        })),
        MemoryFail::Denied.code(),
        "a user-made Nole is not a kernel-minted retention object"
    );
    // 缺 VEST 的子授权：原判据不放宽。
    let no_vest = with_running(holder.task.clone(), || {
        let raw = holder.call(EnvCall::Pie(PieCall::Accord {
            src: cap.token(),
            dst: receiver.task.ident.id,
            subset: Permission::FETCH,
            mark: Mark::NONE,
        }));
        assert!(raw > 0, "no-vest child: {raw}");
        PieToken::mint(raw as usize)
    });
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: receiver_source,
                offset: 1,
                count: 1,
            },
            retention: no_vest,
        })),
        MemoryFail::Denied.code(),
        "VEST is the criterion for new retention"
    );
    let narrowed_cap = with_running(holder.task.clone(), || {
        let raw = holder.call(EnvCall::Pie(PieCall::Accord {
            src: cap.token(),
            dst: receiver.task.ident.id,
            subset: Permission::FETCH | Permission::VEST,
            mark: Mark::NONE,
        }));
        assert!(raw > 0, "narrowable child: {raw}");
        PieToken::mint(raw as usize)
    });
    let narrowed = with_running(receiver.task.clone(), || {
        receiver.call(EnvCall::Pie(PieCall::Narrow {
            token: narrowed_cap,
            subset: Permission::FETCH,
        }))
    });
    assert_eq!(narrowed, 0, "narrow away the child's VEST permission");
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: receiver_source,
                offset: 1,
                count: 1,
            },
            retention: narrowed_cap,
        })),
        MemoryFail::Denied.code(),
        "a narrowed grant no longer authorizes new retention"
    );

    // ③ 撤销已发的子授权：不能再新增保留；已取得的真实持有不提前释放。
    let revoked = with_running(holder.task.clone(), || {
        holder.call(EnvCall::Pie(PieCall::Revoke {
            dst: receiver.task.ident.id,
            token: child,
        }))
    });
    assert_eq!(revoked, 0, "the grantor revokes the child it issued");
    assert!(pend::locate(&receiver.task, child).is_none());
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: receiver_source,
                offset: 2,
                count: 1,
            },
            retention: child,
        })),
        MemoryFail::Denied.code(),
        "a revoked authorization creates no new retention"
    );
    assert_eq!(authorization.retention_count(), 1);
    assert!(
        write_through(&seen, 0x52),
        "the existing hold still holds real frames"
    );

    // 同一窗口上的第二份保留合法；解除只撤自己那一份，撤后同一窗口可再取。
    let again = |offset: usize, count: usize| {
        holder.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: holder_source,
                offset,
                count,
            },
            retention: cap.token(),
        }))
    };
    let first = again(0, 7);
    assert!(first > 0, "overlapping retention: {first}");
    assert_eq!(authorization.retention_count(), 2);
    assert_eq!(
        holder.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(first as usize),
        })),
        0
    );
    assert_eq!(authorization.retention_count(), 1);
    let second = again(0, 7);
    assert!(second > 0, "a fresh retention after a release: {second}");
    assert_eq!(authorization.retention_count(), 2);
    assert_eq!(
        holder.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(second as usize),
        })),
        0
    );
    assert_eq!(authorization.retention_count(), 1);

    // 源 Task/Pole/Pie 退出：真实帧仍被这一份保留持有（等待既有停止确认）。
    owner.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    pend::doom(&owner.task);
    assert!(owner.team.space.nest.translate(KVirt::wrap(at)).is_none());
    assert!(
        write_through(&seen, 0x53),
        "frames survive source Task/Pole/Pie loss"
    );
    assert_eq!(authorization.retention_count(), 1);

    let surviving_cap = with_running(holder.task.clone(), || {
        let raw = holder.call(EnvCall::Pie(PieCall::Accord {
            src: cap.token(),
            dst: receiver.task.ident.id,
            subset: Permission::FETCH | Permission::VEST,
            mark: Mark::NONE,
        }));
        assert!(raw > 0, "surviving child: {raw}");
        PieToken::mint(raw as usize)
    });
    let (_, _, live_source) = receiver.attach_alloc_reference(PAGE_SIZE, 6);
    let live_window = env::ReferenceRange::new(live_source, 0, 1).unwrap();
    let before_seal = receiver.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Reference(live_window),
        retention: surviving_cap,
    }));
    assert!(
        before_seal > 0,
        "the source and child authorize before Seal"
    );
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(before_seal as usize),
        })),
        0
    );
    let sealed = with_running(holder.task.clone(), || {
        holder.call(EnvCall::Nole(NoleCall::Seal { token: cap.token() }))
    });
    assert_eq!(sealed, 0);
    assert!(
        !pend::locate(&receiver.task, surviving_cap)
            .expect("sealed child keeps its query identity")
            .alive()
    );
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Reference(live_window),
            retention: surviving_cap,
        })),
        MemoryFail::Denied.code(),
        "a sealed Mail authorizes no new retention from a live source"
    );
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Release {
            reference: live_source,
        })),
        0
    );

    holder.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    pend::doom(&holder.task);
    assert!(
        write_through(&seen, 0x54),
        "the issuer's exit does not release an existing hold early"
    );
    assert_eq!(authorization.retention_count(), 1);
    assert_eq!(
        receiver.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(delegated as usize),
        })),
        0
    );
    assert_eq!(authorization.retention_count(), 0);
    {
        let _commit = crate::core::work::commit();
        assert_eq!(
            authorization.check_publish(PageOffset::ZERO, count, None),
            Ok(()),
            "the precise range is reusable again"
        );
    }

    drop(authorization);
    drop(pole);
    drop(owner);
    drop(holder);
    drop(receiver);
    drop(outsider);
    assert!(
        seen.iter().all(|frame| frame.upgrade().is_none()),
        "all retained frames are reclaimed after releasing the last hold"
    );
}
