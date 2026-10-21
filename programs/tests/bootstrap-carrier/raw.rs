pub use crate::PieInfo;

pub fn pies() -> alloc::vec::IntoIter<PieInfo> {
    crate::EFFECTS.with(|effects| effects.borrow().pies.clone()).into_iter()
}

pub struct Hole(crate::PieToken);

impl Hole {
    pub fn from_raw(token: crate::PieToken) -> Self {
        Self(token)
    }

    pub fn push(&self, bytes: &[u8], _: crate::Wait) -> Result<(), ()> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.operations.push((1, self.0));
            if effects.fail_push {
                return Err(());
            }
            effects.pushes += 1;
            effects.pushed = bytes.to_vec();
            Ok(())
        })
    }

    pub fn pull(&self, out: &mut [u8], _: crate::Wait) -> Result<(usize, crate::TaskId), ()> {
        let (bytes, len, from) = crate::EFFECTS.with(|effects| {
            let effects = effects.borrow();
            (effects.pull_bytes.clone(), effects.pull_len, effects.pull_from)
        });
        let count = len.min(out.len());
        out[..count].copy_from_slice(&bytes[..count]);
        Ok((count, from))
    }
}
