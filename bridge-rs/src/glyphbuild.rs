//! Downloads KanjiVG and the Hershey "futural" font and converts them into centre-line glyphs in a
//! 1000-unit em box (Go: internal/glyphbuild).
//!
//! KanjiVG (https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0. The generated file is a
//! derivative work and stays under CC BY-SA 3.0, which is why it is built on the user's machine
//! instead of being shipped. Hershey fonts via hersheytext (MIT).

use crate::glyphs::{Font, Glyph};
use std::collections::HashMap;
use std::io::Read;

pub const KANJIVG_URL: &str = "https://github.com/KanjiVG/kanjivg/releases/download/r20260714/kanjivg-20260714.xml.gz";
pub const HERSHEY_URL: &str = "https://unpkg.com/hersheytext@2.0.0/hersheytext.json";

const EM: f64 = 1000.0;
const KVG_BOX: f64 = 109.0;
const KVG_SCALE: f64 = EM / KVG_BOX;
/// KanjiVG's "A" spans y=14.41..92.88 (cap height) in its 109 box; Hershey's spans 1..22.
const KVG_BASELINE: f64 = 92.88 * KVG_SCALE;
const HERSHEY_SCALE: f64 = (92.88 - 14.41) * KVG_SCALE / 21.0;

// Everything except kanji is written at one smaller size, sitting on the kanji bottom line — the
// balance Japanese readers find natural in handwriting. Scales were measured on the KanjiVG data so
// that hiragana, katakana, Latin capitals and digits end up about the same height (≈75% of kanji).
// Digits get a minimum width so a narrow "1" doesn't leave gaps.
const HIRAGANA_SCALE: f64 = 0.85;
const KATAKANA_SCALE: f64 = 0.94;
const LATIN_SCALE: f64 = 0.83;
const MIN_DIGIT_WIDTH: f64 = 0.36 * EM;
const CJK_BOTTOM_LINE: f64 = 0.9 * EM;
/// Side bearing for Latin/digit glyphs; wider than raw scaling gives, or letters touch.
const LATIN_BEARING: f64 = 47.0;

/// Max distance (em units) between a curve and its polyline approximation: 0.4% of the character
/// size, well below a pixel at any size the plugin writes.
const FLATTEN_TOLERANCE: f64 = 4.0;

#[derive(Clone, Copy, Default)]
struct Pt {
    x: f64,
    y: f64,
}

/// The tokens of a path: command letters and numbers; anything else (commas, spaces, letters this
/// parser does not know) separates them and is dropped, as a pattern match over the text would.
fn tokens(d: &str) -> Vec<String> {
    let b = d.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        let c = b[i];
        if b"MmLlHhVvCcSsQqTtZz".contains(&c) {
            out.push((c as char).to_string());
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        let int_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let mut ok = false;
        if j > int_start {
            ok = true;
            if j < b.len() && b[j] == b'.' {
                j += 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
            }
        } else if j < b.len() && b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
            ok = true;
            j += 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
        }
        if !ok {
            i += 1;
            continue;
        }
        if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
            let mut k = j + 1;
            if k < b.len() && (b[k] == b'-' || b[k] == b'+') {
                k += 1;
            }
            let ds = k;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            if k > ds {
                j = k;
            }
        }
        out.push(d[start..j].to_string());
        i = j;
    }
    out
}

fn flatten_cubic(p0: Pt, p1: Pt, p2: Pt, p3: Pt, out: &mut Vec<f64>) {
    let l = (p1.x - p0.x).hypot(p1.y - p0.y) + (p2.x - p1.x).hypot(p2.y - p1.y) + (p3.x - p2.x).hypot(p3.y - p2.y);
    let n = ((l / FLATTEN_TOLERANCE).sqrt().ceil() as i64).max(1);
    for k in 1..=n {
        let t = k as f64 / n as f64;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push(a * p0.x + b * p1.x + c * p2.x + d * p3.x);
        out.push(a * p0.y + b * p1.y + c * p2.y + d * p3.y);
    }
}

