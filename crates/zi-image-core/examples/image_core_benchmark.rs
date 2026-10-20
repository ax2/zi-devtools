use image::ImageFormat;
use std::{env, fs, io::Cursor, process};

fn format(name: &str) -> Result<ImageFormat, String> {
    match name {
        "png" => Ok(ImageFormat::Png),
        "jpeg" => Ok(ImageFormat::Jpeg),
        "webp" => Ok(ImageFormat::WebP),
        _ => Err("unsupported format".into()),
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 5 {
        return Err("usage: image_core_benchmark INPUT OUTPUT MAX_WIDTH FORMAT QUALITY".into());
    }
    let max_width = args[2]
        .parse::<u32>()
        .map_err(|_| "invalid maximum width")?;
    let quality = args[4].parse::<u8>().map_err(|_| "invalid quality")?;
    let source_bytes = fs::read(&args[0]).map_err(|_| "cannot read input fixture")?;
    let source = zi_image_core::decode(&source_bytes).map_err(|_| "shared decode failed")?;
    let resized =
        zi_image_core::resize_to_width(&source, max_width).map_err(|_| "shared resize failed")?;
    let mut output = Cursor::new(Vec::new());
    zi_image_core::encode(&resized, format(&args[3])?, quality, &mut output)
        .map_err(|_| "shared encode failed")?;
    let bytes = output.into_inner();
    fs::write(&args[1], &bytes).map_err(|_| "cannot write benchmark output")?;
    println!(
        "{{\"sourceWidth\":{},\"sourceHeight\":{},\"width\":{},\"height\":{},\"byteLength\":{}}}",
        source.width(),
        source.height(),
        resized.width(),
        resized.height(),
        bytes.len()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}
