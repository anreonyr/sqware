use std::fmt::Write;

use super::{OUTPUT, line::Line};

fn take_output() -> Vec<String> {
    OUTPUT.with(|output| std::mem::take(&mut *output.borrow_mut()))
}

#[test]
fn short_chunks_wait_for_emit() {
    let mut line = Line::new();
    line.write_str("first").unwrap();
    line.write_str(" second").unwrap();
    assert!(take_output().is_empty());
    line.emit();
    assert_eq!(take_output(), ["first second"]);
    line.emit();
    assert!(take_output().is_empty());
}

#[test]
fn full_buffer_waits_for_emit() {
    let mut line = Line::new();
    let text = "x".repeat(256);
    line.write_str(&text).unwrap();
    assert!(take_output().is_empty());
    line.emit();
    assert_eq!(take_output(), [text]);
}

#[test]
fn overflow_flushes_then_writes_chunk_directly() {
    let mut line = Line::new();
    let prefix = "x".repeat(250);
    let suffix = "y".repeat(10);
    line.write_str(&prefix).unwrap();
    line.write_str(&suffix).unwrap();
    assert_eq!(take_output(), [prefix, suffix]);
    line.emit();
    assert!(take_output().is_empty());
}

#[test]
fn oversized_chunk_bypasses_empty_buffer() {
    let mut line = Line::new();
    let text = "x".repeat(257);
    line.write_str(&text).unwrap();
    assert_eq!(take_output(), [text]);
    line.emit();
    assert!(take_output().is_empty());
}

#[test]
fn overflow_does_not_split_utf8() {
    let mut line = Line::new();
    let prefix = "x".repeat(255);
    line.write_str(&prefix).unwrap();
    line.write_str("é").unwrap();
    assert_eq!(take_output(), [prefix, "é".to_owned()]);
    line.emit();
    assert!(take_output().is_empty());
}
