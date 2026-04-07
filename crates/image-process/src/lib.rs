use ccode_config::schema::ImageStrategy;
use image::DynamicImage;
use std::io::Cursor;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ProcessedImage {
    pub data: Vec<u8>,
    pub media_type: &'static str,
    pub original_size: usize,
    pub processed_size: usize,
}

#[derive(Debug, Error)]
pub enum ImageProcessError {
    #[error("image decode/encode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("png decode failed: {0}")]
    PngDecode(#[from] png::DecodingError),
    #[error("png encode failed: {0}")]
    PngEncode(#[from] png::EncodingError),
    #[error("image quantization failed: {0}")]
    Quantize(#[from] imagequant::Error),
    #[error("unsupported PNG format for quantization: color={color_type:?}, depth={bit_depth:?}")]
    UnsupportedFormat {
        color_type: png::ColorType,
        bit_depth: png::BitDepth,
    },
}

pub fn process(
    bytes: &[u8],
    strategy: ImageStrategy,
    max_dimension: u32,
) -> Result<ProcessedImage, ImageProcessError> {
    let original_size = bytes.len();
    let data = match strategy {
        ImageStrategy::Resize => resize_png(bytes, max_dimension)?,
        ImageStrategy::Quantize => quantize_png(bytes, max_dimension)?,
        ImageStrategy::None => bytes.to_vec(),
    };
    let processed_size = data.len();

    Ok(ProcessedImage {
        data,
        media_type: "image/png",
        original_size,
        processed_size,
    })
}

fn resize_png(bytes: &[u8], max_dimension: u32) -> Result<Vec<u8>, ImageProcessError> {
    let mut img = image::load_from_memory(bytes)?;
    if img.width() > max_dimension || img.height() > max_dimension {
        img = img.resize(
            max_dimension,
            max_dimension,
            image::imageops::FilterType::Triangle,
        );
    }

    encode_dynamic_as_png(&img)
}

fn encode_dynamic_as_png(img: &DynamicImage) -> Result<Vec<u8>, ImageProcessError> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

fn quantize_png(bytes: &[u8], max_dimension: u32) -> Result<Vec<u8>, ImageProcessError> {
    let resized_png = resize_png(bytes, max_dimension)?;

    let decoder = png::Decoder::new(Cursor::new(&resized_png));
    let mut reader = decoder.read_info()?;
    let mut raw_buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw_buf)?;
    let pixels = &raw_buf[..info.buffer_size()];

    let rgba_pixels = normalize_to_rgba(pixels, info.color_type, info.bit_depth)?;

    let width = info.width as usize;
    let height = info.height as usize;

    let mut liq = imagequant::new();
    liq.set_quality(0, 80)?;
    let mut image = liq.new_image(&rgba_pixels[..], width, height, 0.0)?;
    let mut result = liq.quantize(&mut image)?;
    result.set_dithering_level(1.0)?;
    let (palette, pixels_out) = result.remapped(&mut image)?;

    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, info.width, info.height);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);

        let mut plte = Vec::with_capacity(palette.len() * 3);
        let mut trns = Vec::with_capacity(palette.len());
        let mut has_transparency = false;
        for color in &palette {
            plte.extend_from_slice(&[color.r, color.g, color.b]);
            trns.push(color.a);
            has_transparency |= color.a != 255;
        }

        if has_transparency {
            encoder.set_trns(trns);
        }
        encoder.set_palette(plte);

        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels_out)?;
    }

    Ok(encoded)
}

fn normalize_to_rgba(
    buf: &[u8],
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
) -> Result<Vec<imagequant::RGBA>, ImageProcessError> {
    let rgba = match (color_type, bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf
            .chunks_exact(4)
            .map(|c| imagequant::RGBA::new(c[0], c[1], c[2], c[3]))
            .collect(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf
            .chunks_exact(3)
            .map(|c| imagequant::RGBA::new(c[0], c[1], c[2], 255))
            .collect(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => buf
            .iter()
            .copied()
            .map(|v| imagequant::RGBA::new(v, v, v, 255))
            .collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => buf
            .chunks_exact(2)
            .map(|c| imagequant::RGBA::new(c[0], c[0], c[0], c[1]))
            .collect(),
        _ => {
            return Err(ImageProcessError::UnsupportedFormat {
                color_type,
                bit_depth,
            });
        }
    };
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn make_solid_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let img = ImageBuffer::from_pixel(width, height, Rgba(color));
        let dynimg = image::DynamicImage::ImageRgba8(img);
        let mut bytes = Vec::new();
        dynimg
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("should encode png");
        bytes
    }

    fn decode_png_size(bytes: &[u8]) -> (u32, u32) {
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("decode png info");
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).expect("decode png frame");
        (info.width, info.height)
    }

    #[test]
    fn process_supports_all_three_strategies_for_4x4_png() {
        let input = make_solid_png(4, 4, [120, 30, 200, 255]);

        for strategy in [
            ImageStrategy::Resize,
            ImageStrategy::Quantize,
            ImageStrategy::None,
        ] {
            let out = process(&input, strategy, 4).expect("process should succeed");
            let (w, h) = decode_png_size(&out.data);
            assert_eq!((w, h), (4, 4));
            assert_eq!(out.media_type, "image/png");
        }
    }

    #[test]
    fn resize_and_quantize_are_bounded_by_max_dimension() {
        let input = make_solid_png(32, 16, [0, 180, 20, 255]);

        for strategy in [ImageStrategy::Resize, ImageStrategy::Quantize] {
            let out = process(&input, strategy, 8).expect("process should succeed");
            let (w, h) = decode_png_size(&out.data);
            assert!(w <= 8);
            assert!(h <= 8);
        }
    }

    #[test]
    fn none_strategy_keeps_original_size() {
        let input = make_solid_png(4, 4, [10, 20, 30, 255]);
        let out = process(&input, ImageStrategy::None, 4).expect("process should succeed");
        assert_eq!(out.original_size, input.len());
        assert_eq!(out.processed_size, input.len());
        assert_eq!(out.data, input);
    }
}