/// Flattens an SVG path into polylines (one per subpath), scaling every coordinate by `scale`.
/// Supports the commands KanjiVG and Hershey use: M L H V C S Q T Z.
pub fn path_to_polylines(d: &str, scale: f64) -> Result<Vec<Vec<f64>>, String> {
    let toks = tokens(d);
    let (mut lines, mut cur): (Vec<Vec<f64>>, Vec<f64>) = (vec![], vec![]);
    let (mut pos, mut start, mut last_ctrl) = (Pt::default(), Pt::default(), Pt::default());
    let (mut cmd, mut prev_cmd) = (0u8, 0u8);
    let mut i = 0;
    macro_rules! num {
        () => {{
            if i >= toks.len() {
                return Err(format!("path {d:?}: missing number"));
            }
            let v: f64 = toks[i].parse().map_err(|e| format!("path {d:?}: {e}"))?;
            i += 1;
            v * scale
        }};
    }
    macro_rules! pt {
        ($rel:expr) => {{
            let (mut x, mut y) = (num!(), num!());
            if $rel {
                x += pos.x;
                y += pos.y;
            }
            Pt { x, y }
        }};
    }
    fn flush(cur: &mut Vec<f64>, lines: &mut Vec<Vec<f64>>) {
        if cur.len() >= 4 {
            lines.push(std::mem::take(cur));
        }
        cur.clear();
    }
    while i < toks.len() {
        let t = toks[i].as_bytes();
        if t.len() == 1 && t[0].is_ascii_alphabetic() {
            cmd = t[0];
            i += 1;
        } else if cmd | 0x20 == b'z' || cmd == 0 {
            return Err(format!("path {d:?}: number without a command"));
        } else if cmd == b'M' {
            cmd = b'L'; // implicit lineto after moveto
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd >= b'a';
        match cmd | 0x20 {
            b'm' => {
                flush(&mut cur, &mut lines);
                pos = pt!(rel);
                start = pos;
                cur = vec![pos.x, pos.y];
            }
            b'l' => {
                pos = pt!(rel);
                cur.extend([pos.x, pos.y]);
            }
            b'h' => {
                let mut x = num!();
                if rel {
                    x += pos.x;
                }
                pos.x = x;
                cur.extend([pos.x, pos.y]);
            }
            b'v' => {
                let mut y = num!();
                if rel {
                    y += pos.y;
                }
                pos.y = y;
                cur.extend([pos.x, pos.y]);
            }
            b'c' | b's' => {
                let c1 = if cmd | 0x20 == b'c' {
                    pt!(rel)
                } else if prev_cmd | 0x20 == b'c' || prev_cmd | 0x20 == b's' {
                    Pt { x: 2.0 * pos.x - last_ctrl.x, y: 2.0 * pos.y - last_ctrl.y }
                } else {
                    pos
                };
                let c2 = pt!(rel);
                let end = pt!(rel);
                flatten_cubic(pos, c1, c2, end, &mut cur);
                last_ctrl = c2;
                pos = end;
            }
            b'q' | b't' => {
                let q = if cmd | 0x20 == b'q' {
                    pt!(rel)
                } else if prev_cmd | 0x20 == b'q' || prev_cmd | 0x20 == b't' {
                    Pt { x: 2.0 * pos.x - last_ctrl.x, y: 2.0 * pos.y - last_ctrl.y }
                } else {
                    pos
                };
                let end = pt!(rel);
                let c1 = Pt { x: pos.x + 2.0 / 3.0 * (q.x - pos.x), y: pos.y + 2.0 / 3.0 * (q.y - pos.y) };
                let c2 = Pt { x: end.x + 2.0 / 3.0 * (q.x - end.x), y: end.y + 2.0 / 3.0 * (q.y - end.y) };
                flatten_cubic(pos, c1, c2, end, &mut cur);
                last_ctrl = q;
                pos = end;
            }
            b'z' => {
                cur.extend([start.x, start.y]);
                pos = start;
            }
            _ => return Err(format!("path {d:?}: unsupported command {:?}", cmd as char)),
        }
        prev_cmd = cmd;
    }
    flush(&mut cur, &mut lines);
    Ok(lines)
}

fn is_full_width(c: char) -> bool {
    let r = c as u32;
    r >= 0x2e80 && !(0xff61..=0xffdc).contains(&r)
}

fn x_range(strokes: &[Vec<f64>]) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in strokes {
        for x in s.iter().step_by(2) {
            lo = lo.min(*x);
            hi = hi.max(*x);
        }
    }
    (lo, hi)
}

fn shifted(strokes: &[Vec<f64>], dx: f64) -> Vec<Vec<i16>> {
    strokes.iter().map(|s| s.iter().enumerate().map(|(j, v)| (if j % 2 == 0 { v + dx } else { *v }).round() as i16).collect()).collect()
}

/// Shifts a glyph so its ink starts at `bearing` and gives it a matching advance.
fn proportional(strokes: &[Vec<f64>], bearing: f64) -> Glyph {
    let (lo, hi) = x_range(strokes);
    Glyph { advance: (hi - lo + 2.0 * bearing).round() as i16, strokes: shifted(strokes, bearing - lo) }
}

