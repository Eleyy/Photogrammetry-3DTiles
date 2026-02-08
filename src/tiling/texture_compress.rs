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
        TextureFormat::Ktx2 => {
            warn!("KTX2 not yet supported, falling back to WebP");
            encode_webp(image, width, height)
        }
    }
}

fn encode_webp(image: &RgbaImage, width: u32, height: u32) -> TextureData {
    let mut buf = Cursor::new(Vec::new());
    image
        .write_to(&mut buf, ImageFormat::WebP)
        .expect("WebP encoding failed");
    TextureData {
        data: buf.into_inner(),
        mime_type: "image/webp".into(),
        width,
        height,
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
    fn ktx2_falls_back_to_webp() {
        let img = checkerboard(2);
        let config = TextureConfig {
            format: TextureFormat::Ktx2,
            ..Default::default()
        };
        let td = compress_texture(&img, &config);
        assert_eq!(td.mime_type, "image/webp");
    }
}
