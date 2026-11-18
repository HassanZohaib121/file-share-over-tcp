use axum::{
    extract::Query,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use qrcode::{QrCode, render::svg};
use serde::Deserialize;
use std::io::Cursor;

const QUIET_ZONE: usize = 4;
const PIXELS_PER_MODULE: usize = 4;

#[derive(Deserialize)]
pub struct QrRequest {
    url: String,
}

pub(crate) fn image_for_url(url: &str) -> Result<slint::Image, qrcode::types::QrError> {
    let code = QrCode::new(url.as_bytes())?;
    let modules = code.width();
    let side = ((modules + QUIET_ZONE * 2) * PIXELS_PER_MODULE) as u32;
    let colors = code.to_colors();
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(side, side);
    let pixels = buffer.make_mut_slice();
    let white = slint::Rgba8Pixel::new(255, 255, 255, 255);
    let black = slint::Rgba8Pixel::new(0, 0, 0, 255);

    for y in 0..side as usize {
        for x in 0..side as usize {
            let module_x = x / PIXELS_PER_MODULE;
            let module_y = y / PIXELS_PER_MODULE;
            let is_dark = module_x >= QUIET_ZONE
                && module_y >= QUIET_ZONE
                && module_x < QUIET_ZONE + modules
                && module_y < QUIET_ZONE + modules
                && colors[(module_y - QUIET_ZONE) * modules + module_x - QUIET_ZONE]
                    == qrcode::types::Color::Dark;
            pixels[y * side as usize + x] = if is_dark { black } else { white };
        }
    }

    Ok(slint::Image::from_rgba8(buffer))
}

pub(crate) fn png_for_url(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let code = QrCode::new(url.as_bytes())?;
    let image = code
        .render::<image::Luma<u8>>()
        .quiet_zone(true)
        .min_dimensions(400, 400)
        .build();
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image).write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

pub async fn generate(Query(request): Query<QrRequest>) -> Response {
    match QrCode::new(request.url.as_bytes()) {
        Ok(code) => (
            [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
            code.render::<svg::Color>().build(),
        )
            .into_response(),
        Err(error) => {
            eprintln!("Could not generate QR code: {error}");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{image_for_url, png_for_url};

    #[test]
    fn generates_visible_image_with_quiet_zone() {
        let image = image_for_url("http://192.168.1.10:3000/download/file").unwrap();
        let pixels = image.to_rgba8().unwrap();

        assert!(pixels.width() > 0);
        assert!(pixels.height() > 0);
        assert!(pixels.as_slice().iter().any(|pixel| pixel.r == 0));
        assert!(pixels.as_slice().iter().any(|pixel| pixel.r == 255));
    }

    #[test]
    fn generates_downloadable_png() {
        let png = png_for_url("http://192.168.1.10:3000/download/file").unwrap();

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(image::load_from_memory(&png).unwrap().width() >= 400);
    }
}
