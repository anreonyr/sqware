use alloc::sync::Weak;

use crate::core::work::unit::life::Life;

pub(crate) const FWD_MAX: usize = 8;

#[derive(Clone)]
pub(super) struct Fwd {
    ids: [usize; FWD_MAX],
    lives: [Weak<Life>; FWD_MAX],
    len: usize,
}

impl Fwd {
    pub(super) fn empty() -> Self {
        Self {
            ids: [0; FWD_MAX],
            lives: core::array::from_fn(|_| Weak::new()),
            len: 0,
        }
    }

    pub(super) fn attach(&mut self, tole: usize, life: Weak<Life>) -> Result<(), ()> {
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

    pub(super) fn detach(&mut self, tole: usize) {
        if let Some(at) = self.ids[..self.len].iter().position(|&i| i == tole) {
            self.ids.copy_within(at + 1..self.len, at);
            for i in at..self.len - 1 {
                self.lives.swap(i, i + 1);
            }
            self.lives[self.len - 1] = Weak::new();
            self.len -= 1;
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = (usize, &Weak<Life>)> {
        self.ids[..self.len]
            .iter()
            .copied()
            .zip(self.lives[..self.len].iter())
    }
}
