use std::io::{self, Read};

fn main() {
    let mut html = String::with_capacity(2048);
    io::stdin().lock().read_to_string(&mut html)
        .expect("Failed to read HTML from stdin.");

    let stdout = std::io::stdout();
    wkhtml_link::convert_html_to_pdf(html, &mut io::BufWriter::new(stdout.lock()))
        .expect("Failed to convert HTML to PDF.");
}