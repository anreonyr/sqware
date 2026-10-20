use alloc::sync::{Arc, Weak};
use alloc::vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::flat::{FlatRange, Frame};
use crate::core::memory::nest::{Reservation, View};
use crate::core::work::pend::SourceAuthorization;
use crate::core::work::room::messenger;
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::task::Task;
use crate::core::work::unit::team::{Team, TeamBuilder};
use crate::machine::switcher::context::{Gprs, TrapContext};

use super::super::support;
use super::{TARGET, rw};

const SCRATCH: usize = 0x7000_0000;

pub(super) struct Fixture {
    pub(super) team: Arc<Team>,
    pub(super) task: Arc<Task>,
    scratch: Arc<View>,
    scratch_frame: Weak<Frame>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        assert!(
            crate::core::work::room::conductor::current()
                .running_task()
                .is_none(),
            "attach_rollback: a stray running task would receive this call"
        );
        let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let task = team.task().hold().unwrap();
        let frame = Frame::allocate().unwrap();
        let scratch_frame = Arc::downgrade(&frame);
        let scratch =
            support::map_frames(&team.space, KVirt::wrap(SCRATCH), vec![frame], rw(), rw())
                .unwrap();
        Self {
            team,
            task,
            scratch,
            scratch_frame,
        }
    }

    pub(super) fn source(&self) -> (Arc<SourceAuthorization>, FlatRange, Weak<Frame>) {
        let frame = Frame::allocate().unwrap();
        let held = Arc::downgrade(&frame);
        let range = support::frames_source(vec![frame]).unwrap();
        let authorization =
            SourceAuthorization::new(range.clone(), rw(), self.task.ident.id).unwrap();
        (authorization, range, held)
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
            "attach_rollback: the description page is writable"
        );
    }

    pub(super) fn attach(&self, reference: usize, at: usize, team: env::TeamId) -> isize {
        self.write(SCRATCH, &[reference, 0, 1]);
        self.write(SCRATCH + 32, &[2, at, PAGE_SIZE]);
        let call = env::MemoryCall::Attach {
            range: env::VirtAddr::new(SCRATCH),
            team,
            placement: env::VirtAddr::new(SCRATCH + 32),
            access: 6,
            control: env::ViewControl::Caller,
        };
        let (slot, regs) = (call.slot(), call.pack());
        let frame = unsafe { &mut *(self.task.ident.frame_pa.as_usize() as *mut TrapContext) };
        for (index, value) in regs.into_iter().enumerate() {
            frame.gpr.set_x(Gprs::A0 + index, value);
        }
        frame.gpr.set_x(Gprs::A7, slot);
        assert!(
            crate::entry::call::dispatch(frame, self.task.ident.clone()).is_some(),
            "attach_rollback: the env call resumes this task"
        );
        frame.gpr.x(Gprs::A0) as isize
    }

    pub(super) fn claim(&self, authorization: &Arc<SourceAuthorization>) -> (usize, Reservation) {
        let reservation =
            support::reserve(&self.team.space, KVirt::wrap(TARGET), PAGE_SIZE).unwrap();
        let id = reservation.id();
        self.team
            .space
            .views
            .claim(
                id,
                &self.team.space,
                self.task.ident.id,
                env::ViewControl::Caller,
                authorization.clone(),
                None,
            )
            .unwrap();
        self.task.track_control(&self.team.space, id).unwrap();
        (id, reservation)
    }

    pub(super) fn publish(&self, reservation: Reservation, range: FlatRange) -> Arc<View> {
        self.team
            .space
            .nest
            .attach(reservation, range, rw(), rw())
            .unwrap()
    }

    pub(super) fn finish(self) {
        drop(self.scratch);
        messenger::slay(&self.task);
        assert!(self.task.reaped());
        drop(self.task);
        drop(self.team);
        messenger::bury();
        crate::core::work::room::scheduler::roster::prune_dead();
        assert!(
            self.scratch_frame.upgrade().is_none(),
            "attach_rollback: the fixture's page is released"
        );
    }
}
