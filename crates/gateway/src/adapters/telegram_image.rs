use anyhow::Context;
use base64::Engine;
use ccode_config::schema::{ImageConfig, ImageStrategy};
use ccode_domain::llm::{ImageData, ImageMediaType, ImageSource};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    description: Option<String>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct TelegramFile {
    file_path: Option<String>,
}

pub async fn download_and_process_image(
    http_client: &reqwest::Client,
    bot_token: &str,
    file_id: &str,
    preferred_media_type: Option<ImageMediaType>,
    image_cfg: &ImageConfig,
) -> anyhow::Result<ImageSource> {
    let file_path = get_file_path(http_client, bot_token, file_id).await?;

    let bytes = http_client
        .get(format!(
            "https://api.telegram.org/file/bot{bot_token}/{file_path}"
        ))
        .send()
        .await
        .with_context(|| format!("telegram file download request failed for {file_path}"))?
        .error_for_status()
        .with_context(|| format!("telegram file download failed for {file_path}"))?
        .bytes()
        .await
        .with_context(|| format!("telegram file download bytes failed for {file_path}"))?;

    let strategy = image_cfg.strategy.clone().unwrap_or(ImageStrategy::Resize);
    let max_dimension = image_cfg.max_dimension.unwrap_or(2048);
    let processed = ccode_image_process::process(bytes.as_ref(), strategy.clone(), max_dimension)
        .context("telegram image process failed")?;

    let media_type = match strategy {
        ImageStrategy::None => preferred_media_type
            .or_else(|| media_type_from_path(file_path.as_str()))
            .unwrap_or(ImageMediaType::Jpeg),
        ImageStrategy::Resize | ImageStrategy::Quantize => ImageMediaType::Png,
    };

    Ok(ImageSource {
        media_type,
        data: ImageData::Base64(base64::engine::general_purpose::STANDARD.encode(processed.data)),
    })
}

async fn get_file_path(
    http_client: &reqwest::Client,
    bot_token: &str,
    file_id: &str,
) -> anyhow::Result<String> {
    let response = http_client
        .get(format!("https://api.telegram.org/bot{bot_token}/getFile"))
        .query(&[("file_id", file_id)])
        .send()
        .await
        .with_context(|| format!("telegram getFile request failed for file_id={file_id}"))?
        .error_for_status()
        .with_context(|| format!("telegram getFile failed for file_id={file_id}"))?
        .json::<TelegramApiResponse<TelegramFile>>()
        .await
        .context("telegram getFile decode failed")?;

    if !response.ok {
        let desc = response
            .description
            .unwrap_or_else(|| "no description".to_string());
        anyhow::bail!("telegram getFile API error: {desc}");
    }

    response
        .result
        .and_then(|f| f.file_path)
        .ok_or_else(|| anyhow::anyhow!("telegram getFile response missing result.file_path"))
}

pub fn media_type_from_mime_or_name(
    mime_type: Option<&str>,
    file_name: Option<&str>,
) -> Option<ImageMediaType> {
    if let Some(mime) = mime_type.map(str::trim).filter(|s| !s.is_empty()) {
        match mime.to_ascii_lowercase().as_str() {
            "image/jpeg" | "image/jpg" => return Some(ImageMediaType::Jpeg),
            "image/png" => return Some(ImageMediaType::Png),
            "image/gif" => return Some(ImageMediaType::Gif),
            "image/webp" => return Some(ImageMediaType::Webp),
            _ => {}
        }
    }

    file_name.and_then(media_type_from_path)
}

pub fn media_type_from_path(path: &str) -> Option<ImageMediaType> {
    let ext = path.rsplit('.').next()?.trim();
    if ext.is_empty() {
        return None;
    }
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some(ImageMediaType::Jpeg),
        "png" => Some(ImageMediaType::Png),
        "gif" => Some(ImageMediaType::Gif),
        "webp" => Some(ImageMediaType::Webp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ccode_domain::llm::ImageMediaType;

    use super::media_type_from_mime_or_name;

    #[test]
    fn media_type_from_mime_wins_over_filename() {
        let media = media_type_from_mime_or_name(Some("image/png"), Some("photo.jpg"));
        assert_eq!(media, Some(ImageMediaType::Png));
    }

    #[test]
    fn media_type_falls_back_to_filename() {
        let media = media_type_from_mime_or_name(None, Some("photo.jpeg"));
        assert_eq!(media, Some(ImageMediaType::Jpeg));
    }

    #[test]
    fn unsupported_document_type_returns_none() {
        let media = media_type_from_mime_or_name(Some("application/pdf"), Some("report.pdf"));
        assert_eq!(media, None);
    }
}
