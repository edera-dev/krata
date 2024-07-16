use std::future::Future;
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::rt::ReadBufCursor;
use hyper_util::rt::TokioIo;
use pin_project_lite::pin_project;
use tokio::io::AsyncWrite;
use tokio::net::UnixStream;
use tonic::transport::Uri;
use tower::Service;

pin_project! {
    #[derive(Debug)]
    pub struct HyperUnixStream {
        #[pin]
        pub stream: UnixStream,
    }
}

impl hyper::rt::Read for HyperUnixStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<Result<(), Error>> {
        let mut tokio = TokioIo::new(self.project().stream);
        Pin::new(&mut tokio).poll_read(cx, buf)
    }
}

impl hyper::rt::Write for HyperUnixStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        self.project().stream.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_shutdown(cx)
    }
}

pub struct HyperUnixConnector;

impl Service<Uri> for HyperUnixConnector {
    type Response = HyperUnixStream;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn call(&mut self, req: Uri) -> Self::Future {
        let fut = async move {
            let path = req.path().to_string();
            let stream = UnixStream::connect(path).await?;
            Ok(HyperUnixStream { stream })
        };

        Box::pin(fut)
    }

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
