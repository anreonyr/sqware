use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use env::{MemoryCall, PieToken, Placement, PoleCall, ReferenceRange, TeamId};
use env::{ViewControl, VirtAddr as EnvAddr};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr as KVirt};
use crate::core::memory::flat::{FlatRange, Frame};
use crate::core::memory::nest::{Access, SpaceKind, View};
use crate::core::work::mail::pole::PoleMeta;
use crate::core::work::pend::{self, SourceAuthorization};
use crate::core::work::room::{messenger, scheduler};
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::task::{Task, TaskWeak};
use crate::core::work::unit::team::{Team, TeamBuilder};
use crate::entry::call::pole::create_pole;
use crate::machine::switcher::context::{Gprs, TrapContext};

use super::super::support;
use super::{FIRST, flat_reference, rw};

const SCRATCH: usize = 0x7000_0000;

fn dispatch(task: &Task, slot: usize, regs: [usize; 6]) -> (isize, usize) {
    let frame = unsafe { &mut *(task.ident.frame_pa.as_usize() as *mut TrapContext) };
    for (index, value) in regs.into_iter().enumerate() {
        frame.gpr.set_x(Gprs::A0 + index, value);
    }
    frame.gpr.set_x(Gprs::A7, slot);
    assert!(
        crate::entry::call::dispatch(frame, task.ident.clone()).is_some(),
        "sub_pole: the env call resumes this task"
    );
    (frame.gpr.x(Gprs::A0) as isize, frame.gpr.x(Gprs::A1))
}

pub(super) fn call(task: &Task, request: MemoryCall) -> (isize, usize) {
    dispatch(task, request.slot(), request.pack())
}

pub(super) fn pole_call(task: &Arc<Task>, request: PoleCall) -> (isize, usize) {
    crate::core::work::room::conductor::tests::with_running(task.clone(), || {
        dispatch(task, request.slot(), request.pack())
    })
}

/// 可迁移的父来源及一页用于线外调用的描述区。
pub(super) struct Fixture {
    pub(super) team: Arc<Team>,
    pub(super) task: Arc<Task>,
    scratch: Arc<View>,
    pub(super) token: PieToken,
    pub(super) meta: Arc<PoleMeta>,
    pub(super) authorization: Arc<SourceAuthorization>,
    pub(super) source: FlatRange,
}

impl Fixture {
    pub(super) fn new(pages: usize) -> Self {
        Self::with_sharing(pages, false)
    }

    pub(super) fn shared(pages: usize) -> Self {
        Self::with_sharing(pages, true)
    }

    fn with_sharing(pages: usize, shared: bool) -> Self {
        assert!(
            crate::core::work::room::conductor::current()
                .running_task()
                .is_none(),
            "sub_pole: a stray running task would receive these calls"
        );
        let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let task = team.task().hold().unwrap();
        let input = support::allocate(
            &team.space,
            core::alloc::Layout::from_size_align(pages * PAGE_SIZE, PAGE_SIZE).unwrap(),
            rw(),
            Some(FIRST),
        )
        .unwrap();
        let scratch = support::map_frames(
            &team.space,
            KVirt::wrap(SCRATCH),
            vec![Frame::allocate().unwrap()],
            rw(),
            rw(),
        )
        .unwrap();
        let (token, meta) = support::source_pole(&task, &team.space, &input, shared);
        drop(input);
        let authorization = meta.authorization().expect("source authorization");
        let source = meta.source();
        Self {
            team,
            task,
            scratch,
            token,
            meta,
            authorization,
            source,
        }
    }

    fn write(&self, at: usize, words: &[usize]) {
        let mut bytes = [0u8; 32];
        for (index, word) in words.iter().enumerate() {
            bytes[index * 8..(index + 1) * 8].copy_from_slice(&(*word as u64).to_le_bytes());
        }
        assert!(
            self.team
                .space
                .copy_out(&bytes[..words.len() * 8], KVirt::wrap(at)),
            "sub_pole: the description page is writable"
        );
    }

