//! Procedurally rasterized window/taskbar icon — a phosphor-gold anvil on a
//! transparent field, in the shared ThoughtSlayer/Vault palette. Drawn in code
//! so the binary stays self-contained (no asset pipeline). Consumed both at
//! runtime (main.rs window icon) and at build time (build.rs → embedded .ico).

pub struct IconRaster {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn icon_raster() -> IconRaster {
    const SIZE: u32 = 128;
    let w = SIZE as usize;
    let h = SIZE as usize;
    let mut rgba = vec![0u8; w * h * 4];

    for y in 0..h {
        // Vertical metal gradient: bright phosphor gold at the face (theme's
        // COLOR_GOLD_BRIGHT), a dim amber at the foot.
        let t = (y as f32 / h as f32).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| (a * (1.0 - t) + b * t) as u8;
        let (r, g, b) = (lerp(240.0, 140.0), lerp(210.0, 110.0), lerp(100.0, 40.0));
        for x in 0..w {
            if anvil_contains(x, y) {
                let index = (y * w + x) * 4;
                rgba[index] = r;
                rgba[index + 1] = g;
                rgba[index + 2] = b;
                rgba[index + 3] = 255;
            }
        }
    }

    IconRaster {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

/// Anvil silhouette in 128px space: flat face + pointed horn, narrow waist,
/// flared base, and a foot. Tuned to still read as an anvil at 16px.
fn anvil_contains(x: usize, y: usize) -> bool {
    let x = x as i32;
    let y = y as i32;

    let face = (24..100).contains(&x) && (32..50).contains(&y);
    let horn = if (32..50).contains(&y) {
        let left = 12 + (y - 41).abs(); // narrows away from the centerline
        (left..24).contains(&x)
    } else {
        false
    };
    let waist = (52..80).contains(&x) && (50..84).contains(&y);
    let base = (28..100).contains(&x) && (84..100).contains(&y);
    let foot = (22..106).contains(&x) && (100..108).contains(&y);

    face || horn || waist || base || foot
}
