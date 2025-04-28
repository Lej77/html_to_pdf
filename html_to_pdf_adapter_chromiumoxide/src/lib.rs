//! Provides an adapter that implements `html_to_pdf`'s trait using [`chromiumoxide`].
//!
//! [`chromiumoxide`]: https://crates.io/crates/chromiumoxide

#[cfg(all(not(feature = "tokio-runtime"), not(feature = "async-std-runtime")))]
std::compile_error!("The `html_to_pdf_adapter_chromiumoxide` crate requires either the `tokio-runtime` or `async-std-runtime` feature to be enabled.");

use bytes::Bytes;
pub use chromiumoxide::{cdp::browser_protocol::page::PrintToPdfParams, error::CdpError as Error};
use chromiumoxide::{Browser, BrowserConfig};
use html_to_pdf::{HtmlSink, HtmlToPdfConverter, WriteBuilder};
use hyper::{Method, StatusCode};
use std::{
    convert::Infallible,
    future::Future,
    io::{self, Write},
    marker::PhantomData,
    net::SocketAddr,
};

#[cfg(feature = "async-std-runtime")]
use async_std::{net::TcpListener, stream::StreamExt as _};
#[cfg(all(feature = "tokio-runtime", not(feature = "async-std-runtime")))]
use {futures_util::StreamExt as _, tokio::net::TcpListener};

// TODO: we might need this to support hyper for async-std
#[allow(dead_code)]
fn spawn<F>(fut: F) -> impl Future<Output = F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    #[cfg(feature = "async-std-runtime")]
    {
        async_std::task::spawn(fut)
    }
    #[cfg(all(feature = "tokio-runtime", not(feature = "async-std-runtime")))]
    {
        let handle = tokio::task::spawn(fut);
        async move { handle.await.unwrap() }
    }
}
fn block_on<F>(fut: F) -> F::Output
where
    F: Future,
{
    #[cfg(feature = "async-std-runtime")]
    {
        async_std::task::block_on(fut)
    }
    #[cfg(all(feature = "tokio-runtime", not(feature = "async-std-runtime")))]
    {
        tokio::runtime::Runtime::new()
            .expect("Failed to create tokio runtime")
            .block_on(fut)
    }
}

async fn simple_http_server<T>(listener: TcpListener, content: Bytes) -> Result<T, Error> {
    use http_body_util::{Either, Empty, Full};
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto;

    async fn handle_request(
        req: Request<impl hyper::body::Body>,
        content: Bytes,
    ) -> Result<Response<Either<Full<Bytes>, Empty<Bytes>>>, Infallible> {
        Ok(if Method::GET != req.method() {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Either::Right(Empty::new()))
                .unwrap()
        } else {
            Response::builder()
                .header("Content-Type", "text/html")
                .body(Either::Left(Full::new(content.clone())))
                .unwrap()
        })
    }

    loop {
        // When an incoming TCP connection is received grab a TCP stream for
        // client<->server communication.
        let (tcp, _) = listener.accept().await?;
        // Use an adapter to access something implementing `tokio::io` traits as if they implement
        // `hyper::rt` IO traits.
        let io = TokioIo::new(tcp);

        // Spin up a new task in Tokio so we can continue to listen for new TCP connection on the
        // current task without waiting for the processing of the HTTP1 connection we just received
        // to finish
        let content = content.clone();
        tokio::task::spawn(async move {
            // Handle the connection from the client using HTTP1 and pass any
            // HTTP requests received on that connection to the `hello` function
            if let Err(_err) = auto::Builder::new(TokioExecutor::new())
                // .timer(TokioTimer::new())
                .serve_connection(
                    io,
                    service_fn({
                        move |req| {
                            let content = content.clone();
                            handle_request(req, content)
                        }
                    }),
                )
                .await
            {
                // TODO: handle error
            }
        });
    }
}

pub fn html_to_pdf(html: Bytes, options: PrintToPdfParams) -> Result<Vec<u8>, Error> {
    block_on(async {
        // Inspired by example at:
        // https://github.com/mattsse/chromiumoxide/blob/bd62ee35df3fad70d0b72e25faeed793bdab597c/examples/pdf.rs
        let (mut browser, mut handler) =
            Browser::launch(BrowserConfig::builder().build().map_err(Error::msg)?).await?;

        // port 0 to bind to any available port
        let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
        let listener = TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();

        // Close server when chromiumoxide is done...
        let res: Result<(Infallible, Infallible), Result<Vec<u8>, Error>> =
            futures_util::future::try_join(
                // Serve HTML on localhost:
                async { simple_http_server(listener, html).await.map_err(Err) },
                async {
                    // Exit early if the background tasks fails:
                    let res = futures_util::future::try_join(
                        // Run background tasks:
                        async move {
                            loop {
                                match handler.next().await {
                                    Some(Ok(())) => {}
                                    Some(Err(e)) => break Err(e),
                                    None => break Ok(()),
                                }
                            }
                        },
                        // Load data from local HTTP server and convert it into a PDF:
                        async move {
                            let page = browser
                                .new_page(format!("http://localhost:{}/", port))
                                .await?;

                            // save the page as pdf
                            let data = page.pdf(options).await?;

                            browser.close().await?;

                            Ok(data)
                        },
                    )
                    .await;
                    Err::<Infallible, _>(res.map(|((), data)| data))
                },
            )
            .await;
        match res {
            Ok((v, _)) => match v {},
            Err(res) => res,
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct ChromiumoxideConverter {
    pub pdf_options: PrintToPdfParams,
}

impl<'scope, W> HtmlToPdfConverter<'scope, W> for ChromiumoxideConverter
where
    W: WriteBuilder + Send + 'scope,
{
    type HtmlSink = ChromiumoxideHtmlSink<'scope, W>;
    type Error = Error;

    fn start(
        self,
        _scope: html_to_pdf::PdfScope<'scope, '_>,
        output: W,
    ) -> Result<Self::HtmlSink, Self::Error> {
        Ok(ChromiumoxideHtmlSink {
            buffer: Vec::new(),
            writer: output,
            options: self,
            _scope: PhantomData,
        })
    }
}
impl<'scope, W> HtmlSink<W, Error> for ChromiumoxideHtmlSink<'scope, W>
where
    W: WriteBuilder + Send + 'scope,
{
    fn complete(mut self) -> Result<W, Error> {
        let mut writer = self.writer.get_writer()?;
        const UTF8_BOM: &[u8] = "\u{feff}".as_bytes();
        if self.buffer.starts_with(UTF8_BOM) {
            drop(self.buffer.drain(..UTF8_BOM.len()));
        }

        let data = html_to_pdf(self.buffer.into(), self.options.pdf_options)?;
        writer.write_all(data.as_slice())?;

        drop(writer);
        Ok(self.writer)
    }
}

pub struct ChromiumoxideHtmlSink<'scope, W> {
    buffer: Vec<u8>,
    writer: W,
    options: ChromiumoxideConverter,
    _scope: PhantomData<&'scope ()>,
}
impl<'scope, W> Write for ChromiumoxideHtmlSink<'scope, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
