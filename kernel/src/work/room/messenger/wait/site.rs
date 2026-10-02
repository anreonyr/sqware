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
    Task { id: TaskId },
    Pies { task: TaskId },
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
            WakeKey::Task { id } => (id.get() as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task.get() as u64).wrapping_mul(0xA24B_AED4_963E_E407),
            WakeKey::Pies { task } => (task.get() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
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
        // **（临时读数）站点是被谁摘的、摘的时候什么形状**：`wipe`/`unforward` 是另两条路
        // （各自有读数），这一条是"没人动它、它自己被判死"——`dead=1` 才是那个判据。
        // **带转发的站点被摘**（`fwd>0`）就是"转发凭空没了"最硬的一条原文，故**无论多少次都打**；
        // 空站点那种收尾只打前 40 次（否则每收一枚孔都占一行）。
        let n_fwd = site.fwd.entries().count();
        let interesting = n_fwd > 0;
        static HOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
        static COLD: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
        let said = if interesting {
            HOT.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 200
        } else {
            COLD.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 40
        };
        if said {
            let (id, kind) = label(key);
            crate::putln!(
                "site: prune key={} kind={} fwd={} pend={} dead={}",
                id,
                kind,
                n_fwd,
                site.pend,
                Life::dead(&site.life)
            );
        }
        sites.remove(&key);
    }
}

/// **（临时读数）键长什么样**：`(号, 种类)`。种类 `0/1` = 孔 `Pull/Push`；`2` = 组；
/// `3` = 铃；`4` = 任务；`5` = 一族的位；`6` = 闹钟；`7` = 空间那一格。
///
/// 站点那些读数（`prune`/`wipe`/`unforward`）都要与"哪一枚孔、哪个方向"对得上号——
/// **`Hole{Pull}` 与 `Hole{Push}` 是两个站点**，不报方向这一格就分不开。
pub(in super::super) fn label(key: WakeKey) -> (usize, usize) {
    match key {
        WakeKey::Hole { hole, dir } => (
            hole,
            match dir {
                HoleDir::Pull => 0,
                HoleDir::Push => 1,
            },
        ),
        WakeKey::Tole { id } => (id, 2),
        WakeKey::Nole { id } => (id, 3),
        WakeKey::Task { id } => (id.get(), 4),
        WakeKey::Pies { task } => (task.get(), 5),
        WakeKey::Alarm { task } => (task.get(), 6),
        WakeKey::Space { space, slot } => ((space.get() as usize) ^ slot, 7),
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
