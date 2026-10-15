//! Characterization of the whole effect chain on synthetic frames, with the
//! mask, faces and auto-light gain injected so no model is needed.

use super::*;
use crate::inference::models::{Face, Mask};

const GOLDEN_W: usize = 96;
const GOLDEN_H: usize = 64;
const GOLDEN_FRAMES: usize = 24;

/// A drifting gradient with a brighter oval "subject" that slides right.
fn golden_frame(i: usize) -> Vec<u8> {
    let (w, h) = (GOLDEN_W, GOLDEN_H);
    let cx = 0.35 + i as f32 * 0.01;
    let mut img = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let d = ((u - cx) / 0.18).powi(2) + ((v - 0.55) / 0.35).powi(2);
            let p = (y * w + x) * 4;
            if d < 1.0 {
                img[p..p + 4].copy_from_slice(&[200, (120 + x) as u8, 90, 255]);
            } else {
                img[p..p + 4].copy_from_slice(&[
                    (x * 2 + i) as u8,
                    (y * 3) as u8,
                    (255 - x - y + i) as u8,
                    255,
                ]);
            }
        }
    }
    img
}

/// A soft oval matching `golden_frame`'s subject, as the segmenter
/// would return it.
fn golden_mask(i: usize) -> Mask {
    let size = 32;
    let cx = 0.35 + i as f32 * 0.01;
    let mut values = vec![0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let d = ((u - cx) / 0.18).powi(2) + ((v - 0.55) / 0.35).powi(2);
            values[y * size + x] = (1.5 - d).clamp(0.0, 1.0);
        }
    }
    Mask { size, values }
}

fn golden_face(i: usize) -> Face {
    let cx = 0.35 + i as f32 * 0.01;
    Face {
        cx,
        cy: 0.4,
        w: 0.2,
        h: 0.25,
        score: 0.9,
        eye_cx: cx,
        eye_cy: 0.35,
        eye_dist: 0.08,
    }
}

/// Every model pre-failed, so nothing loads and the injected mask, faces
/// and gain map are what the effects see, models installed or not.
fn offline_effects(settings: Settings) -> Effects {
    Effects {
        settings,
        engine: Engine::offline(),
        ..Effects::default()
    }
}

/// SHA-256 of every output frame of the golden clip under `settings`.
fn golden_digest(settings: Settings) -> String {
    use sha2::{Digest, Sha256};
    let mut fx = offline_effects(settings);
    if fx.settings.auto_light_enabled {
        let values = (0..64).map(|i| 1.0 + (i % 8) as f32 * 0.15).collect();
        fx.auto_light.gain = Some(Mask { size: 8, values });
        fx.auto_light.gate = 0.7;
    }
    if fx.settings.reactions_enabled {
        fx.trigger_reaction("heart");
    }
    let mut hasher = Sha256::new();
    let mut changed = false;
    for i in 0..GOLDEN_FRAMES {
        let mut img = golden_frame(i);
        fx.mask.mask = Some(golden_mask(i));
        fx.mask.generation = fx.mask.generation.wrapping_add(1);
        fx.center_stage.faces = vec![golden_face(i)];
        fx.process(&mut img, GOLDEN_W, GOLDEN_H);
        changed |= img != golden_frame(i);
        hasher.update(&img);
    }
    assert!(changed, "no frame was touched, so the digest pins nothing");
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn golden_bg_image() -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
    let img = image::RgbaImage::from_fn(40, 30, |x, y| {
        image::Rgba([(x * 6) as u8, (y * 8) as u8, 128, 255])
    });
    img.save(file.path()).unwrap();
    file
}

/// Characterization of the whole effect chain on synthetic frames. The
/// digests pin today's output byte for byte; a pure move or refactor
/// must leave them unchanged.
#[test]
fn golden_frames_are_unchanged() {
    let bg = golden_bg_image();
    let cases = [
        (
            "studio + blur",
            Settings {
                studio_enabled: true,
                studio_brightness: 0.2,
                studio_contrast: 1.4,
                studio_bg_brightness: -0.2,
                blur_enabled: true,
                blur_strength: 70,
                ..Settings::default()
            },
            "521306030b1da5e3a18dcb08e6538f6804c61b37027e7f6d16b3eb9142b940f9",
        ),
        (
            "light blur",
            Settings {
                blur_enabled: true,
                blur_strength: 10,
                ..Settings::default()
            },
            "6a00c6ec2826a04bc7775507b5a218aac1c757004856be162ea7979a19889978",
        ),
        (
            "bg color + studio",
            Settings {
                bg_enabled: true,
                bg_path: "#204060".into(),
                studio_enabled: true,
                studio_brightness: 0.1,
                ..Settings::default()
            },
            "13aca9e564d8f9bf2140f658029a21f4ec77c6e51390d3357f5534d08a1deda0",
        ),
        (
            "bg image",
            Settings {
                bg_enabled: true,
                bg_path: bg.path().to_str().unwrap().into(),
                ..Settings::default()
            },
            "79291583db6f4209a3d0ac440ecbde24906cde9b1056ab7926f7313437aef8b3",
        ),
        (
            "center stage only",
            Settings {
                cs_enabled: true,
                cs_zoom: "tight".into(),
                ..Settings::default()
            },
            "fead953b38f7f4a6b20ac3c73312564ac54b67f29db4e5c33e83b021c10328b9",
        ),
        (
            "center stage + blur",
            Settings {
                cs_enabled: true,
                blur_enabled: true,
                ..Settings::default()
            },
            "4a5008bc1c1d4f1361366bada3d8cf75eb306445445a95d2d0aa912f2a662f63",
        ),
        (
            "center stage group + bg color",
            Settings {
                cs_enabled: true,
                cs_mode: "group".into(),
                cs_zoom: "subtle".into(),
                bg_enabled: true,
                bg_path: "#102030".into(),
                ..Settings::default()
            },
            "994ffd33fc5e614d9c926cc3fee3e380187fb35dc128849667981b90e65bcfb3",
        ),
        (
            "auto light + reactions",
            Settings {
                auto_light_enabled: true,
                reactions_enabled: true,
                ..Settings::default()
            },
            "b653c5fa444a0f76bba16c5af68b59041d335c007d67a849a3c5e9b0d1f54379",
        ),
    ];
    let mut failures = Vec::new();
    for (name, settings, want) in cases {
        let got = golden_digest(settings);
        if got != want {
            failures.push(format!("{name}: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "digests changed:\n{}",
        failures.join("\n")
    );
}
