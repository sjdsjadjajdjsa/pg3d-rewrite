//! bc6h-decode IN OUT WIDTH HEIGHT [signed]
//! Decodes raw BC6H blocks (as stored in a Unity Texture2D) to float32 RGB, row 0 first
//! as stored (Unity: bottom row first). Used by scripts/export_map.py for HDR lightmaps,
//! because the Python decoders clamp BC6H to 8 bits.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: bc6h-decode IN OUT WIDTH HEIGHT [signed]");
        std::process::exit(2);
    }
    let data = std::fs::read(&a[1]).expect("read input");
    let (w, h): (usize, usize) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let signed = a.get(5).is_some_and(|s| s == "signed");
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let pw = bw * 4;
    let mut out = vec![0f32; pw * bh * 4 * 3];
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let Some(block) = data.get(off..off + 16) else { continue };
            let dst = &mut out[(by * 4 * pw + bx * 4) * 3..];
            bcdec_rs::bc6h_float(block, dst, pw * 3, signed);
        }
    }
    let mut bytes = Vec::with_capacity(w * h * 12);
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                bytes.extend_from_slice(&out[(y * pw + x) * 3 + c].to_le_bytes());
            }
        }
    }
    std::fs::write(&a[2], bytes).expect("write output");
}
