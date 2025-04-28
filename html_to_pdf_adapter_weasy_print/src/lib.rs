//! [WeasyPrint: WeasyPrint converts web documents (HTML with CSS, SVG, …) to
//! PDF.](https://github.com/Kozea/WeasyPrint).
//!
//! This crate includes the required files for WeasyPrint into the binary and
//! extracts them into a folder at runtime.
//!
//! There are installation instructions for WeasyPrint at:
//! <https://doc.courtbouillon.org/weasyprint/stable/first_steps.html#installation>
//!
//! For Windows the best options seems to be the executable from the GitHub
//! releases or [the linked .Net wrapper] that has packaged WeasyPrint into a
//! zip file [`weasyprint-python-binary.zip`].
//!
//! [the linked .Net wrapper]:
//!     https://doc.courtbouillon.org/weasyprint/stable/first_steps.html#net-wrapper
//! [`weasyprint-python-binary.zip`]:
//!     https://github.com/balbarak/WeasyPrint-netcore/blob/776ec2ddbaa6ab8a785219bb55b8327795a29b41/src/Balbarak.WeasyPrint/Resources/weasyprint-python-binary.zip
#![warn(clippy::all)]
