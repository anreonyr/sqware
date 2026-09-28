use crate::runtime::diagnose::halt::hush;
use crate::work::room::scheduler::core::{current, fetch};

pub fn run() -> usize {
    hush();
    match current().advance() {
        Some(pa) => pa,
        None => fetch(),
    }
}