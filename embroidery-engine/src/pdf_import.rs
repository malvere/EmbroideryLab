//! Bounded PDF page rasterization for the same artwork workflow as PNG/JPEG.
//! PDFium is loaded from the app package; PDF pages remain source artwork,
//! rather than being described as editable PDF vectors.

use image::DynamicImage;
use pdfium_render::prelude::*;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const MAX_PDF_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;

#[derive(Debug)]
pub struct RenderedPage {
    pub image: DynamicImage,
    pub page_count: usize,
}

fn renderer() -> Result<&'static Pdfium, String> {
    static RENDERER: OnceLock<Result<Pdfium, String>> = OnceLock::new();
    RENDERER
        .get_or_init(|| {
            if let Some(path) = std::env::var_os("EMBROIDERYLAB_PDFIUM_PATH") {
                let mut path = PathBuf::from(path);
                if path.is_dir() {
                    path.push(Pdfium::pdfium_platform_library_name());
                }
                return Pdfium::bind_to_library(path).map(Pdfium::new).map_err(|_| {
                    "Cannot load the configured PDFium library. Check EMBROIDERYLAB_PDFIUM_PATH."
                        .into()
                });
            }
            let mut candidates = Vec::new();
            if let Ok(executable) = std::env::current_exe()
                && let Some(directory) = executable.parent()
            {
                candidates.push(directory.join(Pdfium::pdfium_platform_library_name()));
                // Rust integration tests are in target/{profile}/deps, while
                // build.ps1 places the runtime beside the app binary.
                if directory.file_name().is_some_and(|name| name == "deps")
                    && let Some(parent) = directory.parent()
                {
                    candidates.push(parent.join(Pdfium::pdfium_platform_library_name()));
                }
            }
            for path in candidates {
                if let Ok(bindings) = Pdfium::bind_to_library(path) {
                    return Ok(Pdfium::new(bindings));
                }
            }
            Pdfium::bind_to_system_library()
                .map(Pdfium::new)
                .map_err(|_| {
                    "PDF support is unavailable. Reinstall the complete EmbroideryLab package."
                        .into()
                })
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn document_error(error: PdfiumError) -> String {
    match error {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            "This PDF requires a password. Open an unprotected copy.".into()
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            "This PDF uses unsupported security settings. Open an unprotected copy.".into()
        }
        _ => "PDF file is invalid or damaged.".into(),
    }
}

/// Render one zero-based PDF page to at most `longest_side` pixels per axis.
/// The image retains the displayed page aspect ratio on an opaque white page.
/// Other pages are neither rendered nor imported into the embroidery project.
pub fn render_page(
    path: &Path,
    page_index: usize,
    longest_side: u32,
) -> Result<RenderedPage, String> {
    if !(16..=2048).contains(&longest_side) {
        return Err("PDF render size must be 16–2048 pixels".into());
    }
    let file = File::open(path).map_err(|error| format!("Cannot read PDF: {error}"))?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_PDF_BYTES {
        return Err("PDF file exceeds 128MB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PDF_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read PDF: {error}"))?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err("PDF file exceeds 128MB".into());
    }
    let pdfium = renderer()?;
    let document = pdfium
        .load_pdf_from_byte_vec(bytes, None)
        .map_err(document_error)?;
    let count = document.pages().len();
    if count <= 0 {
        return Err("PDF contains no pages".into());
    }
    let page_count = count as usize;
    if page_count > MAX_PDF_PAGES {
        return Err("PDF exceeds 10000 pages".into());
    }
    if page_index >= page_count {
        return Err(format!(
            "PDF page number must be between 1 and {page_count}"
        ));
    }
    let page = document
        .pages()
        .get(page_index as PdfPageIndex)
        .map_err(|_| "Cannot open this PDF page".to_owned())?;
    let width = f64::from(page.width().value);
    let height = f64::from(page.height().value);
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err("PDF page has invalid dimensions".into());
    }
    let scale = f64::from(longest_side) / width.max(height);
    let width = (width * scale).round().clamp(1., f64::from(longest_side)) as i32;
    let height = (height * scale).round().clamp(1., f64::from(longest_side)) as i32;
    let config = PdfRenderConfig::new()
        .set_fixed_size(width, height)
        .set_clear_color(PdfColor::WHITE)
        .clear_before_rendering(true)
        .limit_render_image_cache_size(true);
    let bitmap = page
        .render_with_config(&config)
        .map_err(|_| "Cannot render this PDF page".to_owned())?;
    let mut image = bitmap
        .as_image()
        .map_err(|_| "Cannot render this PDF page".to_owned())?
        .into_rgba8();
    // The white clear normally makes every pixel opaque. Composite explicitly
    // as well, so downstream background picking never sees transparent PDF paper.
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for component in &mut pixel.0[..3] {
            *component = ((u32::from(*component) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
    Ok(RenderedPage {
        image: DynamicImage::ImageRgba8(image),
        page_count,
    })
}
