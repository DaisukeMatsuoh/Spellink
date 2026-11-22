//! The smaller commands of the command line (Go: main.go's pair and check.go).

use crate::app::pair_urls;
use crate::config::Store;
use crate::probe;
use std::time::Duration;

/// Prints or writes the connection file the plugin reads (MyStyle/Spellink/bridges.json).
pub fn pair(out: Option<&str>) -> Result<(), String> {
    let cfg = Store::user()?.load()?;
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "bridges": [{"name": cfg.name, "token": cfg.token, "urls": pair_urls(&cfg)}]
    }))
    .map_err(|e| e.to_string())?;
    match out {
        None | Some("") => println!("{body}"),
        Some(path) => {
            crate::util::write_private(std::path::Path::new(path), format!("{body}\n").as_bytes()).map_err(|e| format!("{path}: {e}"))?;
            println!("Wrote {path}");
        }
    }
    Ok(())
}

/// Dials the given addresses (by default every address `pair` would hand to the plugin) with this
/// bridge's token and reports, for each, whether the plugin would get through.
pub async fn check(urls: &[String]) -> Result<(), String> {
    let cfg = Store::user()?.load()?;
    let urls: Vec<String> = if urls.is_empty() { pair_urls(&cfg) } else { urls.to_vec() };
    let mut ok = 0;
    for u in &urls {
        match tokio::time::timeout(Duration::from_secs(8), probe::probe(u, &cfg.token)).await {
            Ok(Ok(r)) => {
                ok += 1;
                println!("OK  {u}\n      bridge {:?}, version {}, protocol {}, {} ms", r.bridge, r.version, r.protocol, r.elapsed.as_millis());
            }
            Ok(Err(e)) => println!("NG  {u}\n      {e}"),
            Err(_) => println!("NG  {u}\n      timed out"),
        }
    }
    if ok == 0 {
        return Err("no address could be reached".into());
    }
    Ok(())
}

/// The command line's connection to the running bridge on this computer.
fn admin_client() -> Result<crate::devicecli::Client, String> {
    let store = Store::user()?;
    let cfg = store.load()?;
    let token = store.admin_token()?;
    Ok(crate::devicecli::Client { base: format!("http://127.0.0.1:{}", cfg.port), token, open: Some(std::sync::Arc::new(|u| crate::devicecli::open_browser(u))) })
}

/// `spellink-bridge device …`.
pub async fn device(args: &[String]) -> Result<(), String> {
    let c = admin_client()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    crate::devicecli::run(
        &c,
        args,
        &mut || {
            let mut line = String::new();
            // at most 256 bytes of one line, like the Go command
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line.chars().take(256).collect()),
            }
        },
        &mut stdout,
        &cwd,
        crate::devicecli::Lang::from_env(),
    )
    .await
}

/// `spellink-bridge open`: the app page in the browser.
pub async fn open_app() -> Result<(), String> {
    let c = admin_client()?;
    crate::devicecli::open_app(&c, &mut std::io::stdout()).await
}

/// Renders text as handwriting onto Nomad-sized PNG pages, to check output without a device.
/// Returns the summary line.
pub fn render_preview(font: std::sync::Arc<crate::glyphs::Font>, text: &str, size: f64, pages: i64, out: &std::path::Path) -> Result<String, String> {
    use crate::layout::{Composer, Layout};
    const W: usize = 1404;
    const H: usize = 1872; // Nomad
    let layout = Layout {
        x: 80.0,
        y: 80.0,
        width: (W - 160) as f64,
        page_height: H as f64,
        bottom_margin: 80.0,
        char_size: size,
        max_pages: pages,
        continue_top: 80.0,
        spacing_full: 1.0,
        spacing_half: 1.15,
        slope: -(1.0f64.to_radians()).tan(), // 1° rising
        ..Layout::default()
    };
    let mut c = Composer::new(font, layout, 1);
    let mut lines = c.push(text);
    lines.extend(c.finish());
    // Pages are drawn side by side.
    let pages_drawn = (c.pages.max(1)) as usize;
    let width = W * pages_drawn;
    let mut img = vec![255u8; width * H];
    let mut points = 0;
    for l in &lines {
        for s in &l.strokes {
            points += s.p.len() / 2;
            draw_stroke(&mut img, width, H, &s.p, &s.w, (l.page as usize * W) as f64);
        }
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, H as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&img)).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(format!("{} lines on {} page(s), {points} points, truncated={}, missing={}", lines.len(), c.pages, c.truncated, c.missing.len()))
}

