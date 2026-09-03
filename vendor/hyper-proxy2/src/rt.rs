use std::future::Future;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, BufMut};
use futures_util::ready;
use hyper::rt;
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct ReadBuf<'a, R: ?Sized, B: ?Sized> {
        reader: &'a mut R,
        buf: &'a mut B,
        #[pin]
        _pin: PhantomPinned,
    }
}

impl<R, B> Future for ReadBuf<'_, R, B>
where
    R: rt::Read + Unpin + ?Sized,
    B: BufMut + ?Sized,
{
    type Output = std::io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        use hyper::rt::{Read as _, ReadBuf};
        use std::mem::MaybeUninit;

        let me = self.project();

        if !me.buf.has_remaining_mut() {
            return Poll::Ready(Ok(0));
        }

        let n = {
            let dst = me.buf.chunk_mut();
            let dst = unsafe { &mut *(dst as *mut _ as *mut [MaybeUninit<u8>]) };
            let mut buf = ReadBuf::uninit(dst);
            let ptr = buf.filled().as_ptr();
            ready!(Pin::new(me.reader).poll_read(cx, buf.unfilled())?);

            // Ensure the pointer does not change from under us
            assert_eq!(ptr, buf.filled().as_ptr());
            buf.filled().len()
        };

        // Safety: This is guaranteed to be the number of initialized (and read)
        // bytes due to the invariants provided by `ReadBuf::filled`.
        unsafe {
            me.buf.advance_mut(n);
        }

        Poll::Ready(Ok(n))
    }
}

pub(crate) fn read_buf<'a, R, B>(reader: &'a mut R, buf: &'a mut B) -> ReadBuf<'a, R, B>
where
    R: rt::Read + Unpin + ?Sized,
    B: BufMut + ?Sized,
{
    ReadBuf {
        reader,
        buf,
        _pin: PhantomPinned,
    }
}

pin_project! {
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct WriteBuf<'a, W, B> {
        writer: &'a mut W,
        buf: &'a mut B,
        #[pin]
        _pin: PhantomPinned,
    }
}

impl<W, B> Future for WriteBuf<'_, W, B>
where
    W: rt::Write + Unpin,
    B: Buf,
{
    type Output = std::io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        use rt::Write as _;

        let me = self.project();

        if !me.buf.has_remaining() {
            return Poll::Ready(Ok(0));
        }

        let n = ready!(Pin::new(me.writer).poll_write(cx, me.buf.chunk()))?;
        me.buf.advance(n);
        Poll::Ready(Ok(n))
    }
}

pub(crate) fn write_buf<'a, W, B>(writer: &'a mut W, buf: &'a mut B) -> WriteBuf<'a, W, B>
where
    W: rt::Write + Unpin,
    B: Buf,
{
    WriteBuf {
        writer,
        buf,
        _pin: PhantomPinned,
    }
}

pub(crate) trait ReadExt: rt::Read {
    fn read_buf<'a, B>(&'a mut self, buf: &'a mut B) -> ReadBuf<'a, Self, B>
    where
        Self: Unpin,
        B: BufMut + ?Sized,
    {
        read_buf(self, buf)
    }
}

impl<T> ReadExt for T where T: rt::Read {}

pub(crate) trait WriteExt: rt::Write {
    fn write_buf<'a, B>(&'a mut self, src: &'a mut B) -> WriteBuf<'a, Self, B>
    where
        Self: Sized + Unpin,
        B: Buf,
    {
        write_buf(self, src)
    }
}

impl<T> WriteExt for T where T: rt::Write {}
