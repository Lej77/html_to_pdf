/// A string that can be included in a file path that specifies the dynamically
/// linked library's version.
pub const WK_HTML_LIBRARY_VERSION: &str = "v0.12.3";

#[cfg(all(supported_target, feature = "should_link"))]
mod stuff {
    pub use wkhtmltopdf::*;

    pub fn convert_html_to_pdf<W: std::io::Write>(
        html: impl AsRef<str>,
        mut writer: W,
    ) -> Result<()> {
        let mut pdf_app = PdfApplication::new().expect("Failed to init PDF application");
        let mut builder = pdf_app.builder();
        builder.orientation(Orientation::Portrait);
        // builder.margin(Size::Inches(2));
        // builder.dpi(72);
        builder.page_size(PageSize::A6);
        let mut pdf_out = builder
            .build_from_html(html.as_ref())
            .expect("Failed to build pdf");

        std::io::copy(&mut pdf_out, &mut writer)?;
        Ok(())
    }
}

#[cfg(all(supported_target, not(feature = "should_link")))]
mod stuff {
    #[cfg(feature = "compression")]
    include!(concat!(env!("OUT_DIR"), "/compressed.rs"));

    #[cfg(not(feature = "compression"))]
    pub static WK_HTML_TO_PDF_DLL: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/wkhtmltox.dll"));
}

#[cfg(not(supported_target))]
mod stuff {
    pub static WK_HTML_TO_PDF_DLL: &[u8] = &[];

    pub fn convert_html_to_pdf<W: std::io::Write>(
        html: impl AsRef<str>,
        mut writer: W,
    ) -> Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "wkhtmltopdf doesn't support this target",
        ))
    }
}

#[doc(inline)]
pub use stuff::*;
