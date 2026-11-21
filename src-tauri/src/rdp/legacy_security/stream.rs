//! The socket as the RDP library sees it under legacy security: whole PDUs
//! pass through the [`Transcoder`] in both directions.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::{Transcoder, pdu_length};

/// Encrypted bytes waiting for the socket before writes are held back.
const WRITE_BACKLOG: usize = 256 * 1024;
const READ_CHUNK: usize = 16 * 1024;

pub struct LegacyStream<S> {
    inner: S,
    pub(super) transcoder: Transcoder,
    /// From the socket, not yet a whole PDU.
    received: Vec<u8>,
    /// Decrypted PDUs the library has not read yet, from `readable_at`.
    readable: Vec<u8>,
    readable_at: usize,
    /// From the library, not yet a whole PDU.
    written: Vec<u8>,
    /// Encrypted PDUs the socket has not taken yet, from `sendable_at`.
    sendable: Vec<u8>,
    sendable_at: usize,
}

impl<S> LegacyStream<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            transcoder: Transcoder::default(),
            received: Vec::new(),
            readable: Vec::new(),
            readable_at: 0,
            written: Vec::new(),
            sendable: Vec::new(),
            sendable_at: 0,
        }
    }
}

fn protocol(error: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("legacy RDP security: {error}"),
    )
}

/// Moves every whole PDU at the start of `from` through `transcode`.
fn drain_pdus(
    from: &mut Vec<u8>,
    to: &mut Vec<u8>,
    mut transcode: impl FnMut(&[u8], &mut Vec<u8>) -> Result<(), String>,
) -> io::Result<()> {
    let mut at = 0;
    while let Some(length) = pdu_length(&from[at..]).map_err(protocol)? {
        let Some(pdu) = from.get(at..at + length) else {
            break;
        };
        transcode(pdu, to).map_err(protocol)?;
        at += length;
    }
    from.drain(..at);
    Ok(())
}

impl<S: AsyncRead + Unpin> AsyncRead for LegacyStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.readable_at < this.readable.len() {
                let pending = &this.readable[this.readable_at..];
                let count = pending.len().min(buf.remaining());
                buf.put_slice(&pending[..count]);
                this.readable_at += count;
                return Poll::Ready(Ok(()));
            }
            this.readable.clear();
            this.readable_at = 0;

            let mut chunk = [0u8; READ_CHUNK];
            let mut chunk = ReadBuf::new(&mut chunk);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut chunk))?;
            if chunk.filled().is_empty() {
                return Poll::Ready(Ok(())); // The server closed the connection.
            }
            this.received.extend_from_slice(chunk.filled());
            let transcoder = &mut this.transcoder;
            drain_pdus(&mut this.received, &mut this.readable, |pdu, out| {
                transcoder.incoming(pdu, out)
            })?;
        }
    }
}

impl<S: AsyncWrite + Unpin> LegacyStream<S> {
    /// Hands encrypted bytes to the socket until it takes no more.
    fn poll_send(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.sendable_at < self.sendable.len() {
            let pending = &self.sendable[self.sendable_at..];
            let count = ready!(Pin::new(&mut self.inner).poll_write(cx, pending))?;
            if count == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.sendable_at += count;
        }
        self.sendable.clear();
        self.sendable_at = 0;
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for LegacyStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // Bytes are accepted whole and sent as the socket allows; a socket
        // that stays behind holds further writes back.
        if this.poll_send(cx)?.is_pending()
            && this.sendable.len() - this.sendable_at > WRITE_BACKLOG
        {
            return Poll::Pending;
        }
        this.written.extend_from_slice(buf);
        let transcoder = &mut this.transcoder;
        drain_pdus(&mut this.written, &mut this.sendable, |pdu, out| {
            transcoder.outgoing(pdu, out)
        })?;
        let _ = this.poll_send(cx)?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_send(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_send(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}
