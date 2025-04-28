use std::{
    ffi::OsString,
    io::{self, BufReader, BufWriter, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};

use eyre::{bail, Context, ContextCompat, Result};
use html_to_pdf::{HtmlSink, HtmlToPdfConverter, PdfScope, PdfScopedJoinHandle, WriteBuilder};

#[cfg(feature = "include_exe")]
static EMBEDDED_CONVERTER: include_dir::Dir =
    include_dir::include_dir!("$OUT_DIR/HtmlToPdf_Framework/bin/Release");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DotNetFrameworkPdfConverterMode {
    /// Allow the .Net converter program to choose one of the mode, might change
    /// with newer versions.
    #[default]
    Default = 0,
    ObsoleteHTMLParser,
    XMLWorkerSimple,
    XMLWorkerAdvanced,
}
impl DotNetFrameworkPdfConverterMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            DotNetFrameworkPdfConverterMode::Default => "Default",
            DotNetFrameworkPdfConverterMode::ObsoleteHTMLParser => "HTMLParse_ObsoleteHTMLParser",
            DotNetFrameworkPdfConverterMode::XMLWorkerSimple => "HTMLParse_XMLWorkerSimple",
            DotNetFrameworkPdfConverterMode::XMLWorkerAdvanced => "HTMLParse_XMLWorkerAdvanced",
        }
    }
}

/// Use a small C# program to generate a PDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DotNetFrameworkPdfConverter {
    /// The program supports different modes since the C# library it uses
    /// has different ways to handle the conversion.
    pub mode: DotNetFrameworkPdfConverterMode,
    /// If mode `1` is used then a custom string can be used to indicate page
    /// breaks in the HTML input.
    pub custom_page_break: Option<OsString>,
    /// Extract executable that was embedded into the program at compile time to
    /// this location, and then run them.
    pub extract_included_exe_at: Option<PathBuf>,
}
pub const RECOMMENDED_PAGE_BREAK: &str = "_____CUSTOM_PAGE_BREAK_____";

impl<'scope, W> HtmlToPdfConverter<'scope, W> for DotNetFrameworkPdfConverter
where
    W: WriteBuilder + Send + 'scope,
{
    type HtmlSink = DotNetFrameworkHtmlSink<'scope, W>;
    type Error = eyre::Error;

    fn start(
        self,
        scope: PdfScope<'scope, '_>,
        mut output: W,
    ) -> Result<Self::HtmlSink, Self::Error> {
        #[allow(unused_mut)]
        let mut program_path = OsString::from("HtmlToPdf_Framework");
        #[cfg(feature = "include_exe")]
        if let Some(path) = self.extract_included_exe_at.as_deref() {
            if !path.exists() {
                std::fs::create_dir_all(path)
                    .with_context(|| format!("Failed to create folder at: {}", path.display()))?;
                EMBEDDED_CONVERTER.extract(path).context(
                    "Failed to extract HtmlToPdf_Framework.exe that was \
                    embedded into the program at compile time",
                )?;
            }
            program_path = path.join("HtmlToPdf_Framework").into();
        }
        #[cfg(not(feature = "include_exe"))]
        if self.extract_included_exe_at.is_some() {
            eyre::bail!(
                "Can't extract HtmlToPdf_Framework.exe since it was \
                not embedded into the program when it was compiled"
            );
        }

        let DotNetFrameworkPdfConverter { mode, .. } = self;
        let mut process = Command::new(&program_path);
        #[cfg(all(windows, feature = "windows-gui"))]
        {
            use std::os::windows::process::CommandExt;

            // Hide console window:
            // https://stackoverflow.com/questions/6371149/what-is-the-difference-between-detach-process-and-create-no-window-process-creat
            // https://learn.microsoft.com/sv-se/windows/win32/procthread/process-creation-flags?redirectedfrom=MSDN
            // Need "CREATE_NO_WINDOW" if the created process will spawn its own sub-processes,
            // otherwise DETACHED_PROCESS is enough to prevent a console from being opened.
            process.creation_flags(/*CREATE_NO_WINDOW*/ 0x08000000);
        }
        process.arg(mode.as_arg());

        if let DotNetFrameworkPdfConverter {
            custom_page_break: Some(custom_page_break),
            ..
        } = self
        {
            // Handle page breaks manually in this mode by inserting magic string:
            process.arg(custom_page_break);
        }

        let mut process = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to start \"HtmlToPdf_Framework.exe\" in order to convert HTML to PDF.\
                    \n\tExecutable location: \"{}\"",
                    PathBuf::from(program_path).display()
                )
            })?;

        let pdf_reader = process
            .stdout
            .take()
            .context(r#"Couldn't open stdout for "HtmlToPdf_Framework.exe" conversion program."#)?;
        let pdf_writer = process
            .stdin
            .take()
            .context(r#"Couldn't open stdin for "HtmlToPdf_Framework.exe" conversion program."#)?;

        let reader_thread =
            scope.spawn(move || -> Result<_> {
                let mut pdf_reader = BufReader::new(pdf_reader);
                // Read piped "ToPdf" stdout and redirect it to our output writer:

                io::copy(&mut pdf_reader, &mut output.get_writer()?).context(
                r#"Failed to read pdf data from "HtmlToPdf_Framework" program's stdout and write it to output."#
            )?;
                Ok(output)
            });

        Ok(DotNetFrameworkHtmlSink(DotNetFrameworkHtmlSinkInner {
            process,
            reader_thread,
            writer: BufWriter::new(pdf_writer),
        }))
    }
}
impl<'scope, W> HtmlSink<W, eyre::Error> for DotNetFrameworkHtmlSink<'scope, W>
where
    W: WriteBuilder + Send + 'scope,
{
    fn complete(self) -> eyre::Result<W> {
        let DotNetFrameworkHtmlSink(DotNetFrameworkHtmlSinkInner {
            mut process,
            writer,
            reader_thread,
        }) = self;

        // The HtmlToPdf_Framework conversion program's stdin pipe was owned by
        // the writer which we now drop. The HtmlToPdf_Framework program should
        // therefore exit when it has finished processing its data.
        drop(writer);

        let exit_status = process.wait().context(
            r#"Failed to wait for the "HtmlToPdf_Framework" conversion program to exit."#,
        )?;

        if let Some(error_code) = exit_status.code() {
            if error_code != 0 {
                bail!(
                    r#"The "HtmlToPdf_Framework" conversion program exited with an error (code: {})."#,
                    error_code
                );
            }
        } else {
            bail!(
                r#"The "HtmlToPdf_Framework" conversion program exited with an error (no exit code)."#
            );
        };
        // The worker thread should finish now that stdout for "HtmlToPdf_Framework" has been closed.
        reader_thread.join().unwrap()
    }
}

struct DotNetFrameworkHtmlSinkInner<'scope, W> {
    process: Child,
    writer: BufWriter<ChildStdin>,
    reader_thread: PdfScopedJoinHandle<'scope, Result<W>>,
}
pub struct DotNetFrameworkHtmlSink<'scope, W>(DotNetFrameworkHtmlSinkInner<'scope, W>);
impl<W> DotNetFrameworkHtmlSink<'_, W> {
    #[inline]
    fn writer(&mut self) -> &mut BufWriter<ChildStdin> {
        &mut self.0.writer
    }
}
impl<W> Write for DotNetFrameworkHtmlSink<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer().flush()
    }
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.writer().write_vectored(bufs)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.writer().write_all(buf)
    }
    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
        self.writer().write_fmt(fmt)
    }
}
