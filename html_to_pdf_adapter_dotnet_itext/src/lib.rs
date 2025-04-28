use std::{
    ffi::OsString,
    io::{self, BufReader, BufWriter, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};

use eyre::{bail, Context, ContextCompat, Result};
use html_to_pdf::{HtmlSink, HtmlToPdfConverter, PdfScope, PdfScopedJoinHandle, WriteBuilder};

#[cfg(all(feature = "include_exe", feature = "compression"))]
include!(concat!(env!("OUT_DIR"), "/compressed.rs"));

#[cfg(all(feature = "include_exe", not(feature = "compression")))]
fn embedded_converter() -> &'static [u8] {
    static EMBEDDED_CONVERTER: &[u8] = 'data: {
        #[cfg(windows)]
        break 'data include_bytes!(concat!(env!("OUT_DIR"), "/HtmlToPdf_Publish/HtmlToPdf.exe"));
        #[cfg(not(windows))]
        break 'data include_bytes!(concat!(env!("OUT_DIR"), "/HtmlToPdf_Publish/HtmlToPdf"));
    };
    EMBEDDED_CONVERTER
}

/// Use a small C# program to generate a PDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DotNetPdfConverter {
    /// Extract executable that was embedded into the program at compile time to
    /// this location, and then run them.
    pub extract_included_exe_at: Option<PathBuf>,
}

impl<'scope, W> HtmlToPdfConverter<'scope, W> for DotNetPdfConverter
where
    W: WriteBuilder + Send + 'scope,
{
    type HtmlSink = DotNetHtmlSink<'scope, W>;
    type Error = eyre::Error;

    fn start(
        self,
        scope: PdfScope<'scope, '_>,
        mut output: W,
    ) -> Result<Self::HtmlSink, Self::Error> {
        #[allow(unused_mut)]
        let mut program_path = OsString::from("HtmlToPdf");
        #[cfg(feature = "include_exe")]
        if let Some(path) = self.extract_included_exe_at.as_deref() {
            if !path.exists() {
                std::fs::create_dir_all(path)
                    .with_context(|| format!("Failed to create folder at: {}", path.display()))?;
                std::fs::write(
                    path.join(if cfg!(windows) {
                        "HtmlToPdf.exe"
                    } else {
                        "HtmlToPdf"
                    }),
                    embedded_converter(),
                )
                .context(
                    "Failed to extract HtmlToPdf.exe that was \
                    embedded into the program at compile time",
                )?;
            }
            program_path = path.join("HtmlToPdf").into();
        }
        #[cfg(not(feature = "include_exe"))]
        if self.extract_included_exe_at.is_some() {
            eyre::bail!(
                "Can't extract HtmlToPdf.exe since it was \
                not embedded into the program when it was compiled"
            );
        }

        let mut process = Command::new(program_path);
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

        let mut process = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context(r#"Failed to start "HtmlToPdf" in order to convert HTML to PDF."#)?;

        let pdf_reader = process
            .stdout
            .take()
            .context(r#"Couldn't open stdout for "HtmlToPdf" conversion program."#)?;
        let pdf_writer = process
            .stdin
            .take()
            .context(r#"Couldn't open stdin for "HtmlToPdf" conversion program."#)?;

        let reader_thread =
            scope.spawn(move || -> Result<_> {
                let mut pdf_reader = BufReader::new(pdf_reader);
                // Read piped "ToPdf" stdout and redirect it to our output writer:

                io::copy(&mut pdf_reader, &mut output.get_writer()?).context(
                r#"Failed to read pdf data from "HtmlToPdf" program's stdout and write it to output."#
            )?;
                Ok(output)
            });

        Ok(DotNetHtmlSink(DotNetHtmlSinkInner {
            process,
            reader_thread,
            writer: BufWriter::new(pdf_writer),
        }))
    }
}
impl<'scope, W> HtmlSink<W, eyre::Error> for DotNetHtmlSink<'scope, W>
where
    W: WriteBuilder + Send + 'scope,
{
    fn complete(self) -> eyre::Result<W> {
        let DotNetHtmlSink(DotNetHtmlSinkInner {
            mut process,
            writer,
            reader_thread,
        }) = self;

        // The HtmlToPdf conversion program's stdin pipe was owned by
        // the writer which we now drop. The HtmlToPdf program should
        // therefore exit when it has finished processing its data.
        drop(writer);

        let exit_status = process
            .wait()
            .context(r#"Failed to wait for the "HtmlToPdf" conversion program to exit."#)?;

        if let Some(error_code) = exit_status.code() {
            if error_code != 0 {
                bail!(
                    r#"The "HtmlToPdf" conversion program exited with an error (code: {})."#,
                    error_code
                );
            }
        } else {
            bail!(r#"The "HtmlToPdf" conversion program exited with an error (no exit code)."#);
        };
        // The worker thread should finish now that stdout for "HtmlToPdf" has been closed.
        reader_thread.join().unwrap()
    }
}

struct DotNetHtmlSinkInner<'scope, W> {
    process: Child,
    writer: BufWriter<ChildStdin>,
    reader_thread: PdfScopedJoinHandle<'scope, Result<W>>,
}
pub struct DotNetHtmlSink<'scope, W>(DotNetHtmlSinkInner<'scope, W>);
impl<'scope, W> DotNetHtmlSink<'scope, W> {
    #[inline]
    fn writer(&mut self) -> &mut BufWriter<ChildStdin> {
        &mut self.0.writer
    }
}
impl<'scope, W> Write for DotNetHtmlSink<'scope, W> {
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
