//! Provides an interface for HTML to PDF conversions.

use std::{fmt, io::Write, marker::PhantomData};

mod thread_scope {
    //! A scope that can spawn either `'static` "owned" threads or limited
    //! "scoped" threads.

    use std::{
        any::Any,
        thread::{self, JoinHandle, ScopedJoinHandle},
    };

    enum PdfScopedJoinHandleState<'scope, T> {
        Static(
            JoinHandle<Box<dyn AsAny + Send + 'static>>,
            &'scope StaticThread<'scope>,
        ),
        Scoped(ScopedJoinHandle<'scope, T>),
    }
    /// A thread join handle that internally can be either [`JoinHandle`] or
    /// [`ScopedJoinHandle`].
    pub struct PdfScopedJoinHandle<'scope, T>(PdfScopedJoinHandleState<'scope, T>);
    impl<'scope, T: 'scope> PdfScopedJoinHandle<'scope, T> {
        pub fn join(self) -> thread::Result<T> {
            match self.0 {
                PdfScopedJoinHandleState::Static(v, dyn_static) => v.join().map(|v| {
                    let mut slot = DynDowncastSlot(None);
                    let downcast = (dyn_static.static_dyn_downcast)(&mut slot);
                    downcast.downcast(v.as_any());
                    slot.0
                        .expect("failed to downcast type returned from spawned thread")
                }),
                PdfScopedJoinHandleState::Scoped(v) => v.join(),
            }
        }
    }

    /// A trait that allows downcasts for a type `T` stored inside `Self` if we
    /// can prove that `T: 'static` using [`StaticThread`].
    trait DynDowncast {
        fn downcast(&mut self, obj: Box<dyn Any>)
        where
            Self: 'static;
    }
    struct DynDowncastSlot<T>(Option<T>);
    impl<T> DynDowncast for DynDowncastSlot<T> {
        fn downcast(&mut self, obj: Box<dyn Any>)
        where
            Self: 'static,
        {
            self.0 = obj.downcast::<T>().ok().map(|v| *v);
        }
    }

    /// A trait object that implements `Any` if `Self: 'static`.
    trait AsAny {
        fn as_any(self: Box<Self>) -> Box<dyn Any>
        where
            Self: 'static;
    }
    impl<T> AsAny for T {
        fn as_any(self: Box<Self>) -> Box<dyn Any>
        where
            Self: 'static,
        {
            self
        }
    }

    /// If there exists a trait object with the lifetime `'scope` then that lifetime
    /// is actually `'static`.
    ///
    /// This type uses that fact to allow spawning a thread using types that
    /// outlive the lifetime `'scope`.
    #[allow(clippy::type_complexity)]
    struct StaticThread<'scope> {
        spawn: fn(
            f: Box<dyn FnOnce() -> Box<dyn AsAny + Send + 'scope> + Send + 'scope>,
        ) -> JoinHandle<Box<dyn AsAny + Send + 'static>>,
        static_dyn_downcast: for<'a> fn(
            downcast: &'a mut (dyn DynDowncast + 'scope),
        ) -> &'a mut (dyn DynDowncast + 'static),
    }
    impl StaticThread<'static> {
        fn new() -> &'static Self {
            &Self {
                spawn: thread::spawn,
                static_dyn_downcast: |downcast| downcast,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum PdfScopeInner<'scope, 'env> {
        Static(&'scope StaticThread<'scope>),
        Scoped(&'scope thread::Scope<'scope, 'env>),
    }

    /// A scope for spawning threads. If constructed using [`PdfScope::scoped`]
    /// then it wraps [`thread::Scope`], otherwise it can be constructed using
    /// [`PdfScope::owned`] in which case it will spawn normal `'static` threads
    /// using [`thread::spawn`].
    #[derive(Clone, Copy)]
    pub struct PdfScope<'scope, 'env>(PdfScopeInner<'scope, 'env>);
    impl PdfScope<'static, 'static> {
        /// Create a scope that has no max lifetime, this means only `'static`
        /// data can be used by spawned threads.
        pub fn owned() -> Self {
            Self(PdfScopeInner::Static(StaticThread::new()))
        }
    }
    impl<'scope, 'env> PdfScope<'scope, 'env> {
        /// Create a scope that ensures all spawned threads are joined before
        /// the provide scope ends. Spawned threads can make use of data that
        /// outlives the `'scope` lifetime.
        pub fn scoped(scope: &'scope thread::Scope<'scope, 'env>) -> Self {
            Self(PdfScopeInner::Scoped(scope))
        }
        /// Spawn a thread that might be limited to a scope created by
        /// [`thread::scoped`].
        pub fn spawn<F, T>(self, f: F) -> PdfScopedJoinHandle<'scope, T>
        where
            F: FnOnce() -> T + Send + 'scope,
            T: Send + 'scope,
        {
            PdfScopedJoinHandle(match self.0 {
                PdfScopeInner::Static(dyn_static) => PdfScopedJoinHandleState::Static(
                    (dyn_static.spawn)(Box::new(move || Box::new(f()))),
                    dyn_static,
                ),
                PdfScopeInner::Scoped(scope) => PdfScopedJoinHandleState::Scoped(scope.spawn(f)),
            })
        }
    }
}
pub use thread_scope::*;

mod write_builder {
    use std::io::{self, Write};

    mod sealed_lifetime {
        //! For more info see:
        //! <https://sabrinajewson.org/blog/the-better-alternative-to-lifetime-gats>
        pub trait Sealed: Sized {}
        pub struct Bounds<T>(T);
        impl<T> Sealed for Bounds<T> {}
    }
    use sealed_lifetime::{Bounds, Sealed};

    /// Supertrait for [`WriteBuilder`] that emulates lifetime GATs.
    pub trait WriteBuilderLifetime<'borrow, ImplicitBounds: Sealed = Bounds<&'borrow Self>> {
        type Writer: Write;
    }
    /// For writers that need to borrow state when used.
    ///
    /// Note: this trait could be simplified when GAT become stable.
    pub trait WriteBuilder: for<'borrow> WriteBuilderLifetime<'borrow> {
        fn get_writer(&mut self) -> io::Result<<Self as WriteBuilderLifetime<'_>>::Writer>;
    }
    impl<'a, W> WriteBuilderLifetime<'a> for &mut W
    where
        W: WriteBuilderLifetime<'a>,
    {
        type Writer = W::Writer;
    }
    impl<W> WriteBuilder for &mut W
    where
        W: WriteBuilder,
    {
        fn get_writer(&mut self) -> io::Result<<Self as WriteBuilderLifetime<'_>>::Writer> {
            <W as WriteBuilder>::get_writer(self)
        }
    }

    /// A write builder that wraps a normal writer.
    pub struct WriteBuilderSimple<W>(pub W);
    impl<'a, W> WriteBuilderLifetime<'a> for WriteBuilderSimple<W>
    where
        W: Write,
    {
        type Writer = &'a mut W;
    }
    impl<W> WriteBuilder for WriteBuilderSimple<W>
    where
        W: Write,
    {
        fn get_writer(&mut self) -> io::Result<<Self as WriteBuilderLifetime<'_>>::Writer> {
            Ok(&mut self.0)
        }
    }

    /// A write builder that constructs a builder via a closure.
    pub struct WriteBuilderFn<F>(F);
    impl WriteBuilderFn<()> {
        pub fn new<'a, F, W>(f: F) -> WriteBuilderFn<F>
        where
            F: FnMut() -> io::Result<W> + 'a,
            W: Write + 'a,
        {
            WriteBuilderFn(f)
        }
        pub fn new_infallible<'a, F, W>(
            mut f: F,
        ) -> WriteBuilderFn<impl FnMut() -> io::Result<W> + 'a>
        where
            F: FnMut() -> W + 'a,
            W: Write + 'a,
        {
            WriteBuilderFn(move || Ok(f()))
        }
    }
    impl<W, F> WriteBuilderLifetime<'_> for WriteBuilderFn<F>
    where
        F: FnMut() -> io::Result<W>,
        W: Write,
    {
        type Writer = W;
    }
    impl<W, F> WriteBuilder for WriteBuilderFn<F>
    where
        F: FnMut() -> io::Result<W>,
        W: Write,
    {
        fn get_writer(&mut self) -> io::Result<<Self as WriteBuilderLifetime<'_>>::Writer> {
            (self.0)()
        }
    }
}
pub use write_builder::*;

