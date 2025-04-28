//! Provides an adapter that implements `html_to_pdf`'s trait using [`pdf-min`].
//!
//! [`pdf-min`]: https://crates.io/crates/pdf-min

use html_to_pdf::{HtmlSink, HtmlToPdfConverter, WriteBuilder};
use std::{
    io::{self, Error, Write},
    marker::PhantomData,
};

#[derive(Debug, Clone, Default)]
pub struct PdfMinConverter;

impl<'scope, W> HtmlToPdfConverter<'scope, W> for PdfMinConverter
where
    W: WriteBuilder + Send + 'scope,
{
    type HtmlSink = PdfMinHtmlSink<'scope, W>;
    type Error = Error;

    fn start(
        self,
        _scope: html_to_pdf::PdfScope<'scope, '_>,
        output: W,
    ) -> Result<Self::HtmlSink, Self::Error> {
        Ok(PdfMinHtmlSink {
            buffer: Vec::new(),
            writer: output,
            _scope: PhantomData,
        })
    }
}
impl<'scope, W> HtmlSink<W, Error> for PdfMinHtmlSink<'scope, W>
where
    W: WriteBuilder + Send + 'scope,
{
    fn complete(mut self) -> Result<W, Error> {
        let mut writer = self.writer.get_writer()?;
        let mut w = ::pdf_min::Writer::default();
        w.b.nocomp = true;
        w.line_pad = 8; // Other Writer default values could be adjusted here.

        const UTF8_BOM: &[u8] = "\u{feff}".as_bytes();
        let text = if self.buffer.starts_with(UTF8_BOM) {
            &self.buffer[UTF8_BOM.len()..]
        } else {
            self.buffer.as_slice()
        };
        ::pdf_min::html(&mut w, text);
        w.finish();

        writer.write_all(&w.b.b)?;
        drop(writer);
        Ok(self.writer)
    }
}

pub struct PdfMinHtmlSink<'scope, W> {
    buffer: Vec<u8>,
    writer: W,
    _scope: PhantomData<&'scope ()>,
}
impl<'scope, W> Write for PdfMinHtmlSink<'scope, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