fn half_width(mut strokes: Vec<Vec<f64>>, c: char) -> Glyph {
    for s in strokes.iter_mut() {
        for i in (0..s.len()).step_by(2) {
            s[i] *= LATIN_SCALE;
            s[i + 1] = KVG_BASELINE - (KVG_BASELINE - s[i + 1]) * LATIN_SCALE;
        }
    }
    if c.is_ascii_digit() {
        let (lo, hi) = x_range(&strokes);
        let adv = MIN_DIGIT_WIDTH.max(hi - lo + 2.0 * LATIN_BEARING);
        return Glyph { advance: adv.round() as i16, strokes: shifted(&strokes, (adv - (hi - lo)) / 2.0 - lo) };
    }
    let g = proportional(&strokes, LATIN_BEARING);
    // Narrow letters (i, l, t…) and tight pairs (a-u, l-a) read as blobs; keep a minimum pitch.
    let min = (0.3 * EM) as i16;
    if g.advance < min && c != ' ' {
        let pad = (min - g.advance) as f64 / 2.0;
        let back: Vec<Vec<f64>> = g.strokes.iter().map(|s| s.iter().map(|v| *v as f64).collect()).collect();
        return Glyph { advance: min, strokes: shifted(&back, pad) };
    }
    g
}

/// Keeps CJK glyphs near-square but tightens narrow ones (kana, punctuation) around their ink so
/// lines read like natural Japanese handwriting.
fn full_width(mut strokes: Vec<Vec<f64>>, c: char) -> Glyph {
    let r = c as u32;
    let scale = match r {
        0x3041..=0x309f => HIRAGANA_SCALE,
        0x30a1..=0x30ff => KATAKANA_SCALE,
        _ => 1.0,
    };
    if scale != 1.0 {
        let (lo, hi) = x_range(&strokes);
        let cx = (lo + hi) / 2.0;
        for s in strokes.iter_mut() {
            for i in (0..s.len()).step_by(2) {
                s[i] = cx + (s[i] - cx) * scale;
                s[i + 1] = CJK_BOTTOM_LINE - (CJK_BOTTOM_LINE - s[i + 1]) * scale;
            }
        }
    }
    let is_punct = (0x3000..=0x303f).contains(&r) || (0xff00..=0xff0f).contains(&r);
    let (lo, hi) = x_range(&strokes);
    let min_w = if is_punct { 0.55 * EM } else { 0.8 * EM };
    let w = min_w.max(EM.min(hi - lo + 180.0)).round();
    let dx = if is_punct { 40.0 - lo } else { (w - (hi - lo)) / 2.0 - lo };
    Glyph { advance: w as i16, strokes: shifted(&strokes, dx) }
}

/// The `d` of every `<path …>` inside `body`.
fn path_ds(body: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = body;
    while let Some(i) = rest.find("<path ") {
        let tag_rest = &rest[i + 6..];
        let tag_end = tag_rest.find('>').unwrap_or(tag_rest.len());
        let tag = &tag_rest[..tag_end];
        // the attribute named d (a word boundary before it), as `\bd="…"`
        let mut from = 0;
        while let Some(j) = tag[from..].find("d=\"") {
            let at = from + j;
            if at == 0 || !tag.as_bytes()[at - 1].is_ascii_alphanumeric() && tag.as_bytes()[at - 1] != b'_' {
                if let Some(end) = tag[at + 3..].find('"') {
                    if end > 0 {
                        out.push(&tag[at + 3..at + 3 + end]);
                    }
                }
                break;
            }
            from = at + 3;
        }
        rest = &tag_rest[tag_end..];
    }
    out
}

pub fn add_kanjivg(font: &mut Font, xml: &str) -> Result<(), String> {
    const OPEN: &str = "<kanji id=\"kvg:kanji_";
    let mut rest = xml;
    while let Some(i) = rest.find(OPEN) {
        let after = &rest[i + OPEN.len()..];
        let hex_len = after.bytes().take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)).count();
        let Some(close) = after.find("</kanji>") else { break };
        if hex_len > 0 && after[hex_len..].starts_with("\">") {
            let body = &after[hex_len + 2..close.max(hex_len + 2)];
            if let Some(cp) = u32::from_str_radix(&after[..hex_len], 16).ok().and_then(|v| char::from_u32(v).map(|c| (v, c))) {
                let mut strokes = vec![];
                for d in path_ds(body) {
                    strokes.extend(path_to_polylines(d, KVG_SCALE).map_err(|e| format!("KanjiVG U+{:04X}: {e}", cp.0))?);
                }
                if !strokes.is_empty() {
                    let g = if is_full_width(cp.1) { full_width(strokes, cp.1) } else { half_width(strokes, cp.1) };
                    font.glyphs.insert(cp.1, g);
                }
            }
        }
        rest = &after[close + 8..];
    }
    Ok(())
}

