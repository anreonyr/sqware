use alloc::rc::Rc;

#[derive(Clone, Debug)]
pub struct Source {
    name: Rc<str>,
    text: Rc<str>,
}
impl Source {
    pub fn new(name: &str, text: &str) -> Self {
        Self {
            name: Rc::from(name),
            text: Rc::from(text),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn location(&self, offset: usize) -> (usize, usize) {
        let prefix = &self.text[..offset
            .min(self.text.len())
            .min(self.text.floor_char_boundary(offset.min(self.text.len())))];
        let row = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let col = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        (row, col)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}