mod io_stream {
    //! Utility that is useful to implement a lot of converters.
    use std::{
        io::{self, BufRead, Read, Write},
        thread,
    };

    use crate::{PdfScope, PdfScopedJoinHandle};

    /// Reads data from another thread.
    pub struct ReadStream(pipe::PipeReader);
    impl BufRead for ReadStream {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.0.fill_buf()
        }

        fn consume(&mut self, amt: usize) {
            self.0.consume(amt)
        }
    }

    impl Read for ReadStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    /// Writes data that can be read from another thread.
    pub struct WriteStream<'scope, R> {
        /// A spawned thread that generates PDF data and writes it to a specified
        /// output sink.
        reader_thread: PdfScopedJoinHandle<'scope, R>,
        /// A pipe through which HTML data can be written so that the spawned thread
        /// can read it and use it to generate the PDF.
        writer: pipe::PipeWriter,
    }
    impl<'scope, R> WriteStream<'scope, R>
    where
        R: Send + 'scope,
    {
        /// Preform the PDF generation on a background thread.
        pub fn stream(
            scope: PdfScope<'scope, '_>,
            f: impl FnOnce(ReadStream) -> R + Send + 'scope,
        ) -> Self {
            let (reader, writer) = pipe::pipe();
            WriteStream {
                reader_thread: scope.spawn(move || f(ReadStream(reader))),
                writer,
            }
        }
    }
    impl<'scope, R> WriteStream<'scope, R>
    where
        R: 'scope,
    {
        /// Wait for the spawned thread to finish.
        pub fn join(self) -> thread::Result<R> {
            // Drop the writer first so that the background thread doesn't
            // deadlock trying to read more data:
            drop(self.writer);
            // Then wait for the background thread to finish:
            self.reader_thread.join()
        }
    }
    impl<R> Write for WriteStream<'_, R> {
        #[inline]
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writer.write(buf)
        }

        #[inline]
        fn flush(&mut self) -> io::Result<()> {
            self.writer.flush()
        }
    }
}
pub use io_stream::*;