pub fn add_hershey(font: &mut Font, js: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Char {
        #[serde(default)]
        d: String,
    }
    #[derive(serde::Deserialize, Default)]
    struct Futural {
        #[serde(default)]
        chars: Vec<Char>,
    }
    #[derive(serde::Deserialize)]
    struct Data {
        #[serde(default)]
        futural: Futural,
    }
    let data: Data = serde_json::from_str(js).map_err(|e| format!("Hershey: {e}"))?;
    for (i, c) in data.futural.chars.iter().enumerate() {
        let ch = char::from_u32(33 + i as u32).ok_or("Hershey: too many characters")?;
        if font.glyphs.contains_key(&ch) || c.d.is_empty() {
            continue;
        }
        let mut lines = path_to_polylines(&c.d, 1.0).map_err(|e| format!("Hershey {ch:?}: {e}"))?;
        // Hershey: baseline at y=22, cap height 21 units. Map onto KanjiVG's Latin metrics.
        for s in lines.iter_mut() {
            for j in (0..s.len()).step_by(2) {
                s[j] *= HERSHEY_SCALE;
                s[j + 1] = KVG_BASELINE - (22.0 - s[j + 1]) * HERSHEY_SCALE;
            }
        }
        font.glyphs.insert(ch, half_width(lines, ch));
    }
    Ok(())
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut resp = ureq::get(url).call().map_err(|e| format!("GET {url}: {e}"))?;
    let mut out = Vec::new();
    resp.body_mut().as_reader().take(512 << 20).read_to_end(&mut out).map_err(|e| format!("GET {url}: {e}"))?;
    Ok(out)
}

/// Downloads both sources and returns the merged font.
pub fn build() -> Result<Font, String> {
    let mut font = Font {
        em: EM as i16,
        sources: vec!["KanjiVG r20260714 (https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0".into(), "Hershey futural via hersheytext 2.0.0 (MIT)".into()],
        glyphs: HashMap::new(),
    };
    let gz = download(KANJIVG_URL)?;
    let mut xml = String::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut xml).map_err(|e| format!("KanjiVG: {e}"))?;
    add_kanjivg(&mut font, &xml)?;
    let js = String::from_utf8(download(HERSHEY_URL)?).map_err(|e| format!("Hershey: {e}"))?;
    add_hershey(&mut font, &js)?;
    Ok(font)
}

/// `spellink-bridge build-glyphs`: downloads the sources and writes the glyph file.
pub fn command(store: &crate::config::Store) -> Result<(), String> {
    let file = store.glyph_file();
    println!("Downloading KanjiVG and Hershey fonts…");
    let font = build()?;
    let mut buf = Vec::new();
    crate::glyphs::write_font(&font, &mut buf).map_err(|e| e.to_string())?;
    if let Some(dir) = file.parent() {
        crate::util::create_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&file, &buf).map_err(|e| format!("{}: {e}", file.display()))?;
    println!("Wrote {}: {} glyphs, {} KB", file.display(), font.glyphs.len(), buf.len() / 1024);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(got: &[f64], want: &[f64]) {
        assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "got {got:?}, want {want:?}");
    }

    #[test]
    fn lines_and_implicit_lineto() {
        let lines = path_to_polylines("M4,1 L4,22 M4,1 L13,1 16,2", 1.0).unwrap();
        assert_eq!(lines.len(), 2, "want 2 subpaths");
        near(&lines[1], &[4.0, 1.0, 13.0, 1.0, 16.0, 2.0]);
    }

    #[test]
    fn relative_cubic_with_packed_numbers() {
        // KanjiVG style: negative numbers act as separators.
        let lines = path_to_polylines("M54.5,15.79c0,6.07-0.29,55.49-0.29,60.55", 1.0).unwrap();
        let l = &lines[0];
        near(&l[l.len() - 2..], &[54.21, 76.34]);
        near(&l[..2], &[54.5, 15.79]);
    }

    #[test]
    fn a_smooth_cubic_reflects_the_control_point() {
        let lines = path_to_polylines("M0,0 C0,10 10,10 10,0 S20,-10 20,0", 2.0).unwrap();
        let l = &lines[0];
        near(&l[l.len() - 2..], &[40.0, 0.0]);
    }

    #[test]
    fn garbage_is_rejected() {
        for d in ["1,2", "M0,0 Z 5,5", "M0,0 A1,1 0 0 0 2,2"] {
            assert!(path_to_polylines(d, 1.0).is_err(), "{d:?}: expected an error");
        }
    }

    #[test]
    fn numbers_with_exponents_and_signs_are_read() {
        near(&path_to_polylines("M1e1,-2.5 L.5,+3", 1.0).unwrap()[0], &[10.0, -2.5, 0.5, 3.0]);
    }

    const KVG_SAMPLE: &str = r#"<kanjivg>
