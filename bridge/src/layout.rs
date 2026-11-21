//! `bridge/internal/ink/layout.go`: answer text streaming in → finished lines of strokes.
//! A line-for-line port, so that both bridges write the same handwriting for the same answer.

use crate::glyphs::Font;
use crate::handwriting::{Stroke, render_glyph};
use crate::random::Random;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub page_height: f64,
    pub bottom_margin: f64,
    pub char_size: f64,
    #[serde(default)]
    pub max_pages: i64,
    #[serde(default)]
    pub continue_top: f64,
    #[serde(default)]
    pub start_limit: f64,
    /// Set by the bridge (not from the plugin): the writer's slope and letter spacing.
    #[serde(default, skip)]
    pub slope: f64,
    #[serde(default, skip)]
    pub spacing_full: f64,
    #[serde(default, skip)]
    pub spacing_half: f64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub index: i64,
    pub page: i64,
    pub text: String,
    pub strokes: Vec<Stroke>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cont: bool,
}

const FRAGMENT_CHARS: usize = 8;
const LINE_HEIGHT: f64 = 1.45;
const SPACE_WIDTH: f64 = 0.33;
const NO_LINE_START: &str = "、。，．,.:;!?！？）」』】〕ー…‥・ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮ";
const DROPPED: &str = "*`#_\r";

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '\'' || c == '’' || c == '-'
}

#[derive(Clone, Copy)]
struct Pending {
    ch: char,
    advance: f64,
    base: f64,
}

pub struct Composer {
    font: std::sync::Arc<Font>,
    layout: Layout,
    rng: Random,
    pub missing: BTreeSet<char>,
    pub truncated: bool,
    pub pages: i64,
    line: Vec<Pending>,
    line_drawn: usize,
    line_drawn_x: f64,
    line_width: f64,
    word_start: isize,
    line_index: f64,
    page: i64,
    emitted: i64,
    at_paragraph_start: bool,
}

