#![warn(clippy::all)]

use eyre::{bail, ContextCompat, WrapErr};
use html_to_pdf::WriteBuilder;
use std::{
    error::Error as StdError,
    fmt,
    io::{self, Read, Write},
};

macro_rules! is_supported {
    ($( $token:tt )*) => {
        #[cfg(windows)]
        $( $token )*
    };
}
macro_rules! has_link {
    ($( $token:tt )*) => {
        #[cfg(feature = "should_link")]
        $( $token )*
    };
}
macro_rules! no_link {
    ($( $token:tt )*) => {
        #[cfg(not(feature = "should_link"))]
        $( $token )*
    };
}
macro_rules! has_dll {
    ($( $token:tt )*) => {
        #[cfg(feature = "should_include_dll")]
        $( $token )*
    };
}

macro_rules! wk_html_library_version {
    () => {
        "v0.12.3"
    };
}
pub const WK_HTML_LIBRARY_VERSION: &str = wk_html_library_version!();

// Check that the library version number is same as in the link crate.
#[cfg(any(feature = "should_link", feature = "should_include_dll"))]
const _: () = {
    if WK_HTML_LIBRARY_VERSION.len() != wkhtml_link::WK_HTML_LIBRARY_VERSION.len() {
        panic!("Incorrect WK_HTML_LIBRARY_VERSION")
    }
    let mut i = 0;
    while i < WK_HTML_LIBRARY_VERSION.len() {
        if WK_HTML_LIBRARY_VERSION.as_bytes()[i]
            != wkhtml_link::WK_HTML_LIBRARY_VERSION.as_bytes()[i]
        {
            panic!(concat!("Incorrect WK_HTML_LIBRARY_VERSION, the linked version was not: ", wk_html_library_version!()));
        }
        i += 1;
    }
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotSupportedError;
impl fmt::Display for NotSupportedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#""wkhtmltopdf" is not supported for this platform."#)
    }
}
impl StdError for NotSupportedError {}

/// `true` if we should prefer providing a buffer (via `convert_html_str_to_pdf`)
/// over a reader (via `convert_html_to_pdf`).
pub const PREFER_BUFFER_OVER_READER: bool = {
    const fn prefer() -> bool {
        is_supported!({
            has_link!({
                // More efficient:
                return true;
            });
        });
        // We are reading from a child process's stdout anyway, so might as well stream:
        false
    }
    prefer()
};

