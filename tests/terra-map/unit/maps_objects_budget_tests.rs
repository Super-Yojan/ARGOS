use super::*;
#[test]
fn compressed_texture_is_rejected_before_large_image_allocation() {
    use base64::Engine;
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(64, 64, image::Rgba([40, 80, 120, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let doc = serde_json::json!({
        "asset": {"version":"2.0"},
        "images": [{"uri": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.into_inner()))}],
        "scenes": [{"nodes":[]}], "scene":0
    });
    let error = content::decode_with_limit(
        &serde_json::to_vec(&doc).unwrap(),
        &Url::parse("file:///test.gltf").unwrap(),
        |_| unreachable!(),
        1024,
    )
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("image allocation limit"),
        "{error}"
    );
}
use std::io::Cursor;
