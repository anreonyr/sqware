use alloc::{string::String, vec::Vec};
use serde::Serialize;

use crate::machine::hart;
use crate::machine::clock;

mod paragraph;
pub use paragraph::Paragraph;

#[derive(Serialize, Default)]
pub struct Report {
    seal: (usize, u64),
    pub paras: Vec<Paragraph>,
}

impl Report {
    pub fn paragraph(&mut self, name: &'static str, title: Option<String>) -> &mut Paragraph {
        self.paras.push(Paragraph {
            name,
            title,
            items: Vec::new(),
        });
        self.paras.last_mut().expect("just pushed")
    }

    pub fn seal(&mut self) -> &Self {
        self.seal = (hart::hart_id().get(), clock::now().as_ticks());
        self
    }

    #[allow(unused)]
    pub fn clear(&mut self) {
        self.paras.clear();
    }
}
