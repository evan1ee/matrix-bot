//! PDF receipts: rasterise the first pages so the image OCR can read them.
//!
//! Bounded on purpose. A PDF is attacker-shaped input (it arrives by email), so
//! the page count and the bitmap size are fixed here rather than taken from the
//! file: a 5000-page document or a 200 000 pt wide `MediaBox` must cost the
//! same as a one-page receipt.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

/// Pages read from one document; a receipt is one or two, the rest is terms.
pub const MAX_PDF_PAGES: usize = 3;
/// Longest bitmap side. ~2000 px keeps small print legible at A4 for OCR.
const MAX_SIDE_PX: f32 = 2000.0;
/// Small pages (a till slip is ~200 pt wide) are enlarged, but not without limit.
const MAX_SCALE: f32 = 4.0;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PdfError {
    #[error("not a readable PDF (encrypted or corrupt): {0}")]
    Unreadable(String),
    #[error("PDF has no pages")]
    NoPages,
    #[error("PDF page has an unusable size")]
    BadPageSize,
    #[error("could not encode a rendered page: {0}")]
    Encode(String),
}

pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Renders at most [`MAX_PDF_PAGES`] pages to PNG, in page order.
pub fn render_pages(bytes: &[u8]) -> Result<Vec<Vec<u8>>, PdfError> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| PdfError::Unreadable(format!("{e:?}")))?;
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();

    let mut pages = Vec::new();
    for page in pdf.pages().iter().take(MAX_PDF_PAGES) {
        let (w, h) = page.render_dimensions();
        let scale = page_scale(w, h).ok_or(PdfError::BadPageSize)?;
        let pixmap = render(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        pages.push(
            pixmap
                .into_png()
                .map_err(|e| PdfError::Encode(format!("{e:?}")))?,
        );
    }
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    Ok(pages)
}

/// Scale that puts the longest side at [`MAX_SIDE_PX`], or `None` for a page
/// whose size is not a positive finite number.
fn page_scale(width: f32, height: f32) -> Option<f32> {
    let longest = width.max(height);
    if !(width > 0.0 && height > 0.0 && longest.is_finite()) {
        return None;
    }
    Some((MAX_SIDE_PX / longest).min(MAX_SCALE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid PDF with `pages` blank pages of `w` x `h` points.
    fn pdf(pages: usize, w: u32, h: u32) -> Vec<u8> {
        let mut objs = vec![String::from("<< /Type /Catalog /Pages 2 0 R >>")];
        let kids: String = (0..pages).map(|i| format!("{} 0 R ", 3 + i)).collect();
        objs.push(format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"));
        for _ in 0..pages {
            objs.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] >>"
            ));
        }
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
        }
        let xref = out.len();
        out.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objs.len() + 1
        ));
        for off in offsets {
            out.push_str(&format!("{off:010} 00000 n \n"));
        }
        out.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        ));
        out.into_bytes()
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(png).expect("a rendered page is a PNG");
        (img.width(), img.height())
    }

    #[test]
    fn recognises_the_pdf_magic_only() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(!is_pdf(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_pdf(b""));
    }

    #[test]
    fn a_one_page_pdf_renders_one_png() {
        let pages = render_pages(&pdf(1, 595, 842)).unwrap();
        assert_eq!(pages.len(), 1);
        let height = png_size(&pages[0]).1;
        assert!((1990..=2000).contains(&height), "got {height}");
    }

    #[test]
    fn only_the_first_three_pages_are_rendered() {
        let pages = render_pages(&pdf(5, 200, 300)).unwrap();
        assert_eq!(pages.len(), MAX_PDF_PAGES);
    }

    #[test]
    fn a_huge_page_is_scaled_down_to_the_bitmap_cap() {
        let pages = render_pages(&pdf(1, 200_000, 100)).unwrap();
        let (w, h) = png_size(&pages[0]);
        assert!(w <= 2000 && h >= 1, "got {w}x{h}");
    }

    #[test]
    fn a_tiny_page_is_enlarged_but_only_up_to_the_scale_cap() {
        let pages = render_pages(&pdf(1, 100, 50)).unwrap();
        assert_eq!(png_size(&pages[0]), (400, 200));
    }

    #[test]
    fn garbage_after_the_magic_is_unreadable() {
        let err = render_pages(b"%PDF-1.4 this is not a pdf").unwrap_err();
        assert!(matches!(err, PdfError::Unreadable(_) | PdfError::NoPages));
    }

    #[test]
    fn a_zero_sized_page_is_refused() {
        assert_eq!(page_scale(0.0, 100.0), None);
        assert_eq!(page_scale(f32::INFINITY, 100.0), None);
        assert_eq!(page_scale(f32::NAN, 100.0), None);
    }
}
