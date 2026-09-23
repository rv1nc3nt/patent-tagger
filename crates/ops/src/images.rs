//! Parses OPS images-inquiry responses and converts fetched TIFF pages to
//! PNG (SPEC 5.5). XML shape and the real Group 4 TIFF page verified
//! against recorded fixtures in `tests/fixtures/ops/` (see
//! docs/DECISIONS.md).

use crate::error::OpsError;
use roxmltree::Document;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInstance {
    /// `"FullDocument" | "Drawing" | "FirstPageClipping"`.
    pub desc: String,
    /// The path (relative to the REST base) to fetch pages from, e.g.
    /// `published-data/images/EP/1000000/A1/thumbnail` - verified against
    /// the live host that this, despite its name, is the actual link for
    /// full-resolution drawing pages, not a small preview.
    pub link: String,
    pub number_of_pages: u32,
}

pub fn parse_images_inquiry(xml: &str) -> Result<Vec<DocumentInstance>, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    let instances = doc
        .descendants()
        .filter(|n| n.has_tag_name("document-instance"))
        .filter_map(|n| {
            Some(DocumentInstance {
                desc: n.attribute("desc")?.to_string(),
                link: n.attribute("link")?.to_string(),
                number_of_pages: n.attribute("number-of-pages")?.parse().ok()?,
            })
        })
        .collect();
    Ok(instances)
}

pub fn find_drawing(instances: &[DocumentInstance]) -> Option<&DocumentInstance> {
    instances.iter().find(|i| i.desc == "Drawing")
}

pub fn find_first_page_clipping(instances: &[DocumentInstance]) -> Option<&DocumentInstance> {
    instances.iter().find(|i| i.desc == "FirstPageClipping")
}

/// Converts one decoded TIFF page to PNG bytes, returning `(width, height,
/// png_bytes)` (the caller persists width/height alongside the file per
/// SPEC 4.2's `drawings` schema). SPEC 5.5: "convert each page to a 1-bit
/// or greyscale PNG for storage and display, since webviews do not
/// display TIFF"; "apply the rotation stored in the page, if any". Output
/// is 8-bit greyscale regardless of the source bit depth - SPEC explicitly
/// allows either "1-bit or greyscale", and keeping a single output path
/// sidesteps re-packing 1-bit rows after a rotation changes the row
/// byte-alignment.
pub fn tiff_page_to_png(tiff_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), OpsError> {
    let cursor = std::io::Cursor::new(tiff_bytes);
    let mut decoder =
        tiff::decoder::Decoder::new(cursor).map_err(|e| OpsError::Parse(format!("invalid TIFF: {e}")))?;
    let (width, height) =
        decoder.dimensions().map_err(|e| OpsError::Parse(format!("reading TIFF dimensions: {e}")))?;
    let orientation = decoder
        .get_tag_u32(tiff::tags::Tag::Orientation)
        .unwrap_or(1); // 1 = normal, per TIFF/EXIF convention, when the tag is absent.

    let image = decoder.read_image().map_err(|e| OpsError::Parse(format!("decoding TIFF: {e}")))?;
    let grey = to_grey_bytes(image, width, height)?;
    let (width, height, grey) = apply_orientation(width, height, grey, orientation);

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| OpsError::Parse(format!("writing PNG header: {e}")))?;
        writer
            .write_image_data(&grey)
            .map_err(|e| OpsError::Parse(format!("writing PNG data: {e}")))?;
    }
    Ok((width, height, png_bytes))
}

