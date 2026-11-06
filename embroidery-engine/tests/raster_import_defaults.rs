use embroidery_lab::import::{self, RasterOptions};
use image::{DynamicImage, Rgba, RgbaImage};
use std::path::Path;

#[test]
fn interactive_defaults_report_substitch_speckles_without_exceeding_the_object_limit() {
    // One 100mm-wide pixel here is ~0.068mm²: large enough to defeat the old
    // 0.04mm² preset, while too small for a reliable filled embroidery detail.
    let mut pixels = RgbaImage::new(383, 512);
    let red = Rgba([200, 20, 30, 255]);
    for y in 0..48 {
        for x in 0..48 {
            pixels.put_pixel(x, y, red);
        }
    }
    // Retain a genuine four-pixel (~0.273mm²) detail as an independent region.
    for y in 64..66 {
        for x in 64..66 {
            pixels.put_pixel(x, y, red);
        }
    }
    for y in (70..512).step_by(4) {
        for x in (70..383).step_by(4) {
            pixels.put_pixel(x, y, red);
        }
    }
    let options = RasterOptions::autodigitize();
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(pixels.clone()),
        Path::new("synthetic-flat-art-with-speckles.png"),
        100.,
        &options,
    )
    .unwrap();
    assert_eq!(project.objects.len(), 2);
    assert!(report.omitted_region_count > 4096);
    assert!(report.omitted_regions.iter().all(|r| r.pixels == 1));
    assert_eq!(
        project
            .source_image
            .as_ref()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8(),
        pixels
    );
    assert_eq!(project.raster_recipe.as_ref().unwrap().options, options);
    assert!(
        project
            .raster_recipe
            .as_ref()
            .unwrap()
            .issues
            .iter()
            .any(
                |i| i.message.contains("Raster import omitted") && i.message.contains("aggregated")
            )
    );
    assert_eq!(
        RasterOptions::default().min_region_area_mm2,
        0.,
        "Historical imports and existing recipes keep their prior policy"
    );
}
