use arboard::Clipboard;
use base64::Engine;
use ccode_config::schema::{Config, ImageStrategy};
use ccode_domain::llm::{ImageData, ImageMediaType, ImageSource};
use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};

const IMAGE_PLACEHOLDER_PREFIX: &str = "@image:";

#[derive(Debug, thiserror::Error)]
pub enum ImageInputError {
    #[error("unsupported image extension for placeholder path: {path}")]
    UnsupportedExtension { path: String },
    #[error("failed to read image file: {path} ({source})")]
    ReadFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("clipboard access failed: {0}")]
    Clipboard(#[source] arboard::Error),
    #[error("clipboard does not currently contain image data")]
    ClipboardNotImage,
    #[error("failed to read copied file from clipboard: {path} ({source})")]
    ClipboardReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode RGBA clipboard image as PNG: {0}")]
    EncodePng(#[source] image::ImageError),
    #[error("failed to process image: {0}")]
    Process(#[source] ccode_image_process::ImageProcessError),
    #[error("failed to write clipboard image temp file: {0}")]
    Tempfile(#[source] std::io::Error),
}

pub fn load_images_from_placeholders(input: &str) -> Result<Vec<ImageSource>, ImageInputError> {
    collect_image_placeholder_paths(input)
        .into_iter()
        .map(|path| {
            let media_type = media_type_from_path(Path::new(path.as_str()))?;
            let bytes =
                std::fs::read(path.as_str()).map_err(|source| ImageInputError::ReadFailed {
                    path: path.clone(),
                    source,
                })?;
            Ok(ImageSource {
                media_type,
                data: ImageData::Base64(base64::engine::general_purpose::STANDARD.encode(bytes)),
            })
        })
        .collect()
}

pub fn paste_image_from_clipboard_to_temp_file() -> Result<PathBuf, ImageInputError> {
    let mut cb = Clipboard::new().map_err(ImageInputError::Clipboard)?;
    let source_bytes = read_clipboard_image_bytes(&mut cb)?;
    let (strategy, max_dimension) = image_processing_config();
    let processed = ccode_image_process::process(source_bytes.as_slice(), strategy, max_dimension)
        .map_err(ImageInputError::Process)?;
    let png_bytes = ensure_png(processed.data)?;

    let mut tmp = tempfile::Builder::new()
        .prefix("ccode-clipboard-")
        .suffix(".png")
        .tempfile_in(std::env::temp_dir())
        .map_err(ImageInputError::Tempfile)?;
    tmp.write_all(png_bytes.as_slice())
        .map_err(ImageInputError::Tempfile)?;
    let (_file, path) = tmp.keep().map_err(|e| ImageInputError::Tempfile(e.error))?;
    Ok(path)
}

fn collect_image_placeholder_paths(input: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut cursor = input;
    while let Some(idx) = cursor.find(IMAGE_PLACEHOLDER_PREFIX) {
        let start = idx + IMAGE_PLACEHOLDER_PREFIX.len();
        let after = &cursor[start..];
        let end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let candidate = after[..end].trim();
        if !candidate.is_empty() {
            paths.push(candidate.to_string());
        }
        cursor = &after[end..];
    }
    paths
}

fn media_type_from_path(path: &Path) -> Result<ImageMediaType, ImageInputError> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let media = match ext.as_str() {
        "png" => ImageMediaType::Png,
        "jpg" | "jpeg" => ImageMediaType::Jpeg,
        "gif" => ImageMediaType::Gif,
        "webp" => ImageMediaType::Webp,
        _ => {
            return Err(ImageInputError::UnsupportedExtension {
                path: path.display().to_string(),
            });
        }
    };
    Ok(media)
}

fn read_clipboard_image_bytes(cb: &mut Clipboard) -> Result<Vec<u8>, ImageInputError> {
    if let Some(file_bytes) = try_read_image_from_file_list(cb)? {
        return Ok(file_bytes);
    }
    read_image_pixels_from_clipboard(cb)
}

fn try_read_image_from_file_list(cb: &mut Clipboard) -> Result<Option<Vec<u8>>, ImageInputError> {
    let data = cb.get();
    let Ok(file_list) = data.file_list() else {
        return Ok(None);
    };

    for path in file_list {
        if !is_supported_image_path(path.as_path()) {
            continue;
        }
        let bytes =
            std::fs::read(path.as_path()).map_err(|source| ImageInputError::ClipboardReadFile {
                path: path.display().to_string(),
                source,
            })?;
        return Ok(Some(bytes));
    }
    Ok(None)
}

fn read_image_pixels_from_clipboard(cb: &mut Clipboard) -> Result<Vec<u8>, ImageInputError> {
    let image = cb.get_image().map_err(|err| match err {
        arboard::Error::ContentNotAvailable => ImageInputError::ClipboardNotImage,
        other => ImageInputError::Clipboard(other),
    })?;

    encode_rgba_to_png(image.width as u32, image.height as u32, image.bytes)
}

fn encode_rgba_to_png(
    width: u32,
    height: u32,
    bytes: Cow<'_, [u8]>,
) -> Result<Vec<u8>, ImageInputError> {
    let rgba = image::RgbaImage::from_raw(width, height, bytes.into_owned()).ok_or_else(|| {
        ImageInputError::EncodePng(image::ImageError::Limits(
            image::error::LimitError::from_kind(image::error::LimitErrorKind::DimensionError),
        ))
    })?;

    let dyn_img = image::DynamicImage::ImageRgba8(rgba);
    let mut png_bytes = Vec::new();
    dyn_img
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(ImageInputError::EncodePng)?;
    Ok(png_bytes)
}

fn image_processing_config() -> (ImageStrategy, u32) {
    let cfg = ccode_config::load().unwrap_or_else(|_| Config::default());
    let strategy = cfg.image.strategy.unwrap_or(ImageStrategy::Resize);
    let max_dimension = cfg.image.max_dimension.unwrap_or(2048);
    (strategy, max_dimension)
}

fn ensure_png(bytes: Vec<u8>) -> Result<Vec<u8>, ImageInputError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(bytes);
    }

    let img = image::load_from_memory(bytes.as_slice()).map_err(ImageInputError::EncodePng)?;
    let mut png_bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .map_err(ImageInputError::EncodePng)?;
    Ok(png_bytes)
}

fn is_supported_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_and_loads_all_placeholder_images() {
        let tmp = TempDir::new().expect("tempdir");
        let png_path = tmp.path().join("a.png");
        let gif_path = tmp.path().join("b.gif");
        std::fs::write(&png_path, b"png-bytes").expect("write png");
        std::fs::write(&gif_path, b"gif-bytes").expect("write gif");

        let input = format!(
            "hello @image:{} world @image:{}",
            png_path.display(),
            gif_path.display()
        );
        let images = load_images_from_placeholders(&input).expect("parse images");
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].media_type, ImageMediaType::Png);
        assert_eq!(images[1].media_type, ImageMediaType::Gif);
    }
}
