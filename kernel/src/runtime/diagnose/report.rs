use alloc::{string::String, vec::Vec};
use serde::Serialize;

use crate::hart;
use crate::runtime::chrono::clock;

#[derive(Serialize, Default)]
pub struct Report {
    seal: (usize, u64),
    pub paras: Vec<Paragraph>,
}

#[derive(Serialize)]
pub struct Paragraph {
    pub name: &'static str,
    pub title: Option<String>,
    pub items: Vec<Vec<Option<String>>>,
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