#![allow(dead_code)]

mod machine {
    pub mod hart {
        include!("fixture.rs");
    }
}

mod copies {
    include!("../../src/core/work/unit/copies.rs");
}

#[cfg(test)]
mod cases;