fn draw_stroke(img: &mut [u8], w: usize, h: usize, p: &[i64], pressure: &[i64], dx: f64) {
    let mut dot = |x: f64, y: f64, r: f64| {
        for yy in (y - r) as i64..=(y + r) as i64 {
            for xx in (x - r) as i64..=(x + r) as i64 {
                if (xx as f64 - x).powi(2) + (yy as f64 - y).powi(2) <= r * r && xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                    img[yy as usize * w + xx as usize] = 0;
                }
            }
        }
    };
    let mut i = 0;
    while i + 3 < p.len() {
        let (x0, y0) = (p[i] as f64 + dx, p[i + 1] as f64);
        let (x1, y1) = (p[i + 2] as f64 + dx, p[i + 3] as f64);
        let r = 0.6 + pressure.get(i / 2).copied().unwrap_or(0) as f64 / 4096.0 * 1.6;
        let n = ((x1 - x0).hypot(y1 - y0).ceil() as i64).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            dot(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r);
        }
        i += 2;
    }
}

/// `spellink-bridge preview TEXT OUT.png [charSize] [maxPages]`.
pub fn preview(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("preview TEXT OUT.png [charSize] [maxPages]".into());
    }
    let size = args.get(2).map_or(44.0, |s| s.parse().unwrap_or(0.0));
    let pages = args.get(3).map_or(1, |s| s.parse().unwrap_or(0));
    let font = crate::app::load_font(&Store::user()?)?;
    println!("{}", render_preview(font, &args[0], size, pages, std::path::Path::new(&args[1]))?);
    Ok(())
}

/// Writes a tiny glyph file — every hiragana, digit and ASCII letter as a two-stroke mark — so that
/// tests can run a bridge without downloading the real glyph data. It is for the plugin's
/// integration tests (plugin/__tests__/integration.test.ts), not for use.
pub fn testfont(args: &[String]) -> Result<(), String> {
    use crate::glyphs::{Font, Glyph};
    let [out] = args else { return Err("usage: spellink-bridge testfont OUT".into()) };
    let mut f = Font { em: 100, sources: vec![], glyphs: std::collections::HashMap::new() };
    let mut mark = |c: char, full: bool| {
        f.glyphs.insert(c, Glyph { advance: if full { 100 } else { 55 }, strokes: vec![vec![10, 10, 60, 10, 60, 90], vec![10, 50, 50, 50]] });
    };
    for c in 'ぁ'..='ん' {
        mark(c, true);
    }
    for c in '0'..='9' {
        mark(c, false);
    }
    for c in 'a'..='z' {
        mark(c, false);
        mark(c.to_ascii_uppercase(), false);
    }
    for c in "。、？！：:.,?!+-=".chars() {
        mark(c, c as u32 > 0x7f);
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{out}: {e}"))?;
    crate::glyphs::write_font(&f, std::io::BufWriter::new(file)).map_err(|e| format!("{out}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_is_a_png_with_ink_on_it() {
        let font = std::sync::Arc::new(crate::glyphs::read_font(std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/glyphs.bin")).unwrap()).unwrap());
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("p.png");
        let summary = render_preview(font, "こんにちは。Hello 123", 44.0, 1, &out).unwrap();
        assert!(summary.contains("1 page(s)") && summary.contains("missing=0"), "{summary}");
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&out).unwrap()));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (1404, 1872));
        let dark = buf[..info.buffer_size()].iter().filter(|b| **b == 0).count();
        assert!(dark > 500 && dark < 400_000, "{dark} dark pixels");
    }
}
