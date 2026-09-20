//! png in and out : saving window grabs, and decoding the two logos I
//! drew that get baked into the binary.
//!
//! note that anything reaching for the `png` **crate** from inside the gui
//! has to say `::png`, because this module shadows it.

use std::fs::File;
use std::io::BufWriter;

use egui::ColorImage;

/// dump a frame next to wherever you ran the binary, named by the clock.
///
/// returns the filename so the ui can say what it did
pub fn save(image: &ColorImage) -> Result<String, Box<dyn std::error::Error>> {
    let name = format!("xdso-{}.png", stamp());
    let file = BufWriter::new(File::create(&name)?);

    let [w, h] = [image.width() as u32, image.height() as u32];
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);

    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    enc.write_header()?.write_image_data(&bytes)?;
    Ok(name)
}

/// seconds since the epoch. yes a real timestamp would be nicer but that means
/// pulling in chrono for one line and i cant be bothered
fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// my icon, 64x64, used for the window icon
pub const ICON_BYTES: &[u8] = include_bytes!("../../../packaging/xdso.png");
/// my banner, 96x16 pixel art, shown along the top of the window
pub const LOGO_BYTES: &[u8] = include_bytes!("../../../packaging/xdso-logo.png");

/// decode one of the baked in pngs to plain rgba.
///
/// returns None rather than panicking if the file ever stops being 8 bit
/// rgba, because a missing picture is not a reason to refuse to start. theres
/// a test so it cant go unnoticed
pub fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    // `::png` : our own module name is in the way otherwise
    let mut reader = ::png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != ::png::ColorType::Rgba || info.bit_depth != ::png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((buf, info.width, info.height))
}

/// the banner, ready to hand to egui
pub fn logo_image() -> Option<egui::ColorImage> {
    let (rgba, w, h) = decode_rgba(LOGO_BYTES)?;
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_pictures_decode() {
        let (rgba, w, h) = decode_rgba(ICON_BYTES).expect("icon");
        assert_eq!((w, h), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);

        let (rgba, w, h) = decode_rgba(LOGO_BYTES).expect("logo");
        assert_eq!((w, h), (96, 16));
        assert_eq!(rgba.len(), 96 * 16 * 4);
    }
}
