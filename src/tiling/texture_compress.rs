use std::io::Cursor;

use image::{ImageFormat, RgbaImage};
use tracing::warn;

use crate::config::{TextureConfig, TextureFormat};
use crate::types::TextureData;

/// Encode an RGBA image according to the given texture configuration.
pub fn compress_texture(image: &RgbaImage, config: &TextureConfig) -> TextureData {
    let (width, height) = image.dimensions();

    match config.format {
        TextureFormat::WebP => encode_webp(image, width, height),
        TextureFormat::Original => encode_png(image, width, height),
        TextureFormat::Ktx2 => encode_ktx2(image, width, height, config.quality),
    }
}

/// Encode an RGBA image to Basis Universal format (UASTC mode for high quality).
///
/// When the `ktx2` feature is enabled, uses the basis-universal crate.
/// Otherwise, falls back to WebP with a warning.
fn encode_ktx2(image: &RgbaImage, width: u32, height: u32, quality: u8) -> TextureData {
    #[cfg(feature = "ktx2")]
    {
        match encode_basis_universal(image, width, height, quality) {
            Ok(data) => data,
            Err(e) => {
                warn!("Basis Universal encoding failed ({e}), falling back to WebP");
                encode_webp(image, width, height)
            }
        }
    }

    #[cfg(not(feature = "ktx2"))]
    {
        let _ = quality;
        warn!("KTX2 support requires the 'ktx2' feature flag, falling back to WebP");
        encode_webp(image, width, height)
    }
}

#[cfg(feature = "ktx2")]
fn encode_basis_universal(
    image: &RgbaImage,
    width: u32,
    height: u32,
    quality: u8,
) -> std::result::Result<TextureData, String> {
    use basis_universal::encoding::{
        encoder_init, ColorSpace, Compressor, CompressorParams,
    };
    use basis_universal::{BasisTextureFormat, UASTC_QUALITY_MAX, UASTC_QUALITY_MIN};

    // Initialize the encoder (thread-safe, idempotent)
    encoder_init();

    let mut params = CompressorParams::new();
    params.set_basis_format(BasisTextureFormat::UASTC4x4);

    // Map quality 0-100 to UASTC quality levels
    let uastc_quality = match quality {
        0..=20 => UASTC_QUALITY_MIN,
        21..=50 => 1,
        51..=75 => 2,
        76..=90 => 3,
        _ => UASTC_QUALITY_MAX,
    };
    params.set_uastc_quality_level(uastc_quality);

    // Enable RDO for better compression ratios
    params.set_rdo_uastc(Some(1.0));
    params.set_generate_mipmaps(false);
    params.set_color_space(ColorSpace::Srgb);

    // Set source image data
    let rgba_bytes = image.as_raw();
    params.source_image_mut(0).init(rgba_bytes, width, height, 4);

    // Compress
    let mut compressor = Compressor::new(4); // Use up to 4 threads
    // SAFETY: params and compressor are valid, encoder_init() was called
    unsafe {
        compressor.init(&params);
        compressor
            .process()
            .map_err(|e| format!("Compressor process failed: {e:?}"))?;
    }

    let basis_data = compressor.basis_file().to_vec();
    if basis_data.is_empty() {
        return Err("Basis Universal produced empty output".into());
    }

    Ok(TextureData {
        data: basis_data,
        mime_type: "image/ktx2".into(),
        width,
        height,
    })
}

fn encode_webp(image: &RgbaImage, width: u32, height: u32) -> TextureData {
    let mut buf = Cursor::new(Vec::new());
    match image.write_to(&mut buf, ImageFormat::WebP) {
        Ok(()) => TextureData {
            data: buf.into_inner(),
            mime_type: "image/webp".into(),
            width,
            height,
        },
        Err(e) => {
            warn!("WebP encoding failed ({e}), falling back to PNG");
            encode_png(image, width, height)
        }
    }
}

fn encode_png(image: &RgbaImage, width: u32, height: u32) -> TextureData {
    let mut buf = Cursor::new(Vec::new());
    image
        .write_to(&mut buf, ImageFormat::Png)
        .expect("PNG encoding failed");
    TextureData {
        data: buf.into_inner(),
        mime_type: "image/png".into(),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkerboard(size: u32) -> RgbaImage {
        RgbaImage::from_fn(size, size, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
    }

    #[test]
    fn png_roundtrip() {
        let img = checkerboard(4);
        let config = TextureConfig {
            format: TextureFormat::Original,
            ..Default::default()
        };
        let td = compress_texture(&img, &config);
        assert_eq!(td.mime_type, "image/png");
        assert_eq!(td.width, 4);
        assert_eq!(td.height, 4);
        assert!(!td.data.is_empty());

        // Roundtrip decode
        let decoded = image::load_from_memory(&td.data).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (4, 4));
        assert_eq!(decoded.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn webp_roundtrip() {
        let img = checkerboard(8);
        let config = TextureConfig {
            format: TextureFormat::WebP,
            ..Default::default()
        };
        let td = compress_texture(&img, &config);
        assert_eq!(td.mime_type, "image/webp");
        assert_eq!(td.width, 8);
        assert_eq!(td.height, 8);
        assert!(!td.data.is_empty());

        // Should be decodable
        let decoded = image::load_from_memory(&td.data).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (8, 8));
    }

    #[test]
    fn format_config_respected() {
        let img = checkerboard(2);

        let png_config = TextureConfig {
            format: TextureFormat::Original,
            ..Default::default()
        };
        let webp_config = TextureConfig {
            format: TextureFormat::WebP,
            ..Default::default()
        };

        let png_td = compress_texture(&img, &png_config);
        let webp_td = compress_texture(&img, &webp_config);

        assert_eq!(png_td.mime_type, "image/png");
        assert_eq!(webp_td.mime_type, "image/webp");
    }

    #[test]
    fn ktx2_encoding() {
        let img = checkerboard(4);
        let config = TextureConfig {
            format: TextureFormat::Ktx2,
            ..Default::default()
        };
        let td = compress_texture(&img, &config);
        // With ktx2 feature: produces image/ktx2
        // Without ktx2 feature: falls back to image/webp
        assert!(
            td.mime_type == "image/ktx2" || td.mime_type == "image/webp",
            "KTX2 should produce ktx2 or fallback to webp, got {}",
            td.mime_type
        );
        assert!(!td.data.is_empty());
    }
}
