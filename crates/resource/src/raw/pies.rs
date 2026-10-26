//! 能力表游标与分批读取。

use env::{PieInfo, PieToken, VirtAddr};

use super::Pie;

/// Increasing token cursor; removals do not shift the next position.
/// Batches observe current state rather than freezing the whole table.
pub struct Pies {
    after: PieToken,
    words: [usize; PieInfo::WORDS * 8],
    at: usize,
    count: usize,
    done: bool,
}

impl Iterator for Pies {
    type Item = Pie;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.at == self.count {
            self.count = env::pie::collect(
                self.after,
                VirtAddr::new(self.words.as_mut_ptr() as usize),
                8,
            )
            .expect("capability table read failed");
            self.at = 0;
            if self.count == 0 {
                self.done = true;
                return None;
            }
        }
        let start = self.at * PieInfo::WORDS;
        let info = PieInfo::from_words(
            self.words[start..start + PieInfo::WORDS]
                .try_into()
                .unwrap(),
        )
        .expect("invalid capability table entry");
        self.at += 1;
        self.after = info.token;
        Some(info)
    }
}

pub fn pies() -> Pies {
    Pies {
        after: PieToken::NONE,
        words: [0; PieInfo::WORDS * 8],
        at: 0,
        count: 0,
        done: false,
    }
}