/// Convert HTML to PDF. Takes a reader and a writer. If you already have a string then use the [`convert_html_str_to_pdf`] function instead.
pub fn convert_html_to_pdf<R, W>(mut html_reader: R, mut writer: W) -> eyre::Result<()>
where
    R: Read,
    W: WriteBuilder + Send,
{
    is_supported!({
        /// This will have 0 size if the program is compiled with a link.
        static WK_HTML_RUNNER: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/wkhtml_runner.exe"));
        has_link!({
            let mut html = String::with_capacity(2024);
            html_reader.read_to_string(&mut html)?;

            convert_html_str_to_pdf(html, writer)?;
        });
        no_link!({
            use std::borrow::Cow;
            use std::fs;
            use std::process::{Command, Stdio};

            if WK_HTML_RUNNER.is_empty() {
                return Err(NotSupportedError.into());
            }

            has_dll! {{
                // Should include dll file, so if it isn't there then the platform isn't supported.
                use wkhtml_link::WK_HTML_TO_PDF_DLL;

                if WK_HTML_TO_PDF_DLL.is_empty() {
                    return Err(NotSupportedError.into());
                }
            }}

            let tmp_dir = tempfile::Builder::new()
                .prefix(&format!("wkhtml-{}", WK_HTML_LIBRARY_VERSION))
                .tempdir()?;

            // Write runner executable:
            let exe_path = tmp_dir.path().join("wkhtml_runner.exe");

            fs::File::create(&exe_path)
                .and_then(|mut file| io::copy(&mut &WK_HTML_RUNNER[..], &mut file))
                .context("Failed to create \"wkhtml_runner.exe\".")?;

            // Write needed dynamic library:
            has_dll! {{
                use wkhtml_link::WK_HTML_TO_PDF_DLL;

                let dll_path = tmp_dir.path().join("wkhtmltox.dll");

                fs::File::create(dll_path).and_then(|mut file| {
                    io::copy(&mut &WK_HTML_TO_PDF_DLL[..], &mut file)
                }).context("Failed to create \"wkhtmltox.dll\".")?;
            }}

            // Spawn child process:
            let mut process = Command::new(exe_path);
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
                .context("Failed to start \"wkhtml_runner.exe\"")?;
            // Redirect child process stdout to writer:
            let mut stdout = process
                .stdout
                .take()
                .context("Failed to open stdout for \"wkhtml_runner.exe\".")?;

            crossbeam::scope(|s| -> eyre::Result<_> {
                let redirect_thread = s.spawn(move |_| -> eyre::Result<_> {
                    Ok(io::copy(&mut stdout, &mut writer.get_writer()?)?)
                });

                // Write to child process stdin:
                let mut stdin = process
                    .stdin
                    .take()
                    .context("Failed to open stdin for \"wkhtml_runner.exe\".")?;
                io::copy(&mut html_reader, &mut stdin)
                    .context("Failed to write html data to stdin for \"wkhtml_runner.exe\".")?;
                // Close stdin:
                drop(stdin);
                // Wait for child process to exit:
                let status = process
                    .wait()
                    .context("Failed to wait for \"wkhtml_runner.exe\" to exit.")?;
                if !status.success() {
                    bail!(
                        "\"wkhtml_runner.exe\" exited with an error{}.",
                        if let Some(code) = status.code() {
                            Cow::from(format!(" (code: {})", code))
                        } else {
                            "".into()
                        }
                    );
                }
                redirect_thread
                    .join()
                    .expect(r#"Thread reading from stdin of "wkhtml_runner.exe" panicked"#)
                    .context(r#"Failed to read pdf data from stdout of "wkhtml_runner.exe"."#)?;

                Ok(())
            })
            .unwrap()?;

            tmp_dir
                .close()
                .context("failed to delete temporary folder for wkhtml files")?;
        });
        return Ok(());
    });
    #[allow(unreachable_code)]
    {
        Err(NotSupportedError.into())
    }
}

/// Convert HTML to PDF. Takes a string slice and a writer.
///
/// This version is more efficient when linking directly to wkhtml.
pub fn convert_html_str_to_pdf<R, W>(html: R, writer: W) -> eyre::Result<()>
where
    R: AsRef<str>,
    W: WriteBuilder + Send,
{
    is_supported!({
        has_link!({
            let mut writer = writer;
            let writer = writer.get_writer()?;
            wkhtml_link::convert_html_to_pdf(html, writer)?;
        });
        no_link!({
            let html = html.as_ref();
            convert_html_to_pdf(html.as_bytes(), writer)?;
        });
        return Ok(());
    });
    #[allow(unreachable_code)]
    {
        Err(NotSupportedError.into())
    }
}

mod converter {
    use super::*;

    /// Use WKHtmlToPdf to convert HTML to a PDF.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct WkHtmlPdfConverter;

    // TODO: implement an option to run WKHtml as a child process even if it is
    // linked.
    //
    // /// Shell out to the "wkhtmltopdf" executable. If this is `false` we will
    // /// attempt to link to the "wkhtmltopdf" library instead.
    // shelled: bool,

    impl<'scope, W> html_to_pdf::HtmlToPdfConverter<'scope, W> for WkHtmlPdfConverter
    where
        W: WriteBuilder + Send + 'scope,
    {
        type HtmlSink = HtmlSink<'scope, W>;

        type Error = eyre::Error;

        fn start(
            self,
            _scope: html_to_pdf::PdfScope<'scope, '_>,
            _output: W,
        ) -> Result<Self::HtmlSink, Self::Error> {
            is_supported!({
                let mut output = _output;
                let state = if PREFER_BUFFER_OVER_READER {
                    HtmlSinkState::Wkhtml {
                        output,
                        buffer: Vec::new(),
                    }
                } else {
                    HtmlSinkState::Streaming(html_to_pdf::WriteStream::stream(
                        _scope,
                        move |html| {
                            convert_html_to_pdf::<_, &mut W>(html, &mut output)
                                .context(r#"Failed to convert HTML to PDF using "WKHtmlToPdf""#)?;
                            Ok(output)
                        },
                    ))
                };
                return Ok(HtmlSink(Some(state)));
            });
            #[allow(unreachable_code)]
            {
                Err(NotSupportedError.into())
            }
        }
    }
    impl<'scope, W> html_to_pdf::HtmlSink<W, eyre::Error> for HtmlSink<'scope, W>
    where
        W: WriteBuilder + Send + 'scope,
    {
        fn complete(mut self) -> Result<W, eyre::Error> {
            self._complete().map(Option::unwrap)
        }
    }

    enum HtmlSinkState<'scope, W> {
        /// When "WKHtmlToPdf" is linked to directly it needs a string slice to work
        /// with which means that we can't stream data to it.
        Wkhtml { output: W, buffer: Vec<u8> },
        /// We shell out to another program and so we can stream the data to it.
        Streaming(html_to_pdf::WriteStream<'scope, eyre::Result<W>>),
    }
    pub struct HtmlSink<'scope, W>(Option<HtmlSinkState<'scope, W>>)
    where
        W: WriteBuilder + Send + 'scope;

    impl<'scope, W> HtmlSink<'scope, W>
    where
        W: WriteBuilder + Send + 'scope,
    {
        /// This can be called via `complete` or via the Drop implementation. The
        /// `Option` is guaranteed to be `Some` if this method isn't called in the
        /// `Drop` implementation.
        fn _complete(&mut self) -> eyre::Result<Option<W>> {
            if let Some(state) = self.0.take() {
                Ok(Some(match state {
                    HtmlSinkState::Wkhtml { mut output, buffer } => {
                        convert_html_str_to_pdf::<_, &mut W>(
                            String::from_utf8_lossy(&buffer),
                            &mut output,
                        )
                        .context(r#"Failed to convert HTML to PDF using "WKHtmlToPdf""#)?;
                        output
                    }
                    HtmlSinkState::Streaming(mut writer) => {
                        writer
                            .flush()
                            .context("Failed to flush written HTML data to the PDF converter.")?;
                        // Wait for the thread to stop writing PDF data and return the
                        // PDF sink:
                        writer.join().unwrap()?
                    }
                }))
            } else {
                // Already completed:
                Ok(None)
            }
        }
    }
    /// Ensure the PDF conversion happens even if the [`HtmlSink`] is dropped
    /// without calling [`html_to_pdf::HtmlToPdfConverter::complete`].
    impl<'scope, W> Drop for HtmlSink<'scope, W>
    where
        W: WriteBuilder + Send + 'scope,
    {
        fn drop(&mut self) {
            // This might have been called previously but that is handled by the method:
            self._complete().ok();
        }
    }

    /// Forward writing from the HtmlSink to the inner sink.
    macro_rules! get_writer {
        ($this: ident, $name:ident => $($token:tt)*) => {
            match $this.0.as_mut().unwrap() {
                HtmlSinkState::Wkhtml { buffer: $name, .. } => { $($token)* },
                HtmlSinkState::Streaming($name) => { $($token)* },
            }
        };
    }
    impl<'scope, W> Write for HtmlSink<'scope, W>
    where
        W: WriteBuilder + Send + 'scope,
    {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            get_writer!(self, writer => writer.write(buf))
        }
        fn flush(&mut self) -> io::Result<()> {
            get_writer!(self, writer => writer.flush())
        }
        fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
            get_writer!(self, writer => writer.write_vectored(bufs))
        }
        fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
            get_writer!(self, writer => writer.write_all(buf))
        }
        fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
            get_writer!(self, writer => writer.write_fmt(fmt))
        }
    }
}
#[doc(inline)]
pub use converter::*;
