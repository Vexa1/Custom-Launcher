//! Dev helper for looking at a rendered screenshot up close.
//!
//! ```text
//! cargo run --manifest-path vm-tests/Cargo.toml --example crop -- \
//!     vm-tests/artifacts/game-1280x800.png out.png 530 200 160 160 4
//! ```
//!
//! Arguments: input, output, x, y, width, height, scale.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 8 {
        eprintln!("usage: crop <in> <out> <x> <y> <w> <h> <scale>");
        std::process::exit(2);
    }
    let num = |i: usize| -> u32 { args[i].parse().expect("a pixel count") };
    let image = image::open(&args[1]).expect("open the screenshot");
    let (x, y, w, h, scale) = (num(3), num(4), num(5), num(6), num(7).max(1));
    let crop = image.crop_imm(x, y, w, h);
    let enlarged = crop.resize_exact(w * scale, h * scale, image::imageops::FilterType::Nearest);
    enlarged.save(&args[2]).expect("write the crop");
    println!(
        "{}: {}x{} from ({x},{y}) at {scale}x",
        args[2],
        w * scale,
        h * scale
    );
}