/// Specifies a way to convert HTML to a PDF.
///
/// # Type parameters
///
/// - `W` is the sink that the PDF data should be written to.
/// - `'scope` is a lifetime that the writer mut outlive.
pub trait HtmlToPdfConverter<'scope, W>
where
    W: WriteBuilder + Send + 'scope,
{
    /// A handle to a PDF conversion tool that allows writing HTML data to it.
    ///
    /// Write HTML data into this sink and it will be used by the converter to
    /// generate the PDF data.
    type HtmlSink: HtmlSink<W, Self::Error>;
    /// Info about something that went wrong.
    type Error: fmt::Debug + fmt::Display;

    /// Start the HTML to PDF conversion. `output` provides a sink that the tool
    /// will write PDF data to. The HTML data should be written into the
    /// returned type.
    fn start(
        self,
        scope: PdfScope<'scope, '_>,
        output: W,
    ) -> Result<Self::HtmlSink, Self::Error>;
}

/// Automatically implemented for all [`HtmlSink`] types. Used by blanket
/// implementation for `Box<dyn HtmlSink>`.
///
/// For more info about this pattern, see: [Call consuming method for dyn trait
/// object? - help - The Rust Programming Language
/// Forum](https://users.rust-lang.org/t/call-consuming-method-for-dyn-trait-object/69596/7)
pub trait HtmlSinkBoxed<W, E>: Write {
    fn complete_boxed(self: Box<Self>) -> Result<W, E>;
}
impl<W, E, T> HtmlSinkBoxed<W, E> for T
where
    T: HtmlSink<W, E>,
{
    fn complete_boxed(self: Box<Self>) -> Result<W, E> {
        T::complete(*self)
    }
}

pub trait HtmlSink<W, E>: HtmlSinkBoxed<W, E> {
    /// Close the HTML sink and finish the PDF conversion. Call this to handle
    /// any PDF conversion errors. This will wait for the PDF conversion to
    /// finish and then also retrieve the sink that the converter wrote PDF data
    /// into.
    fn complete(self) -> Result<W, E>
    where
        Self: Sized;

