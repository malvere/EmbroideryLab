//! Reproducible colour-selection latency measurement, no artwork dependencies.
use embroidery_lab::color_selection::{SelectionCache, SelectionMode};
use image::{Rgba, RgbaImage};
use serde_json::json;
use std::time::Instant;

fn main() {
    let mut measurements = vec![];
    for size in [512, 2048] {
        let image = RgbaImage::from_fn(size, size, |x, y| {
            if x < size / 8 || y < size / 8 || x >= size * 7 / 8 || y >= size * 7 / 8 {
                let v = 245 + ((x * 3 + y * 5) % 11) as u8;
                Rgba([v, v, v, 255])
            } else if x == size / 8 || y == size / 8 {
                Rgba([0, 0, 0, 255])
            } else if y < size / 2 {
                Rgba([210, 25, 40, 255])
            } else {
                Rgba([20, 60, 190, 255])
            }
        });
        let start = Instant::now();
        let cache = SelectionCache::new(&image, [0, 0]).unwrap();
        let cache_ms = start.elapsed().as_secs_f64() * 1000.;
        for mode in [
            SelectionMode::Connected,
            SelectionMode::Exterior,
            SelectionMode::Global,
        ] {
            let mut times = vec![];
            let mut selected = 0;
            for i in 0..25 {
                let start = Instant::now();
                let mask = cache.candidates(3. + (i % 5) as f32, mode);
                let elapsed = start.elapsed().as_secs_f64() * 1000.;
                selected = mask.iter().filter(|&&x| x).count();
                if i > 0 {
                    times.push(elapsed);
                }
            }
            times.sort_by(f64::total_cmp);
            measurements.push(json!({"source_pixels":[size,size],"mode":format!("{mode:?}"),"cache_ms":cache_ms,
                "cached_mask_median_ms":times[times.len()/2],"cached_mask_p95_ms":times[(times.len()*95/100).min(times.len()-1)],
                "samples":times.len(),"selected_pixels":selected}));
        }
    }
    let report = json!({"benchmark":"fixed-source OKLab selection; cache once, recompute masks at changing thresholds",
        "build":if cfg!(debug_assertions){"debug"}else{"release"},
        "hardware":std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default(),
        "available_parallelism":std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1),
        "measurements":measurements});
    let text = serde_json::to_string_pretty(&report).unwrap();
    if let Some(path) = std::env::args_os().nth(1) {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, &text).unwrap();
    }
    println!("{text}");
}
