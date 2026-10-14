use super::kind::Kind;

#[derive(Clone, Copy)]
pub(crate) struct Kinds([usize; Kind::COUNT]);

impl Kinds {
    pub(crate) const fn new(values: [usize; Kind::COUNT]) -> Self {
        Self(values)
    }

    pub(crate) fn get(&self, k: Kind) -> usize {
        self.0[k.ix()]
    }

    pub(crate) fn nonzero(&self) -> impl Iterator<Item = (Kind, usize)> + '_ {
        Kind::ALL
            .into_iter()
            .enumerate()
            .filter_map(move |(i, k)| (self.0[i] != 0).then_some((k, self.0[i])))
    }
}

impl core::fmt::Display for Kinds {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut any = false;
        for (k, n) in self.nonzero() {
            if any {
                f.write_str(" ")?;
            }
            any = true;
            write!(f, "{}={n}", k.name())?;
        }
        if !any {
            f.write_str("(空)")?;
        }
        Ok(())
    }
}
