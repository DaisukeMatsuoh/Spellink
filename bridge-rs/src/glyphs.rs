//! The glyph file written by the Go bridge (`bridge/internal/ink/glyphs.go`): gzip, magic
//! "CLGF\x07", little-endian, points after the first of each stroke stored as deltas.

use std::collections::HashMap;
use std::io::{self, Read};

const MAGIC: &[u8] = b"CLGF\x07";

pub struct Glyph {
    pub advance: i16,
    pub strokes: Vec<Vec<i16>>,
}

pub struct Font {
    pub em: i16,
    pub sources: Vec<String>,
    pub glyphs: HashMap<char, Glyph>,
}

#[derive(Debug)]
pub enum FontError {
    Format,
    Corrupt(String),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FontError::Format => write!(f, "not a glyph file of the current format"),
            FontError::Corrupt(e) => write!(f, "corrupt glyph file: {e}"),
        }
    }
}

struct Reader<R: Read>(R);

impl<R: Read> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut b = [0u8; N];
        self.0.read_exact(&mut b)?;
        Ok(b)
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.bytes()?))
    }
    fn i16(&mut self) -> io::Result<i16> {
        Ok(i16::from_le_bytes(self.bytes()?))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
}

pub fn read_font<R: Read>(r: R) -> Result<Font, FontError> {
    let mut rd = Reader(flate2::read::GzDecoder::new(r));
    let mut head = [0u8; 5];
    if rd.0.read_exact(&mut head).is_err() || head != MAGIC {
        return Err(FontError::Format);
    }
    let corrupt = |e: io::Error| FontError::Corrupt(e.to_string());
    let em = rd.i16().map_err(corrupt)?;
    let n_sources = rd.u16().map_err(corrupt)?;
    let mut sources = Vec::with_capacity(n_sources as usize);
    for _ in 0..n_sources {
        let n = rd.u16().map_err(corrupt)? as usize;
        let mut b = vec![0u8; n];
        rd.0.read_exact(&mut b).map_err(corrupt)?;
        sources.push(String::from_utf8_lossy(&b).into_owned());
    }
    let count = rd.u32().map_err(corrupt)?;
    let mut glyphs = HashMap::with_capacity(count as usize);
    for _ in 0..count {
        let cp = rd.u32().map_err(corrupt)?;
        let advance = rd.i16().map_err(corrupt)?;
        let n_strokes = rd.u16().map_err(corrupt)?;
        let mut strokes = Vec::with_capacity(n_strokes as usize);
        for _ in 0..n_strokes {
            let n_pts = rd.u16().map_err(corrupt)? as usize;
            let mut s = Vec::with_capacity(n_pts * 2);
            for _ in 0..n_pts * 2 {
                s.push(rd.i16().map_err(corrupt)?);
            }
            for i in 2..s.len() {
                s[i] = s[i].wrapping_add(s[i - 2]); // undelta
            }
            strokes.push(s);
        }
        // A code point that is not a char (a surrogate) cannot be written by the Go side either.
        let ch = char::from_u32(cp).ok_or_else(|| FontError::Corrupt(format!("code point {cp:#x}")))?;
        glyphs.insert(ch, Glyph { advance, strokes });
    }
    Ok(Font { em, sources, glyphs })
}
