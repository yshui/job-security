use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) struct OptionIo<T>(Option<T>);

impl<T> Default for OptionIo<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> From<Option<T>> for OptionIo<T> {
    fn from(inner: Option<T>) -> Self {
        Self(inner)
    }
}

impl<T> OptionIo<T> {
    fn project(self: Pin<&mut Self>) -> Pin<&mut Option<T>> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

impl<T: AsyncRead> AsyncRead for OptionIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let inner = self.project().as_pin_mut();
        match inner {
            Some(inner) => inner.poll_read(cx, buf),
            None => Poll::Pending,
        }
    }
}

impl<T: AsyncWrite> AsyncWrite for OptionIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let inner = self.project().as_pin_mut();
        match inner {
            Some(inner) => inner.poll_write(cx, buf),
            None => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let inner = self.project().as_pin_mut();
        match inner {
            Some(inner) => inner.poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let inner = self.project().as_pin_mut();
        match inner {
            Some(inner) => inner.poll_shutdown(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        let inner = self.project().as_pin_mut();
        match inner {
            Some(inner) => inner.poll_write_vectored(cx, bufs),
            None => Poll::Pending,
        }
    }
}