<kanji id="kvg:kanji_00041"><g><path id="a" d="M54.46,14.41c-8.67,19.8-31.28,70.81-34.46,78.47"/>
<path id="b" d="M32.18,66.14c12.28,0,33.91,0,44.02,0"/></g></kanji>
<kanji id="kvg:kanji_00031"><g><path id="c" d="M45,15 L45,90"/></g></kanji>
<kanji id="kvg:kanji_03042"><g><path id="d" d="M30,30 L80,30 M55,15 L55,95"/></g></kanji>
<kanji id="kvg:kanji_30a2"><g><path id="e" d="M30,30 L80,30"/></g></kanji>
<kanji id="kvg:kanji_65e5"><g><path id="f" d="M30,20 L80,20 L80,95 L30,95 Z"/></g></kanji>
</kanjivg>"#;

    fn sample() -> Font {
        let mut f = Font { em: 1000, sources: vec![], glyphs: HashMap::new() };
        add_kanjivg(&mut f, KVG_SAMPLE).unwrap();
        f
    }

    fn height(g: &Glyph) -> i16 {
        let ys: Vec<i16> = g.strokes.iter().flat_map(|s| s.iter().skip(1).step_by(2).copied()).collect();
        ys.iter().max().unwrap() - ys.iter().min().unwrap()
    }

    #[test]
    fn kanjivg_parsing() {
        let f = sample();
        for c in ['A', '1', 'あ', 'ア', '日'] {
            assert!(f.glyphs.contains_key(&c), "missing glyph {c:?}");
        }
        assert_eq!(f.glyphs[&'A'].strokes.len(), 2, "A should keep its 2 strokes in order");
    }

    #[test]
    fn kana_are_smaller_than_kanji_on_the_same_bottom_line() {
        let f = sample();
        assert!(height(&f.glyphs[&'あ']) < height(&f.glyphs[&'日']), "hiragana should be shorter than kanji");
    }

    #[test]
    fn digits_have_a_minimum_width_and_latin_gets_bearing() {
        let f = sample();
        assert!(f.glyphs[&'1'].advance >= MIN_DIGIT_WIDTH as i16, "a narrow '1' must keep a minimum pitch");
        assert!(f.glyphs[&'A'].advance > 0);
    }

    #[test]
    fn hershey_adds_symbols_without_overriding_kanjivg() {
        let mut f = sample();
        let before = f.glyphs[&'A'].strokes.len();
        add_hershey(&mut f, r#"{"futural":{"chars":[{"d":"M5,1 L5,15 M5,20 L4,21"},{"d":"M9,1 L1,22"}]}}"#).unwrap();
        assert!(f.glyphs.contains_key(&'!'), "Hershey '!' (U+0021) should have been added");
        assert_eq!(f.glyphs[&'A'].strokes.len(), before, "existing glyphs must win over Hershey");
        assert!(add_hershey(&mut f, "not json").is_err(), "bad JSON must be an error");
    }

    #[test]
    fn a_path_in_the_sources_that_cannot_be_read_names_the_character() {
        let mut f = Font { em: 1000, sources: vec![], glyphs: HashMap::new() };
        let e = add_kanjivg(&mut f, r#"<kanji id="kvg:kanji_65e5"><g><path d="M0,0 A1,1 0 0 0 2,2"/></g></kanji>"#).unwrap_err();
        assert!(e.contains("U+65E5"), "{e}");
    }

    /// The same sample through the Go builder gave these glyphs (bridge/internal/glyphbuild).
    #[test]
    fn the_sample_matches_what_the_go_builder_makes() {
        let f = sample();
        let go: Vec<(char, i16, Vec<Vec<i16>>)> = serde_json::from_str::<Vec<(String, i16, Vec<Vec<i16>>)>>(include_str!("../testdata/glyphbuild-sample.json")).unwrap().into_iter().map(|(c, a, s)| (c.chars().next().unwrap(), a, s)).collect();
        for (c, adv, strokes) in go {
            let g = &f.glyphs[&c];
            assert!(g.advance == adv && g.strokes == strokes, "{c:?} differs from the Go builder: {} {:?} vs {adv} {strokes:?}", g.advance, g.strokes);
        }
    }
}