impl Composer {
    pub fn new(font: std::sync::Arc<Font>, mut layout: Layout, seed: u32) -> Self {
        if layout.max_pages < 1 {
            layout.max_pages = 1;
        }
        Composer {
            font,
            layout,
            rng: Random::new(seed),
            missing: BTreeSet::new(),
            truncated: false,
            pages: 1,
            line: Vec::new(),
            line_drawn: 0,
            line_drawn_x: 0.0,
            line_width: 0.0,
            word_start: -1,
            line_index: 0.0,
            page: 0,
            emitted: 0,
            at_paragraph_start: true,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Adds streamed text and returns the lines it completed.
    pub fn push(&mut self, text: &str) -> Vec<Line> {
        let mut out = Vec::new();
        for mut ch in text.chars() {
            if self.truncated {
                break;
            }
            if ('\u{ff01}'..='\u{ff5e}').contains(&ch) {
                ch = char::from_u32(ch as u32 - 0xfee0).unwrap(); // full-width ASCII → half-width
            }
            if DROPPED.contains(ch) {
            } else if ch == '\n' {
                self.flush(&mut out, true);
            } else {
                self.add(ch, &mut out);
            }
        }
        self.fragment(&mut out);
        out
    }

    /// The last, partial line.
    pub fn finish(&mut self) -> Vec<Line> {
        let mut out = Vec::new();
        self.flush(&mut out, false);
        out
    }

    fn natural(&mut self, ch: char) -> f64 {
        let size = self.layout.char_size;
        match ch {
            ' ' => return SPACE_WIDTH * size,
            '\u{3000}' => return size,
            _ => {}
        }
        match self.font.glyphs.get(&ch) {
            Some(g) => g.advance as f64 / self.font.em as f64 * size,
            None => {
                self.missing.insert(ch);
                0.5 * size
            }
        }
    }

    fn spacing(&self, ch: char) -> f64 {
        let f = if ch as u32 >= 0x2e80 { self.layout.spacing_full } else { self.layout.spacing_half };
        if f <= 0.0 { 1.0 } else { f }
    }

    fn add(&mut self, ch: char, out: &mut Vec<Line>) {
        if ch == ' ' && self.line.is_empty() {
            return; // no leading spaces on a wrapped line
        }
        let base = self.natural(ch);
        let adv = base * self.spacing(ch);
        let is_word = is_word_char(ch);
        let overflows = self.line_width + adv > self.layout.width && !self.line.is_empty();
        if overflows && !NO_LINE_START.contains(ch) {
            if is_word && self.word_start > 0 {
                // Carry the partial Latin word over to the next line.
                let carried: Vec<Pending> = self.line[self.word_start as usize..].to_vec();
                self.line.truncate(self.word_start as usize);
                self.emit(out);
                for p in carried {
                    self.append(p, true);
                }
            } else {
                self.emit(out);
                if ch == ' ' {
                    return;
                }
            }
        }
        self.append(Pending { ch, advance: adv, base }, is_word);
    }

    fn append(&mut self, p: Pending, is_word: bool) {
        if is_word {
            if self.word_start < 0 {
                self.word_start = self.line.len() as isize;
            }
        } else {
            self.word_start = -1;
        }
        self.line.push(p);
        self.line_width += p.advance;
    }

    fn fragment(&mut self, out: &mut Vec<Line>) {
        let pending = self.line.len() - self.line_drawn;
        if pending < FRAGMENT_CHARS || !self.fits() {
            return;
        }
        let end = if self.word_start >= 0 { self.word_start as usize } else { self.line.len() };
        if (end as isize - self.line_drawn as isize) < (FRAGMENT_CHARS / 2) as isize {
            return;
        }
        self.draw(out, self.line_drawn, end, true);
    }

    fn flush(&mut self, out: &mut Vec<Line>, hard_break: bool) {
        if !self.line.is_empty() {
            self.emit(out);
        } else if hard_break && !self.at_paragraph_start {
            self.line_index += 0.5;
            self.at_paragraph_start = true;
        }
    }

    fn top(&self) -> f64 {
        let start = if self.page > 0 { self.layout.continue_top } else { self.layout.y };
        start + self.line_index * self.layout.char_size * LINE_HEIGHT
    }

    fn limit(&self) -> f64 {
        page_limit(&self.layout, self.page)
    }

    fn fits(&self) -> bool {
        self.top() + self.layout.char_size <= self.limit()
    }

    fn emit(&mut self, out: &mut Vec<Line>) {
        if !self.fits() {
            if self.page + 1 >= self.layout.max_pages {
                self.truncated = true;
                self.reset_line();
                return;
            }
            self.page += 1;
            self.pages = self.page + 1;
            self.line_index = 0.0;
            self.at_paragraph_start = true;
        }
        self.draw(out, self.line_drawn, self.line.len(), false);
        self.line_index += 1.0;
        self.at_paragraph_start = false;
        self.reset_line();
    }

    fn draw(&mut self, out: &mut Vec<Line>, from: usize, to: usize, cont: bool) {
        let top = self.top();
        let mut strokes = Vec::new();
        let mut text = String::new();
        let mut x = self.layout.x + self.line_drawn_x;
        for i in from..to {
            let p = self.line[i];
            if let Some(g) = self.font.glyphs.get(&p.ch) {
                let gx = x + (p.advance - p.base) / 2.0;
                strokes.extend(render_glyph(g, self.font.em, gx, top, self.layout.char_size, &mut self.rng));
            }
            x += p.advance;
            text.push(p.ch);
        }
        if self.layout.slope != 0.0 {
            for st in &mut strokes {
                let mut k = 0;
                while k + 1 < st.p.len() {
                    st.p[k + 1] += ((st.p[k] as f64 - self.layout.x) * self.layout.slope).round() as i64;
                    k += 2;
                }
            }
        }
        out.push(Line { index: self.emitted, page: self.page, text, strokes, cont });
        self.emitted += 1;
        self.line_drawn = to;
        self.line_drawn_x = x - self.layout.x;
    }

    fn reset_line(&mut self) {
        self.line.clear();
        self.line_drawn = 0;
        self.line_drawn_x = 0.0;
        self.line_width = 0.0;
        self.word_start = -1;
    }
}

fn page_limit(l: &Layout, page: i64) -> f64 {
    let mut limit = l.page_height - l.bottom_margin;
    if page == 0 && l.start_limit > 0.0 {
        limit = f64::min(limit, l.start_limit);
    }
    limit
}

/// How many full-width characters fit on the starting page below Y.
pub fn capacity(l: &Layout) -> i64 {
    let per_line = (l.width / l.char_size).floor();
    let lines = ((page_limit(l, 0) - l.y) / (l.char_size * LINE_HEIGHT)).floor();
    f64::max(0.0, per_line * lines) as i64
}
