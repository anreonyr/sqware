use core::fmt::Write;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use stanza::renderer::Renderer;
use stanza::renderer::console::{Console, Decor};
use stanza::style::{MaxWidth, MinWidth, Styles};
use stanza::table::{Cell, Col, Content, Row, Table};

use crate::runtime::diagnose::report::{Paragraph, Report};

pub fn fixed(w: usize) -> Styles {
    Styles::default().with(MinWidth(w)).with(MaxWidth(w))
}

pub fn fixed_table(widths: &[usize]) -> Table {
    Table::default().with_cols(widths.iter().map(|&w| Col::new(fixed(w))).collect())
}

pub fn cell(s: &str, w: usize) -> Cell {
    Cell::new(Styles::default(), Content::Label(trunc(s, w)))
}

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        s.chars().take(w).collect()
    }
}

pub fn render_table(t: &Table) -> String {
    let decor = Decor::default()
        .suppress_escape_codes()
        .suppress_outer_border()
        .suppress_inner_horizontal_border()
        .suppress_all_lines();
    Console(decor).render(t)
}

pub fn render(r: &Report, sink: &mut impl Write, indent: usize) {
    for p in &r.paras {
        if let Some(t) = &p.title {
            let _ = writeln!(sink, "{t}");
            let _ = writeln!(sink);
        }
        let mut ind = Indented::new(sink, indent);
        let _ = ind.write_str(&render_paragraph(p));
        let _ = writeln!(sink);
    }
}

fn render_paragraph(p: &Paragraph) -> String {
    let cols = p.items.iter().map(|row| row.len()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            p.items
                .iter()
                .filter_map(|row| row.get(c).and_then(|s| s.as_deref()))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut t = fixed_table(&widths);
    for row in &p.items {
        let cells: Vec<Cell> = (0..cols)
            .map(|c| {
                let s = row.get(c).and_then(|s| s.as_deref()).unwrap_or("");
                cell(s, widths[c])
            })
            .collect();
        t = t.with_row(Row::new(Styles::default(), cells));
    }
    render_table(&t)
}

struct Indented<'a, W: Write> {
    out: &'a mut W,
    at_bol: bool,
    indent: usize,
}

impl<'a, W: Write> Indented<'a, W> {
    fn new(out: &'a mut W, indent: usize) -> Self {
        Self {
            out,
            at_bol: true,
            indent,
        }
    }
}

impl<W: Write> Write for Indented<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut rest = s;
        while let Some(i) = rest.find('\n') {
            if self.at_bol {
                for _ in 0..self.indent {
                    self.out.write_char(' ')?;
                }
                self.at_bol = false;
            }
            if i > 0 {
                self.out.write_str(&rest[..i])?;
            }
            self.out.write_char('\n')?;
            self.at_bol = true;
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            if self.at_bol {
                for _ in 0..self.indent {
                    self.out.write_char(' ')?;
                }
                self.at_bol = false;
            }
            self.out.write_str(rest)?;
        }
        Ok(())
    }
}