    pub(super) fn call(&self, request: MemoryCall) -> (isize, usize) {
        call(&self.task, request)
    }

    pub(super) fn acquire(
        &self,
        token: PieToken,
        offset: usize,
        count: usize,
        retention: PieToken,
    ) -> isize {
        self.call(MemoryCall::Acquire {
            source: env::AcquireSource::Source {
                token,
                offset,
                count,
            },
            retention,
        })
        .0
    }

    pub(super) fn target(&self, entry: KVirt) -> Arc<Team> {
        let target = TeamBuilder::build(
            SpaceKind::User,
            TaskWeak::stored(Arc::downgrade(&self.task), Origin::Sire),
        )
        .unwrap();
        drop(
            support::allocate(
                &target.space,
                core::alloc::Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap(),
                Access::READ | Access::EXECUTE,
                Some(entry),
            )
            .unwrap(),
        );
        target
    }

    pub(super) fn attach(
        &self,
        reference: isize,
        offset: usize,
        count: usize,
        at: KVirt,
    ) -> (isize, usize) {
        self.attach_to(reference, offset, count, at, TeamId::new(0), rw())
    }

    pub(super) fn attach_to(
        &self,
        reference: isize,
        offset: usize,
        count: usize,
        at: KVirt,
        team: TeamId,
        access: crate::core::memory::nest::Access,
    ) -> (isize, usize) {
        let range = ReferenceRange::new(flat_reference(reference), offset, count)
            .expect("reference range")
            .encode();
        self.write(SCRATCH, &range);
        let placement = Placement::exact(EnvAddr::new(at.as_usize()), PAGE_SIZE)
            .expect("placement")
            .encode();
        self.write(SCRATCH + 32, &placement);
        self.call(MemoryCall::Attach {
            range: EnvAddr::new(SCRATCH),
            team,
            placement: EnvAddr::new(SCRATCH + 32),
            access: access.bits() << 1,
            control: ViewControl::Caller,
        })
    }

    pub(super) fn reference(
        &self,
        source: &Arc<SourceAuthorization>,
        meta: &Arc<PoleMeta>,
        token: PieToken,
        offset: PageOffset,
        count: PageCount,
    ) -> env::FlatReference {
        let grant = meta.grant(token).expect("branch grant");
        let id = self
            .task
            .pend
            .references
            .insert(
                source.clone(),
                offset,
                count,
                source.ceiling(),
                None,
                Some((token, grant)),
            )
            .expect("reference");
        env::FlatReference::from_bytes(&id.to_le_bytes()).expect("reference handle")
    }

    pub(super) fn derive(
        &self,
        offset: PageOffset,
        count: PageCount,
        shared: bool,
    ) -> Result<PieToken, env::PieFail> {
        let reference = self.reference(&self.authorization, &self.meta, self.token, offset, count);
        let result = create_pole(&self.task, reference, shared);
        self.task.pend.references.remove(reference.get());
        result
    }

    pub(super) fn child(&self, token: PieToken) -> (Arc<PoleMeta>, Arc<SourceAuthorization>) {
        let meta = pend::locate(&self.task, token)
            .expect("child Pie")
            .mail()
            .pole()
            .expect("Pole");
        let authorization = meta.authorization().expect("child source");
        (meta, authorization)
    }

    pub(super) fn finish(self) {
        let Fixture {
            team,
            task,
            scratch,
            token: _,
            meta,
            authorization,
            source,
        } = self;
        let tokens: Vec<PieToken> = task.pend.pies.lock().iter().map(|p| p.token()).collect();
        for held in tokens {
            let _ = pend::release(&task, held);
        }
        drop(source);
        drop(meta);
        drop(authorization);
        drop(scratch);
        {
            let _commit = crate::core::work::commit();
            if task.ready() {
                // Ready 的退出从饥饿队列摘除；同段入队并退出，其他核不能取走它。
                scheduler::kick(crate::machine::hart::hart_id(), task.clone());
            }
            messenger::slay(&task);
        }
        drop(task);
        drop(team);
        messenger::bury();
        scheduler::roster::prune_dead();
    }
}
