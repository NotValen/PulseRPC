//! Standalone icon generator (no dependencies).
//!
//!   rustc -O tools/make_icon.rs -o tools/make_icon.exe
//!   tools/make_icon.exe
//!
//! Draws a rounded gradient tile with a heartbeat trace, supersampled 6x with
//! proper straight-alpha compositing, and packs PNG entries into a
//! multi-resolution .ico (16 … 256 px). Also writes a 256 px preview PNG.

use std::fs;
use std::io::Write;

const SS: u32 = 6;
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Diagonal gradient, top-left to bottom-right.
const TOP: [f32; 3] = [91.0, 124.0, 250.0]; // #5B7CFA
const BOTTOM: [f32; 3] = [192.0, 75.0, 240.0]; // #C04BF0

/// Heartbeat polyline in 0..1 tile coordinates.
const PULSE: [(f32, f32); 7] = [
    (0.09, 0.55),
    (0.31, 0.55),
    (0.41, 0.66),
    (0.51, 0.27),
    (0.67, 0.73),
    (0.75, 0.55),
    (0.91, 0.55),
];

fn clamp_safe(value: f32, low: f32, high: f32) -> f32 {
    if high <= low {
        (low + high) * 0.5
    } else {
        value.clamp(low, high)
    }
}

fn in_rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, px: f32, py: f32) -> bool {
    if px < x || py < y || px > x + w || py > y + h {
        return false;
    }
    let cx = clamp_safe(px, x + radius, x + w - radius);
    let cy = clamp_safe(py, y + radius, y + h - radius);
    let corner = (px < x + radius || px > x + w - radius)
        && (py < y + radius || py > y + h - radius);
    if !corner {
        return true;
    }
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Shortest distance from (px, py) to the heartbeat polyline.
fn pulse_distance(px: f32, py: f32, n: f32) -> f32 {
    let mut best = f32::MAX;
    for pair in PULSE.windows(2) {
        let (x0, y0) = (pair[0].0 * n, pair[0].1 * n);
        let (x1, y1) = (pair[1].0 * n, pair[1].1 * n);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = dx * dx + dy * dy;
        let t = if len2 <= f32::EPSILON {
            0.0
        } else {
            (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
        };
        let ex = x0 + dx * t - px;
        let ey = y0 + dy * t - py;
        let d = (ex * ex + ey * ey).sqrt();
        if d < best {
            best = d;
        }
    }
    best
}

fn render(size: u32) -> Vec<u8> {
    let n = (size * SS) as f32;
    let radius = 0.225 * n;
    // Keep the trace at least ~1.8 device px wide so 16 px stays readable.
    let inner = (0.044 * n).max(5.4);
    let glow = inner + 0.055 * n;
    let glow_alpha = if size <= 20 { 0.0 } else { 0.26 };
    let samples = (SS * SS) as f32;

    let mut out = vec![0u8; (size * size * 4) as usize];

    for oy in 0..size {
        for ox in 0..size {
            let mut ar = 0f32;
            let mut ag = 0f32;
            let mut ab = 0f32;
            let mut covered = 0f32;

            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (ox * SS + sx) as f32 + 0.5;
                    let py = (oy * SS + sy) as f32 + 0.5;
                    if !in_rounded_rect(0.0, 0.0, n, n, radius, px, py) {
                        continue;
                    }

                    // Diagonal gradient plus a soft sheen near the top edge.
                    let t = ((px * 0.62 + py * 0.38) / n).clamp(0.0, 1.0);
                    let mut r = lerp(TOP[0], BOTTOM[0], t);
                    let mut g = lerp(TOP[1], BOTTOM[1], t);
                    let mut b = lerp(TOP[2], BOTTOM[2], t);
                    let sheen = (1.0 - (py / n) * 2.4).max(0.0) * 0.07;
                    r += (255.0 - r) * sheen;
                    g += (255.0 - g) * sheen;
                    b += (255.0 - b) * sheen;

                    // Heartbeat: solid white core with a soft glow around it.
                    let d = pulse_distance(px, py, n);
                    let white = if d <= inner {
                        1.0
                    } else if d < glow {
                        let k = 1.0 - (d - inner) / (glow - inner);
                        glow_alpha * k * k
                    } else {
                        0.0
                    };
                    if white > 0.0 {
                        r += (255.0 - r) * white;
                        g += (255.0 - g) * white;
                        b += (255.0 - b) * white;
                    }

                    ar += r;
                    ag += g;
                    ab += b;
                    covered += 1.0;
                }
            }

            let idx = ((oy * size + ox) * 4) as usize;
            if covered > 0.0 {
                // Straight alpha: average colour over covered samples only, and
                // scale alpha by coverage so the tile edge blends cleanly.
                out[idx] = (ar / covered).round().clamp(0.0, 255.0) as u8;
                out[idx + 1] = (ag / covered).round().clamp(0.0, 255.0) as u8;
                out[idx + 2] = (ab / covered).round().clamp(0.0, 255.0) as u8;
                out[idx + 3] = ((covered / samples) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    } else {
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + 65535).min(data.len());
            let chunk = &data[offset..end];
            let last = end == data.len();
            out.push(if last { 1 } else { 0 });
            let len = chunk.len() as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
            offset = end;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(tag.len() + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

fn png(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * height * 4) + height) as usize);
    for y in 0..height {
        raw.push(0); // filter: none
        let start = (y * width * 4) as usize;
        raw.extend_from_slice(&pixels[start..start + (width * 4) as usize]);
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA

    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&chunk(b"IDAT", &zlib_stored(&raw)));
    out.extend_from_slice(&chunk(b"IEND", b""));
    out
}

fn blank(width: u32, height: u32, bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        out.extend_from_slice(&[bg[0], bg[1], bg[2], 255]);
    }
    out
}

/// Nearest-neighbour blit with straight-alpha compositing onto an opaque canvas.
fn blit(canvas: &mut [u8], cw: u32, src: &[u8], sw: u32, factor: u32, ox: u32, oy: u32) {
    for y in 0..sw * factor {
        for x in 0..sw * factor {
            let si = (((y / factor) * sw + (x / factor)) * 4) as usize;
            let a = f32::from(src[si + 3]) / 255.0;
            let di = (((oy + y) * cw + ox + x) * 4) as usize;
            for c in 0..3 {
                let fg = f32::from(src[si + c]);
                let bg = f32::from(canvas[di + c]);
                canvas[di + c] = (fg * a + bg * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A contact sheet of the small sizes on dark and light backgrounds, so the
/// 16 px rendering can be judged without opening the .ico.
fn small_preview(images: &[(u32, Vec<u8>)], factor: u32) -> (Vec<u8>, u32, u32) {
    let shown: Vec<&(u32, Vec<u8>)> = images.iter().filter(|(s, _)| *s <= 48).collect();
    let pad = 12u32;
    let cell = 48 * factor;
    let width = shown.len() as u32 * (cell + pad) + pad;
    let height = 2 * (cell + pad) + pad;
    let mut canvas = blank(width, height, [17, 24, 39]);

    let light_row = cell + 2 * pad;
    for y in light_row..light_row + cell + pad {
        for x in 0..width {
            let di = ((y * width + x) * 4) as usize;
            canvas[di..di + 4].copy_from_slice(&[243, 244, 246, 255]);
        }
    }

    let mut ox = pad;
    for (size, pixels) in &shown {
        blit(&mut canvas, width, pixels, *size, factor, ox, pad);
        blit(&mut canvas, width, pixels, *size, factor, ox, light_row + pad);
        ox += size * factor + pad;
    }

    (canvas, width, height)
}

/// Classic 32-bit DIB icon frame (BITMAPINFOHEADER + BGRA rows + AND mask).
/// Small sizes are stored uncompressed so every Win32/GDI+ code path can read
/// them; PNG frames are only used for the large sizes.
fn dib(pixels: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&size.to_le_bytes()); // biWidth
    out.extend_from_slice(&(size * 2).to_le_bytes()); // biHeight (XOR + AND)
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(size * size * 4).to_le_bytes()); // biSizeImage
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes()); // xppm, yppm, clrUsed, clrImportant
    }

    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.push(pixels[i + 2]);
            out.push(pixels[i + 1]);
            out.push(pixels[i]);
            out.push(pixels[i + 3]);
        }
    }

    let stride = ((size + 31) / 32) * 4;
    for y in (0..size).rev() {
        let mut row = vec![0u8; stride as usize];
        for x in 0..size {
            if pixels[((y * size + x) * 4 + 3) as usize] == 0 {
                row[(x / 8) as usize] |= 1 << (7 - (x % 8));
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

fn main() {
    let root = std::env::current_dir().expect("cwd");
    let assets = if root.join("assets").is_dir() {
        root.join("assets")
    } else if root.join("..\\assets").is_dir() {
        root.join("..\\assets")
    } else {
        root.clone()
    };
    fs::create_dir_all(&assets).expect("assets dir");
    let out_path = assets.join("pulserpc.ico");

    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut raw_images: Vec<(u32, Vec<u8>)> = Vec::new();
    for size in SIZES {
        eprintln!("  rendering {size}x{size}");
        let pixels = render(size);
        if size == 256 {
            // Standalone preview for docs / review (git-ignored).
            let preview = assets.join("preview.png");
            fs::write(&preview, png(&pixels, size, size)).expect("preview");
            eprintln!("  preview   {}", preview.display());
        }
        raw_images.push((size, pixels.clone()));
        let frame = if size <= 64 {
            dib(&pixels, size)
        } else {
            png(&pixels, size, size)
        };
        images.push((size, frame));
    }

    let (sheet, sw, sh) = small_preview(&raw_images, 5);
    let sheet_path = std::env::temp_dir().join("pulserpc-small-preview.png");
    fs::write(&sheet_path, png(&sheet, sw, sh)).expect("small preview");
    eprintln!("  small     {}", sheet_path.display());

    let mut header = Vec::new();
    header.extend_from_slice(&0u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * images.len() as u32;
    let mut entries = Vec::new();
    let mut data = Vec::new();
    for (size, bytes) in &images {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        entries.extend_from_slice(&[dim, dim, 0, 0]);
        entries.extend_from_slice(&1u16.to_le_bytes());
        entries.extend_from_slice(&32u16.to_le_bytes());
        entries.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        entries.extend_from_slice(&offset.to_le_bytes());
        offset += bytes.len() as u32;
        data.extend_from_slice(bytes);
    }

    let file = fs::File::create(&out_path).expect("create ico");
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(&header).unwrap();
    writer.write_all(&entries).unwrap();
    writer.write_all(&data).unwrap();
    writer.flush().unwrap();

    eprintln!(
        "wrote {} ({} bytes)",
        out_path.display(),
        fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0)
    );
}
