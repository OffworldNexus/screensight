//! Developer tool: render representative device screens to PPM files so the
//! panel output can be inspected without a GPU or compositor.
//!
//! ```sh
//! cargo run -p screensight --example dump-screens -- /tmp/screens
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use screensight::raster::FrameCanvas;
use screensight::runtime::Screen;
use screensight::screens;

fn write_ppm(path: &Path, canvas: &FrameCanvas) {
    let (w, h) = (canvas.width(), canvas.height());
    let mut out = Vec::with_capacity(w * h * 3 + 32);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in canvas.frame().as_chunks::<4>().0 {
        // BGRA -> RGB
        out.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    fs::write(path, out).expect("writing PPM");
}

fn dashboard(text: &str) -> Screen {
    Screen::Dashboard {
        values: BTreeMap::from([("text".to_owned(), text.to_owned())]),
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/opencode/screensight-screens".to_owned());
    fs::create_dir_all(&dir).expect("creating output directory");

    let screens: Vec<(&str, Screen, f32)> = vec![
        (
            "splash",
            Screen::Splash {
                name: "Brave Otter".to_owned(),
            },
            0.0,
        ),
        ("idle", Screen::Idle, 0.0),
        (
            "pairing-waiting",
            Screen::PairingWaiting {
                name: "Brave Otter".to_owned(),
            },
            0.6,
        ),
        (
            "pairing-handshake",
            Screen::PairingHandshake {
                name: "Brave Otter".to_owned(),
            },
            0.3,
        ),
        (
            "pairing-code",
            Screen::PairingCode {
                sas: "93704101".to_owned(),
            },
            0.0,
        ),
        ("pairing-error", Screen::PairingError, 0.0),
        (
            "confirm",
            Screen::Confirm {
                ha_name: "Home Assistant / My home".to_owned(),
            },
            0.0,
        ),
        (
            "dashboard-text",
            dashboard("It's going to rain from 3pm until 4pm 🌧"),
            0.0,
        ),
        (
            "dashboard-unicode",
            dashboard("Studio · 21.4°C · café ☕ · こんにちは · Ω≈ç√"),
            0.0,
        ),
        (
            "dashboard-empty",
            Screen::Dashboard {
                values: BTreeMap::new(),
            },
            0.0,
        ),
    ];

    for (name, screen, elapsed) in screens {
        let (canvas, hits) = screens::frame_for_with(&screen, None, elapsed);
        let path = Path::new(&dir).join(format!("{name}.ppm"));
        write_ppm(&path, &canvas);
        println!("{} ({} hit regions)", path.display(), hits.len());
    }

    // A full loader cycle makes geometry and motion review possible without
    // depending on the emulator's desktop compositor or a running HA server.
    let waiting = Screen::PairingWaiting {
        name: "Brave Otter".to_owned(),
    };
    for index in 0..48 {
        let (canvas, _) = screens::frame_for_with(&waiting, None, index as f32 * 0.05);
        write_ppm(
            &Path::new(&dir).join(format!("loader-{index:02}.ppm")),
            &canvas,
        );
    }
}
