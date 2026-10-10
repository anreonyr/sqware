//! Mark declarations and collision checks for environment interfaces.

use crate::Mark;

/// A named mark used to identify an interface channel or grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Definition {
    pub name: &'static str,
    pub mark: Mark,
}

/// Return the first pair of declarations that share a mark value.
pub const fn conflict(groups: &[&[Definition]]) -> Option<(&'static str, &'static str)> {
    let mut g = 0;
    while g < groups.len() {
        let mut i = 0;
        while i < groups[g].len() {
            let a = groups[g][i];
            let mut h = g;
            while h < groups.len() {
                let mut j = if h == g { i + 1 } else { 0 };
                while j < groups[h].len() {
                    let b = groups[h][j];
                    if a.mark.get() == b.mark.get() {
                        return Some((a.name, b.name));
                    }
                    j += 1;
                }
                h += 1;
            }
            i += 1;
        }
        g += 1;
    }
    None
}

/// Check provider registries without repeating their channel and grant groups.
pub const fn conflict_between(
    registries: &[&[&[Definition]]],
) -> Option<(&'static str, &'static str)> {
    let mut a = 0;
    while a < registries.len() {
        if let Some(pair) = conflict(registries[a]) {
            return Some(pair);
        }
        let mut b = a + 1;
        while b < registries.len() {
            let mut g = 0;
            while g < registries[a].len() {
                let mut i = 0;
                while i < registries[a][g].len() {
                    let left = registries[a][g][i];
                    let mut h = 0;
                    while h < registries[b].len() {
                        let mut j = 0;
                        while j < registries[b][h].len() {
                            let right = registries[b][h][j];
                            if left.mark.get() == right.mark.get() {
                                return Some((left.name, right.name));
                            }
                            j += 1;
                        }
                        h += 1;
                    }
                    i += 1;
                }
                g += 1;
            }
            b += 1;
        }
        a += 1;
    }
    None
}
