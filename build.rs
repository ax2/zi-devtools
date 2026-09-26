use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let icon_path = out_dir.join("zi-devtools.ico");
    fs::write(&icon_path, make_icon()).expect("write generated icon");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon_path.to_str().expect("icon path"))
        .set("ProductName", "Zi DevTools")
        .set("FileDescription", "Zi Windows Developer Toolbox")
        .set("LegalCopyright", "ZiCode");
    resource.compile().expect("compile Windows resources");
}

fn make_icon() -> Vec<u8> {
    const SIZE: usize = 32;
    let image_size = 40 + SIZE * SIZE * 4 + SIZE * 4;
    let mut out = Vec::with_capacity(22 + image_size);

    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&[SIZE as u8, SIZE as u8, 0, 0]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&(image_size as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());

    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(SIZE as i32).to_le_bytes());
    out.extend_from_slice(&((SIZE * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((SIZE * SIZE * 4) as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..SIZE).rev() {
        for x in 0..SIZE {
            let (r, g, b, a) = logo_pixel(x, y, SIZE);
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out.resize(22 + image_size, 0);
    out
}

fn logo_pixel(x: usize, y: usize, size: usize) -> (u8, u8, u8, u8) {
    let dx = x as f32 - (size as f32 - 1.0) / 2.0;
    let dy = y as f32 - (size as f32 - 1.0) / 2.0;
    let radius = (dx * dx + dy * dy).sqrt();
    if radius > 15.0 {
        return (0, 0, 0, 0);
    }
    if radius > 12.6 {
        return (92, 111, 255, 255);
    }
    let z_stroke = (9..=22).contains(&x)
        && ((9..=11).contains(&y)
            || (20..=22).contains(&y)
            || ((11..=20).contains(&y) && (30..=34).contains(&(x + y))));
    if z_stroke {
        (237, 242, 255, 255)
    } else {
        (26, 35, 63, 255)
    }
}
