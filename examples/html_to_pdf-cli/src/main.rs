use clap::{Parser, Subcommand};
use color_eyre::Section;
use eyre::{bail, Result, WrapErr};
use html_to_pdf::{HtmlSink, HtmlToPdfConverter, PdfScope, WriteBuilder, WriteBuilderSimple};

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::thread;

/// Convert a HTML file to a PDF file.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long, conflicts_with = "input", help_heading = "INPUT")]
    stdin: bool,
    #[arg(
        short,
        long,
        value_name = "INPUT_PATH",
        help_heading = "INPUT",
        required_unless_present = "stdin"
    )]
    input: Option<PathBuf>,

    #[arg(long, conflicts_with = "output", help_heading = "OUTPUT")]
    stdout: bool,
    #[arg(
        short,
        long,
        value_name = "OUTPUT_PATH",
        help_heading = "OUTPUT",
        required_unless_present = "stdout"
    )]
    output: Option<PathBuf>,
    /// Overwrite the output file.
    #[arg(
        long,
        visible_alias = "ow",
        requires = "output",
        help_heading = "OUTPUT"
    )]
    overwrite: bool,

    /// Specify where extra files will be stored. Defaults to the user's global
    /// temp folder.
    ///
    /// Otherwise the files can also be stored next to the executable and it is
    /// also possible to delete the extracted files right before the program
    /// exits.
    #[arg(
        long,
        value_enum,
        default_value_t = ExtraFileLocation::GlobalPersist
    )]
    extract_at: ExtraFileLocation,

    #[command(subcommand)]
    command: PdfConversionMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum ExtraFileLocation {
    LocalPersist,
    LocalTemp,
    GlobalPersist,
    GlobalTemp,
}

/// Configuration for different HTML to PDF converters.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum PdfConversionMethod {
    /// Use a small C# program that calls into the iText .Net Framework library, see:
    ///
    /// https://www.nuget.org/packages/itextsharp.xmlworker
    DotNetItextFramework {
        /// The program supports different modes since the C# library it uses
        /// has different ways to handle the conversion.
        #[command(subcommand)]
        mode: DotNetFrameworkItextMode,
    },
    /// Use the iText .Net library via a small C# program. This is slower than
    /// the older .Net Framework iText library but has more accurate results
    /// (for example some japanese characters are only correctly shown with this
    /// option).
    ///
    /// - No PDF Table of Contents.
    DotNetItext,
    /// Use "wkhtmltopdf" to handle the conversion.
    Wkhtml {
        /// Shell out to the "wkhtmltopdf" executable. If this is `false` we will
        /// attempt to link to the "wkhtmltopdf" library instead.
        ///
        /// NOTE: not implemented yet.
        #[arg(long)]
        shelled: bool,
    },
    /// Use the Rust library "pdf-min" to handle the conversion.
    ///
    /// This library is very minimal and doesn't support many HTML tags, for
    /// example link tags (<a>) doesn't seem to be supported.
    PdfMin,
    /// Use the Rust library "chromiumoxide" to control a headless Chrome
    /// browser with the DevTools Protocol in order to load HTML and "print" a
    /// PDF.
    ///
    /// Note: it's important to specify "<meta charset="UTF-8">" in the HTML
    /// file's head section; otherwise it might not handle all characters
    /// correctly.
    Chromiumoxide,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub enum DotNetFrameworkItextMode {
    /// A C# HTML to PDF converter using its older legacy implementation.
    ///
    /// - Links will not be colored blue but they can still be clicked.
    /// - No PDF Table of Contents.
    PdfLegacy {
        /// This mode doesn't support page break info from the HTML file. This
        /// argument allows specifying a custom string that should be
        /// interpreted as a page break.
        #[arg(long)]
        custom_page_break: Option<OsString>,
    },
    /// A C# HTML to PDF converter using its older XML implementation in its
    /// simpler mode.
    ///
    /// - More than twice as slow when <a> tags are inside a <div>.
    /// - Supports PDF Table of Contents for easier navigation.
    PdfXmlSimple,
    /// A C# HTML to PDF converter using its older XML implementation in
    /// advanced mode.
    ///
    /// - More than twice as slow when <a> tags are inside a <div>.
    /// - No PDF Table of Contents.
    PdfXmlAdv,
}
#[cfg(feature = "dotnet_framework_conversion")]
impl DotNetFrameworkItextMode {
    fn mode(&self) -> html_to_pdf_adapter_dotnet_framework_itext::DotNetFrameworkPdfConverterMode {
        use html_to_pdf_adapter_dotnet_framework_itext::DotNetFrameworkPdfConverterMode as Mode;
        use DotNetFrameworkItextMode::*;

        match self {
            PdfLegacy { .. } => Mode::ObsoleteHTMLParser,
            PdfXmlSimple => Mode::XMLWorkerSimple,
            PdfXmlAdv => Mode::XMLWorkerAdvanced,
        }
    }
    fn into_converter(
        self,
    ) -> html_to_pdf_adapter_dotnet_framework_itext::DotNetFrameworkPdfConverter {
        html_to_pdf_adapter_dotnet_framework_itext::DotNetFrameworkPdfConverter {
            mode: self.mode(),
            custom_page_break: if let DotNetFrameworkItextMode::PdfLegacy { custom_page_break } =
                self
            {
                custom_page_break
            } else {
                None
            },
            #[cfg(feature = "dotnet_framework_conversion_include_exe")]
            extract_included_exe_at: Some(std::env::temp_dir().join("HtmlToPdf_Framework")),
            #[cfg(not(feature = "dotnet_framework_conversion_include_exe"))]
            extract_included_exe_at: None,
        }
    }
}