/// Expands whatever `tiff` decoded (packed 1-bit bilevel, or already 8-bit
/// grey) into one byte per pixel, 0=black/255=white.
fn to_grey_bytes(
    image: tiff::decoder::DecodingResult,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, OpsError> {
    match image {
        tiff::decoder::DecodingResult::U8(data) => {
            let pixels = (width as usize) * (height as usize);
            if data.len() == pixels {
                // Already one byte per pixel.
                Ok(data)
            } else {
                // Packed 1-bit-per-pixel, each row padded to a byte
                // boundary (the TIFF bilevel convention) - unpack it.
                let row_bytes = width.div_ceil(8) as usize;
                let mut out = Vec::with_capacity(pixels);
                for row in data.chunks(row_bytes) {
                    for col in 0..width as usize {
                        let byte = row[col / 8];
                        let bit = (byte >> (7 - col % 8)) & 1;
                        out.push(if bit == 1 { 255 } else { 0 });
                    }
                }
                Ok(out)
            }
        }
        other => Err(OpsError::Parse(format!("unsupported TIFF pixel format: {other:?}"))),
    }
}

/// TIFF/EXIF `Orientation` tag values 1-8. Only rotation (not mirroring) is
/// expected in practice for scanned patent pages, but all eight are
/// handled for correctness.
fn apply_orientation(width: u32, height: u32, grey: Vec<u8>, orientation: u32) -> (u32, u32, Vec<u8>) {
    let w = width as usize;
    let h = height as usize;
    let get = |x: usize, y: usize| grey[y * w + x];

    match orientation {
        1 => (width, height, grey),
        2 => (width, height, transform(w, h, |x, y| get(w - 1 - x, y))),
        3 => (width, height, transform(w, h, |x, y| get(w - 1 - x, h - 1 - y))),
        4 => (width, height, transform(w, h, |x, y| get(x, h - 1 - y))),
        5 => (height, width, transform(h, w, |x, y| get(y, x))),
        6 => (height, width, transform(h, w, |x, y| get(y, h - 1 - x))),
        7 => (height, width, transform(h, w, |x, y| get(w - 1 - y, h - 1 - x))),
        8 => (height, width, transform(h, w, |x, y| get(w - 1 - y, x))),
        _ => (width, height, grey),
    }
}

fn transform(out_w: usize, out_h: usize, pixel_at: impl Fn(usize, usize) -> u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(out_w * out_h);
    for y in 0..out_h {
        for x in 0..out_w {
            out.push(pixel_at(x, y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_bytes(name: &str) -> Vec<u8> {
        fs::read(format!("{}/../../tests/fixtures/ops/{name}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    fn fixture_str(name: &str) -> String {
        fs::read_to_string(format!("{}/../../tests/fixtures/ops/{name}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    #[test]
    fn images_inquiry_lists_full_document_drawing_and_clipping() {
        let instances = parse_images_inquiry(&fixture_str("ep1000000_a1_images_inquiry.xml")).unwrap();
        assert_eq!(instances.len(), 3);
        let drawing = find_drawing(&instances).unwrap();
        assert_eq!(drawing.number_of_pages, 6);
        assert_eq!(drawing.link, "published-data/images/EP/1000000/A1/thumbnail");
        let clipping = find_first_page_clipping(&instances).unwrap();
        assert_eq!(clipping.number_of_pages, 1);
    }

    #[test]
    fn a_publication_with_no_drawing_instance_is_normal() {
        let instances =
            parse_images_inquiry(&fixture_str("ep0100001_a1_images_inquiry_no_drawings.xml")).unwrap();
        assert!(find_drawing(&instances).is_none());
    }

    #[test]
    fn decodes_a_real_group4_tiff_page_matching_the_reference_png() {
        let tiff_bytes = fixture_bytes("ep1000000_a1_drawing_page1_group4.tiff");
        let (width, height, png_bytes) = tiff_page_to_png(&tiff_bytes).expect("should decode and convert");
        assert_eq!((width, height), (3508, 2479), "orientation-corrected, landscape");

        let reference_png = fixture_bytes("ep1000000_a1_drawing_page1_reference.png");

        // Compare decoded pixels, not raw PNG bytes (the encoder's own
        // compression choices could differ from the reference file's,
        // which was produced by an independent, non-Rust check during
        // development - see docs/DECISIONS.md) - decode both back and
        // compare pixel-for-pixel.
        let ours = png::Decoder::new(std::io::Cursor::new(&png_bytes)).read_info().unwrap();
        let reference = png::Decoder::new(std::io::Cursor::new(&reference_png)).read_info().unwrap();
        assert_eq!(ours.info().width, reference.info().width);
        assert_eq!(ours.info().height, reference.info().height);

        let mut ours = ours;
        let mut reference = reference;
        let mut our_buf = vec![0u8; ours.output_buffer_size().expect("buffer size")];
        let our_info = ours.next_frame(&mut our_buf).unwrap();
        let mut ref_buf = vec![0u8; reference.output_buffer_size().expect("buffer size")];
        let ref_info = reference.next_frame(&mut ref_buf).unwrap();

        // Our output is 8-bit grey; the reference (from the initial manual
        // verification) is 1-bit grey - compare via black/white classification
        // rather than raw bytes.
        let our_pixels: Vec<bool> = our_buf[..our_info.buffer_size()].iter().map(|&b| b < 128).collect();
        let ref_pixels_packed = &ref_buf[..ref_info.buffer_size()];
        let ref_row_bytes = (reference.info().width as usize).div_ceil(8);
        let ref_pixels: Vec<bool> = (0..reference.info().height as usize)
            .flat_map(|y| {
                (0..reference.info().width as usize).map(move |x| {
                    let byte = ref_pixels_packed[y * ref_row_bytes + x / 8];
                    ((byte >> (7 - x % 8)) & 1) == 0 // 0 = black in 1-bit grey
                })
            })
            .collect();

        assert_eq!(our_pixels, ref_pixels, "decoded pixels should exactly match the independently-produced reference");
    }
}
