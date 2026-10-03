use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{HoleDir, TaskId};
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::manager::asid::Asid;
use crate::work::unit::life::Life;
use crate::work::unit::task::{Task, TaskState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WakeKey {
    Space { space: Asid, slot: usize },
    Hole { hole: usize, dir: HoleDir },
    Nole { id: usize },
    /// **页上那一位"有事"**（架把铃并进页之后，页也能被等——见 `mail::pole`）。
    Pole { id: usize },
    Task { id: TaskId },
    Pies { task: TaskId },
    /// **能力可观察状态改变**（独立一格：`Pies` 的到达语义留给 `Fall`，一个字不扩）。
    Capabilities { task: TaskId },
    Tole { id: usize },
    Alarm { task: TaskId },
}

impl WakeKey {
    pub(super) fn fold(self) -> u64 {
        match self {
            WakeKey::Space { space, slot } => {
                (space.get() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ slot as u64
            }
            WakeKey::Hole { hole, dir } => {
                (hole as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dir as u64
            }
            WakeKey::Nole { id } => (id as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9),
            WakeKey::Pole { id } => (id as u64).wrapping_mul(0x87C3_7B91_1142_53D5),
            WakeKey::Task { id } => (id.get() as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task.get() as u64).wrapping_mul(0xA24B_AED4_963E_E407),
            WakeKey::Pies { task } => (task.get() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
            WakeKey::Capabilities { task } => {
                (task.get() as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
            }
            WakeKey::Tole { id } => (id as u64).wrapping_mul(0x1656_67B1_9E37_79F9),
        }
    }
}

pub(in super::super) struct Site {
    pub(in super::super) pend: bool,
    pub(in super::super) head: Option<Arc<Task>>,
    pub(in super::super) tail: Option<Arc<Task>>,
    pub(in super::super) life: Weak<Life>,
    pub(in super::super) fwd: Fwd,
}

pub(crate) const FWD_MAX: usize = 8;

#[derive(Clone)]
pub(in super::super) struct Fwd {
    ids: [usize; FWD_MAX],
    lives: [Weak<Life>; FWD_MAX],
    len: usize,
}

impl Fwd {
    pub(in super::super) fn empty() -> Self {
        Self {
            ids: [0; FWD_MAX],
            lives: core::array::from_fn(|_| Weak::new()),
            len: 0,
        }
    }

    pub(in super::super) fn attach(&mut self, tole: usize, life: Weak<Life>) -> Result<(), ()> {
        if self.ids[..self.len].contains(&tole) {
            return Ok(());
        }
        if self.len == FWD_MAX {
            return Err(());
        }
        self.ids[self.len] = tole;
        self.lives[self.len] = life;
        self.len += 1;
        Ok(())
    }

    pub(in super::super) fn detach(&mut self, tole: usize) {
        if let Some(at) = self.ids[..self.len].iter().position(|&i| i == tole) {
            self.ids.copy_within(at + 1..self.len, at);
            for i in at..self.len - 1 {
                self.lives.swap(i, i + 1);
            }
            self.lives[self.len - 1] = Weak::new();
            self.len -= 1;
        }
    }

    pub(in super::super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(in super::super) fn entries(&self) -> impl Iterator<Item = (usize, &Weak<Life>)> {
        self.ids[..self.len]
            .iter()
            .copied()
            .zip(self.lives[..self.len].iter())
    }
}

pub(in super::super) const SITE_SHARDS: usize = 16;
const SITE_SHARDS_MASK: usize = SITE_SHARDS - 1;

#[inline]
fn site_shard(key: WakeKey) -> usize {
    let h = key.fold().wrapping_mul(0x9E3779B97F4A7C15);
    ((h >> 32) ^ h) as usize & SITE_SHARDS_MASK
}

type Shard = SpinLock<HashMap<WakeKey, Site>>;

pub(in super::super) fn shard_at(shard: usize) -> &'static Shard {
    static SHARDS: OnceLock<Box<[Shard]>> = OnceLock::new();
    let arr: &'static [Shard] = SHARDS.get_or_init(|| {
        let mut v: Vec<Shard> = Vec::with_capacity(SITE_SHARDS);
        for _ in 0..SITE_SHARDS {
            v.push(SpinLock::new_level(Level::L3, HashMap::new()));
        }
        v.into_boxed_slice()
    });
    &arr[shard]
}

pub(in super::super) fn sites(key: WakeKey) -> &'static Shard {
    shard_at(site_shard(key))
}

pub(super) fn take_beacon(key: WakeKey) -> bool {
    let mut sites = sites(key).lock();
    let taken = match sites.get_mut(&key) {
        Some(site) if site.pend => {
            site.pend = false;
            true
        }
        _ => false,
    };
    if taken {
        prune(&mut sites, key);
    }
    taken
}

pub(in super::super) fn prune(sites: &mut HashMap<WakeKey, Site>, key: WakeKey) {
    if let Some(site) = sites.get(&key)
        && site.head.is_none()
        && (Life::dead(&site.life) || (!site.pend && site.fwd.is_empty()))
    {
        sites.remove(&key);
    }
}

impl Site {
    pub(super) fn new(life: &Weak<Life>) -> Self {
        Self {
            pend: false,
            head: None,
            tail: None,
            life: life.clone(),
            fwd: Fwd::empty(),
        }
    }

    pub(super) fn push_back(&mut self, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Blocked { next: None, .. }
            ),
            "站点只收 Blocked 任务，且入链前不得挂在链上"
        );
        match self.tail.take() {
            None => self.head = Some(task.clone()),
            Some(mut last) => *Task::blocked_next(&mut last) = Some(task.clone()),
        }
        self.tail = Some(task);
    }

    pub(super) fn pop_front(&mut self) -> Option<Arc<Task>> {
        let mut head = self.head.take()?;
        self.head = Task::blocked_next(&mut head).take();
        if self.head.is_none() {
            self.tail = None;
        }
        Some(head)
    }

    pub(in super::super) fn remove_if(
        &mut self,
        hit: &mut dyn FnMut(&mut Arc<Task>) -> bool,
    ) -> Option<Arc<Task>> {
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = self.head.clone();
        while let Some(mut node) = cur {
            if hit(&mut node) {
                let next = Task::blocked_next(&mut node).take();
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => *Task::blocked_next(p) = next,
                    None => self.head = next,
                }
                if was_tail {
                    self.tail = prev;
                }
                return Some(node);
            }
            prev = Some(node.clone());
            cur = Task::blocked_next(&mut node).clone();
        }
        None
    }
}
