use env::Mark;

#[derive(Clone, Copy)]
pub struct Definition {
    pub name: &'static str,
    pub mark: Mark,
}

/// Return the names of the first colliding definitions.
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
                    if a.mark.get() == b.mark.get() { return Some((a.name, b.name)); }
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
