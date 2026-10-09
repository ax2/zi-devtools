use super::*;
#[allow(dead_code)]
#[path = "legacy.rs"]
mod legacy;

#[test]
fn complete_ascii_names_and_query_precedence_match_original() {
    let queries = [
        "", "7", "07", "0x41", "0X7F", "0xGG", "256", "A", "a", "LF", "换行", "SPACE", "中",
    ];
    for code in 0..=255 {
        assert_eq!(ascii_name(code), legacy::ascii_name(code));
        for query in queries {
            assert_eq!(
                ascii_matches(code, &ascii_name(code), query),
                legacy::ascii_matches(code, &legacy::ascii_name(code), query)
            );
        }
    }
    assert!(ascii_matches(7, &ascii_name(7), "7"));
    assert!(!ascii_matches(b'7', "7", "7"));
    assert!(ascii_matches(32, &ascii_name(32), "SPACE"));
}

#[test]
fn complete_symbol_data_and_category_query_intersection_are_preserved() {
    assert_eq!(SYMBOLS.len(), 37);
    for (current, old) in SYMBOLS.iter().zip(legacy::SYMBOLS) {
        assert_eq!(
            (current.category, current.name, current.value),
            (old.category, old.name, old.value)
        );
    }
    for category in CATEGORIES {
        for query in ["", "  ", "表情", "笑", "(ง", "•", "不存在", "箭头"] {
            let normalized = query.trim().to_lowercase();
            let expected: Vec<_> = legacy::SYMBOLS
                .iter()
                .filter(|s| {
                    (category == "全部" || s.category == category)
                        && (normalized.is_empty()
                            || format!("{} {} {}", s.name, s.value, s.category)
                                .to_lowercase()
                                .contains(&normalized))
                })
                .map(|s| s.value)
                .collect();
            assert_eq!(
                search_symbols(query, category)
                    .iter()
                    .map(|s| s.value)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    assert!(search_symbols("笑", "数学与单位").is_empty());
    assert_eq!(search_symbols("加油", "颜文字")[0].value, "(ง •̀_•́)ง");
}

#[test]
fn every_glyph_and_banner_boundary_matches_original_bytes() {
    for ch in (0u8..=127).map(char::from).chain(['中', '😀', '\u{301}']) {
        for ink in ['#', '@', '!'] {
            assert_eq!(
                banner(&ch.to_string(), ink),
                legacy::banner(&ch.to_string(), ink)
            );
        }
    }
    for text in [
        String::new(),
        "aZ09 -.!?中".into(),
        "A".repeat(40),
        "A".repeat(41),
        "中".repeat(41),
    ] {
        assert_eq!(banner(&text, '#'), legacy::banner(&text, '#'));
        let (output, _) = banner(&text, '#');
        assert_eq!(output.lines().count(), 7);
        assert!(
            output
                .lines()
                .all(|line| line.len() == text.chars().count().min(40) * 6)
        );
        assert!(output.ends_with('\n'));
    }
    assert_eq!(banner("中😀", '#').1, 2);
}

fn sample() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 9, |x, y| {
        image::Rgba([
            (x * 17) as u8,
            (y * 31) as u8,
            ((x + y) * 9) as u8,
            if x < 4 {
                0
            } else if x < 8 {
                127
            } else {
                255
            },
        ])
    })
}

#[test]
fn image_scaling_alpha_luminance_and_geometry_match_original() {
    for source in [
        sample(),
        image::RgbaImage::from_pixel(1, 500, image::Rgba([0, 0, 0, 255])),
        image::RgbaImage::from_pixel(500, 1, image::Rgba([255, 255, 255, 255])),
    ] {
        for width in [8, 16, 64, 120] {
            let output = rgba_art(&source, width).unwrap();
            assert_eq!(output, legacy::rgba_art(&source, width).unwrap());
            assert!((1..=120).contains(&output.lines().count()));
            assert!(output.lines().all(|line| line.len() == width as usize));
        }
    }
    assert!(rgba_art(&sample(), 7).is_err());
    assert!(rgba_art(&sample(), 121).is_err());
    assert!(rgba_art(&image::RgbaImage::new(0, 0), 8).is_err());
    assert_eq!(
        rgba_art(
            &image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0])),
            8
        )
        .unwrap(),
        "        \n".repeat(4)
    );
    assert_eq!(
        rgba_art(
            &image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255])),
            8
        )
        .unwrap(),
        "@@@@@@@@\n".repeat(4)
    );
}

#[test]
fn png_jpeg_webp_byte_input_and_resource_boundaries() {
    use std::io::Cursor;
    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
        image::ImageFormat::WebP,
    ] {
        let source = if format == image::ImageFormat::Jpeg {
            image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(sample()).to_rgb8())
        } else {
            image::DynamicImage::ImageRgba8(sample())
        };
        let mut buffer = Cursor::new(Vec::new());
        source.write_to(&mut buffer, format).unwrap();
        let bytes = buffer.into_inner();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        for width in [8, 120] {
            assert_eq!(
                image_art_bytes(&bytes, width).unwrap(),
                legacy::rgba_art(&decoded, width).unwrap()
            );
        }
        if format == image::ImageFormat::Png {
            let mut oversized_pixels = bytes.clone();
            oversized_pixels[16..20].copy_from_slice(&5001u32.to_be_bytes());
            oversized_pixels[20..24].copy_from_slice(&4000u32.to_be_bytes());
            assert!(image_art_bytes(&oversized_pixels, 8).is_err());
        }
    }
    assert!(image_art_bytes(&[], 8).is_err());
    assert!(image_art_bytes(b"GIF89a", 8).is_err());
    assert!(
        image_art_bytes(&vec![0; 10 * 1024 * 1024 + 1], 8)
            .unwrap_err()
            .to_string()
            .contains("10 MiB")
    );
}
