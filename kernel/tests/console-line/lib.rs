#![cfg(test)]

use std::cell::RefCell;

thread_local! {
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn write(text: &str) {
    OUTPUT.with(|output| output.borrow_mut().push(text.to_owned()));
}

#[path = "../../src/machine/console/line.rs"]
mod line;

mod tests;