    /// Wrap this sink in a sink that maps the error that happens when the
    /// [`HtmlSink::complete`] method is called.
    fn map_completion_err<E2, F>(self, f: F) -> HtmlSinkMappedError<Self, W, E, E2, F>
    where
        Self: Sized,
        F: FnOnce(E) -> E2,
    {
        HtmlSinkMappedError {
            inner: self,
            f,
            marker: PhantomData,
        }
    }

    /// Wrap this sink in a sink that maps the [`WriteBuilder`] that is returned
    /// when the [`HtmlSink::complete`] method is called.
    fn try_map_writer<W2, F>(self, f: F) -> HtmlSinkMappedError<Self, W, W2, E, F>
    where
        Self: Sized,
        F: FnOnce(W) -> Result<W2, E>,
    {
        HtmlSinkMappedError {
            inner: self,
            f,
            marker: PhantomData,
        }
    }
}
impl<W, E, T> HtmlSink<W, E> for Box<T>
where
    T: ?Sized + HtmlSinkBoxed<W, E>,
{
    fn complete(self) -> Result<W, E>
    where
        Self: Sized,
    {
        <T as HtmlSinkBoxed<W, E>>::complete_boxed(self)
    }
}

/// Used by [`HtmlSink::map_completion_err`] to map completion errors for html sinks.
pub struct HtmlSinkMappedError<S, W, E1, E2, F> {
    inner: S,
    f: F,
    /// Use all type parameters, but don't let them affect what auto traits we
    /// implement. `fn` is always `Send`.
    #[allow(clippy::type_complexity)]
    marker: PhantomData<fn() -> (W, E1, E2)>,
}
impl<S, W, E1, E2, F> HtmlSinkMappedError<S, W, E1, E2, F> {
    pub fn into_inner(self) -> S {
        self.inner
    }
}
impl<S, W, E1, E2, F> HtmlSink<W, E2> for HtmlSinkMappedError<S, W, E1, E2, F>
where
    S: HtmlSink<W, E1>,
    F: FnOnce(E1) -> E2,
{
    fn complete(self) -> Result<W, E2>
    where
        Self: Sized,
    {
        <S as HtmlSink<W, E1>>::complete(self.inner).map_err(self.f)
    }
}
impl<S, W, E1, E2, F> Write for HtmlSinkMappedError<S, W, E1, E2, F>
where
    S: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        <S as Write>::write(&mut self.inner, buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        <S as Write>::flush(&mut self.inner)
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        <S as Write>::write_vectored(&mut self.inner, bufs)
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        <S as Write>::write_all(&mut self.inner, buf)
    }

    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> std::io::Result<()> {
        <S as Write>::write_fmt(&mut self.inner, fmt)
    }
}

/// Used by [`HtmlSink::try_map_writer`] to map the writers for html sinks.
pub struct HtmlSinkMappedWriter<S, W1, W2, E, F> {
    inner: S,
    f: F,
    /// Use all type parameters, but don't let them affect what auto traits we
    /// implement. `fn` is always `Send`.
    #[allow(clippy::type_complexity)]
    marker: PhantomData<fn() -> (W1, W2, E)>,
}
impl<S, W1, W2, E, F> HtmlSinkMappedWriter<S, W1, W2, E, F> {
    pub fn into_inner(self) -> S {
        self.inner
    }
}
impl<S, W1, W2, E, F> HtmlSink<W2, E> for HtmlSinkMappedWriter<S, W1, W2, E, F>
where
    S: HtmlSink<W1, E>,
    F: FnOnce(W1) -> Result<W2, E>,
{
    fn complete(self) -> Result<W2, E>
    where
        Self: Sized,
    {
        <S as HtmlSink<W1, E>>::complete(self.inner).and_then(self.f)
    }
}
impl<S, W1, W2, E, F> Write for HtmlSinkMappedWriter<S, W1, W2, E, F>
where
    S: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        <S as Write>::write(&mut self.inner, buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        <S as Write>::flush(&mut self.inner)
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        <S as Write>::write_vectored(&mut self.inner, bufs)
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        <S as Write>::write_all(&mut self.inner, buf)
    }

    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> std::io::Result<()> {
        <S as Write>::write_fmt(&mut self.inner, fmt)
    }
}
