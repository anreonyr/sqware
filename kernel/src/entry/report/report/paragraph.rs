use alloc::{string::String, vec::Vec};
use serde::Serialize;

#[derive(Serialize)]
pub struct Paragraph {
    pub name: &'static str,
    pub title: Option<String>,
    pub items: Vec<Vec<Option<String>>>,
}
