use core::fmt::Write;

use crate::runtime::diagnose::report::{Paragraph, Report};

pub fn render(r: &Report, sink: &mut impl Write, indent: usize) {
    for p in &r.paras {
        if let Some(t) = &p.title {
            let _ = writeln!(sink, "{t}");
            let _ = writeln!(sink);
        }
        let mut ind = Indented::new(sink, indent);
        let _ = render_paragraph(p, &mut ind);
        let _ = writeln!(sink);
    }
}

fn render_paragraph(p: &Paragraph, sink: &mut impl Write) -> core::fmt::Result {
    let cols = p.items.iter().map(|row| row.len()).max().unwrap_or(0);
    for row in &p.items {
        let height = row.iter().filter_map(|cell| cell.as_deref())
            .map(|text| text.lines().count()).max().unwrap_or(0).max(1);
        for line in 0..height {
            for col in 0..cols {
                let text = row.get(col).and_then(|cell| cell.as_deref())
                    .and_then(|text| text.lines().nth(line)).unwrap_or("");
                sink.write_str(text)?;
                if col + 1 < cols {
                    let width = p.items.iter()
                        .filter_map(|row| row.get(col).and_then(|cell| cell.as_deref()))
                        .flat_map(|text| text.lines()).map(|text| text.chars().count())
                        .max().unwrap_or(0);
                    for _ in 0..width.saturating_sub(text.chars().count()) + 2 {
                        sink.write_char(' ')?;
                    }
                }
            }
            sink.write_char('\n')?;
        }
    }
    Ok(())
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
