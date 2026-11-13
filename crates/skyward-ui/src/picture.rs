//! A picture a page shows, as the browser draws it: PNG read into straight
//! RGBA, and put on a canvas at whatever width the page gives it.
//!
//! Only PNG is read: it is the one format the recovery already carries a
//! reader for, and a page's pictures are taken only where a browser here can
//! show them. A picture too big to keep is not shown at all, and the page
//! says so by leaving it out.

use crate::canvas::Canvas;
use crate::color::Rgba;

/// A picture read, its rows top to bottom, red, green, blue and alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Most pixels one picture may hold: what the recovery can keep of a page's
/// pictures and still have room for the page.
const MOST_PIXELS: u32 = 2_000_000;

/// Read `bytes` as a PNG picture; none where it is not one, or is more than
/// the recovery keeps.
pub fn read_png(bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // A palette is turned into colours and a bit depth under eight into
    // eight, so that every picture comes out as rows of four bytes.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (frame.width, frame.height);
    if w == 0 || h == 0 || w.saturating_mul(h) > MOST_PIXELS {
        return None;
    }
    let line = frame.line_size;
    let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for row in buf.chunks(line).take(h as usize) {
        match frame.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(&row[..(w as usize) * 4]),
            png::ColorType::Rgb => {
                for px in row[..(w as usize) * 3].as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            png::ColorType::Grayscale => {
                for &g in &row[..w as usize] {
                    rgba.extend_from_slice(&[g, g, g, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for px in row[..(w as usize) * 2].as_chunks::<2>().0 {
                    rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
            }
            // A palette is turned into colours before it gets here.
            png::ColorType::Indexed => return None,
        }
    }
    if rgba.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    Some(Picture { w, h, rgba })
}

/// Draw `picture` with its top-left corner at (`x`, `y`), as wide as `w` and
/// as tall as `h`: each pixel of the canvas takes the colour of the picture's
/// pixel it stands on.
pub fn draw(c: &mut Canvas, picture: &Picture, x: f32, y: f32, w: f32, h: f32) {
    let (x0, y0) = (x.round() as i32, y.round() as i32);
    let dw = w.round().max(1.0) as i32;
    let dh = h.round().max(1.0) as i32;
    let (pw, ph) = (picture.w as i64, picture.h as i64);
    for row in 0..dh {
        let sy = row as i64 * ph / dh as i64;
        for col in 0..dw {
            let sx = col as i64 * pw / dw as i64;
            let at = ((sy * pw + sx) * 4) as usize;
            let px = &picture.rgba[at..at + 4];
            if px[3] == 0 {
                continue;
            }
            c.blend(x0 + col, y0 + row, Rgba { r: px[0], g: px[1], b: px[2], a: px[3] }, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 PNG: red, green, blue, and a pixel half transparent.
    fn sample() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128])
                .unwrap();
            writer.finish().unwrap();
        }
        out
    }

    #[test]
    fn a_png_comes_out_as_its_pixels() {
        let picture = read_png(&sample()).expect("a picture");
        assert_eq!((picture.w, picture.h), (2, 2));
        assert_eq!(&picture.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&picture.rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&picture.rgba[12..16], &[255, 255, 255, 128]);
    }

    #[test]
    fn what_is_not_a_png_is_no_picture() {
        assert!(read_png(b"not a picture at all").is_none());
    }

    #[test]
    fn a_picture_is_put_where_the_page_gives_it_room() {
        let picture = read_png(&sample()).unwrap();
        let mut c = Canvas::filled(8, 8, Rgba::rgb(255, 255, 255));
        draw(&mut c, &picture, 2.0, 2.0, 4.0, 4.0);
        let at = |x: usize, y: usize| {
            let i = (y * 8 + x) * 4;
            [c.data[i], c.data[i + 1], c.data[i + 2]]
        };
        assert_eq!(at(2, 2), [255, 0, 0], "the top-left quarter is the first pixel");
        assert_eq!(at(5, 5), [255, 255, 255], "and the bottom-right the last");
    }
}