impl<'scope, W> HtmlToPdfConverter<'scope, W> for PdfConversionMethod
where
    W: WriteBuilder + Send + 'scope,
{
    type HtmlSink = Box<dyn HtmlSink<W, Self::Error> + 'scope>;
    type Error = eyre::Error;

    fn start(self, scope: PdfScope<'scope, '_>, output: W) -> Result<Self::HtmlSink> {
        Ok(match self {
            PdfConversionMethod::DotNetItextFramework { mode } => {
                #[cfg(feature = "dotnet_framework_conversion")]
                {
                    Box::new(mode.into_converter().start(scope, output)?)
                }
                #[cfg(not(feature = "dotnet_framework_conversion"))]
                {
                    bail!(
                        r#"The C# .Net Framework PDF conversion program wasn't included when this program was created."#
                    );
                }
            }
            PdfConversionMethod::DotNetItext => {
                #[cfg(feature = "dotnet_conversion")]
                {
                    Box::new(
                        html_to_pdf_adapter_dotnet_itext::DotNetPdfConverter {
                            #[cfg(feature = "dotnet_conversion_include_exe")]
                            extract_included_exe_at: Some(std::env::temp_dir().join("HtmlToPdf")),
                            #[cfg(not(feature = "dotnet_conversion_include_exe"))]
                            extract_included_exe_at: None,
                        }
                        .start(scope, output)?,
                    )
                }
                #[cfg(not(feature = "dotnet_conversion"))]
                {
                    bail!(
                        r#"The C# .Net PDF conversion program wasn't included when this program was created."#
                    );
                }
            }
            PdfConversionMethod::Wkhtml { shelled } => {
                if shelled {
                    bail!("Shell out to wkhtml for PDF conversion is not supported yet.");
                }
                #[cfg(feature = "wk_html_to_pdf")]
                {
                    Box::new(html_to_pdf_adapter_wkhtml::WkHtmlPdfConverter.start(scope, output)?)
                }
                #[cfg(not(feature = "wk_html_to_pdf"))]
                {
                    bail!(
                        r#"The WKHtmlToPdf PDF conversion program wasn't included when this program was created."#
                    );
                }
            }
            PdfConversionMethod::PdfMin => {
                #[cfg(not(feature = "pdf_min_conversion"))]
                {
                    bail!(
                        r#"The "pdf-min" Rust library wasn't built when this program was created."#
                    );
                }
                #[cfg(feature = "pdf_min_conversion")]
                {
                    Box::new(
                        html_to_pdf_adapter_pdf_min::PdfMinConverter
                            .start(scope, output)
                            .map_err(|e| eyre::eyre!(e))?
                            .map_completion_err(|e| eyre::eyre!(e)),
                    )
                }
            }
            PdfConversionMethod::Chromiumoxide => {
                #[cfg(not(feature = "chromiumoxide_conversion"))]
                {
                    bail!(
                        r#"The "chromiumoxide" Rust library wasn't built when this program was created."#
                    );
                }
                #[cfg(feature = "chromiumoxide_conversion")]
                {
                    Box::new(
                        html_to_pdf_adapter_chromiumoxide::ChromiumoxideConverter {
                            pdf_options: Default::default(),
                        }
                        .start(scope, output)
                        .map_err(|e| eyre::eyre!(e))?
                        .map_completion_err(|e| eyre::eyre!(e)),
                    )
                }
            }
        })
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    color_eyre::install()?;

    if cli.extract_at != ExtraFileLocation::GlobalPersist {
        bail!(
            "Locations of extra files can't be configured yet \
            so don't use the --extract-at option"
        )
    }

    let mut input: Box<dyn Read> = if let Some(input) = cli.input {
        eprintln!("Reading input from file at: {}", input.display());
        Box::new(BufReader::new(File::open(&input).with_context(|| {
            format!("Failed to open input file at: {}", input.display())
        })?))
    } else {
        eprintln!("Reading input from stdin");
        Box::new(io::stdin())
    };

    let mut output: Box<dyn Write + Send> = if let Some(output) = cli.output {
        eprintln!("Writing output to file at: {}", output.display());
        Box::new(BufWriter::new({
            let result = OpenOptions::new()
                .truncate(true)
                .write(true)
                .create(true)
                .create_new(!cli.overwrite)
                .open(&output);

            let should_overwrite =
                matches!(&result, Err(e) if e.kind() == io::ErrorKind::AlreadyExists);

            let result = result
                .with_context(|| format!("Failed to create output file at: {}", output.display()));
            if should_overwrite && !cli.overwrite {
                result.suggestion(
                    "pass the --overwrite flag if the output file should be overwritten",
                )?
            } else {
                result?
            }
        }))
    } else {
        eprintln!("Writing output to stdout");
        Box::new(io::stdout())
    };

    let pdf_method = cli.command;
    thread::scope(|s| -> Result<()> {
        eprintln!("Opened input and output, starting PDF converter...");

        let mut html_sink = pdf_method
            .start(PdfScope::scoped(s), WriteBuilderSimple(&mut output))
            .context("Failed to start PDF converter")?;

        eprintln!("Started PDF converter, reading HTML from input...");

        io::copy(&mut input, &mut html_sink)
            .context("Failed to write HTML data to PDF converter")?;

        drop(input);
        eprintln!("Read all of the input file, waiting until PDF has been written to output...");

        html_sink.complete().context("PDF converter failed")?;

        Ok(())
    })?;

    eprintln!("Successfully converted HTML to PDF");

    Ok(())
}
