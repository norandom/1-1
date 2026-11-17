//! Successful TCP payload I/O, below TLS. Includes TLS/negotiation overhead,
//! excludes TCP/IP headers, retransmissions and any separate authentication sockets.
use serde::{Deserialize, Serialize};
use std::io;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

#[derive(Debug, Default)]
pub struct NetworkCounters {
    rx: AtomicU64,
    tx: AtomicU64,
}

impl NetworkCounters {
    pub fn totals(&self) -> (u64, u64) {
        (
            self.rx.load(Ordering::Relaxed),
            self.tx.load(Ordering::Relaxed),
        )
    }
}

pub struct MeasuredStream<S> {
    inner: S,
    counters: Arc<NetworkCounters>,
}

impl<S> MeasuredStream<S> {
    pub fn new(inner: S, counters: Arc<NetworkCounters>) -> Self {
        Self { inner, counters }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for MeasuredStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            this.counters
                .rx
                .fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for MeasuredStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &result {
            this.counters.tx.fetch_add(*n as u64, Ordering::Relaxed);
        }
        result
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(n)) = &result {
            this.counters.tx.fetch_add(*n as u64, Ordering::Relaxed);
        }
        result
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMetrics {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_bytes_per_second: f64,
    pub tx_bytes_per_second: f64,
}

pub struct NetworkSampler {
    counters: Arc<NetworkCounters>,
    previous: (u64, u64),
    at: Instant,
}

impl NetworkSampler {
    pub fn new(counters: Arc<NetworkCounters>, at: Instant) -> Self {
        Self {
            previous: counters.totals(),
            counters,
            at,
        }
    }
    pub fn sample(&mut self, now: Instant) -> NetworkMetrics {
        let (rx, tx) = self.counters.totals();
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        let result = NetworkMetrics {
            rx_bytes: rx,
            tx_bytes: tx,
            rx_bytes_per_second: if elapsed > 0.0 {
                rx.saturating_sub(self.previous.0) as f64 / elapsed
            } else {
                0.0
            },
            tx_bytes_per_second: if elapsed > 0.0 {
                tx.saturating_sub(self.previous.1) as f64 / elapsed
            } else {
                0.0
            },
        };
        // Do not lose deltas when asked twice at the same instant.
        if elapsed > 0.0 {
            self.previous = (rx, tx);
            self.at = now;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn network_counts_actual_partial_io_and_keeps_sessions_isolated() {
        let counters = Arc::new(NetworkCounters::default());
        let other = NetworkCounters::default();
        let (stream, mut peer) = tokio::io::duplex(4);
        let mut stream = MeasuredStream::new(stream, counters.clone());
        assert_eq!(stream.write(b"abcdef").await.unwrap(), 4);
        assert_eq!(counters.totals(), (0, 4));
        peer.write_all(b"abc").await.unwrap();
        let mut buf = [0; 8];
        assert_eq!(stream.read(&mut buf).await.unwrap(), 3);
        assert_eq!(counters.totals(), (3, 4));
        assert_eq!(other.totals(), (0, 0));
        drop(peer);
        assert_eq!(stream.read(&mut buf).await.unwrap(), 0);
        assert!(stream.write(b"fail").await.is_err());
        assert_eq!(counters.totals(), (3, 4));
    }

    #[tokio::test]
    async fn network_pending_and_vectored_io_count_only_completed_bytes() {
        let counters = Arc::new(NetworkCounters::default());
        let (stream, mut peer) = tokio::io::duplex(3);
        let mut stream = MeasuredStream::new(stream, counters.clone());
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut data = [0; 10];
        let mut buf = ReadBuf::new(&mut data);
        assert!(
            Pin::new(&mut stream)
                .poll_read(&mut cx, &mut buf)
                .is_pending()
        );
        assert_eq!(
            stream
                .write_vectored(&[io::IoSlice::new(b"ab"), io::IoSlice::new(b"cd")])
                .await
                .unwrap(),
            3
        );
        assert!(Pin::new(&mut stream).poll_write(&mut cx, b"e").is_pending());
        assert_eq!(counters.totals(), (0, 3));
        peer.write_all(b"xy").await.unwrap();
        buf.put_slice(b"old");
        assert!(matches!(
            Pin::new(&mut stream).poll_read(&mut cx, &mut buf),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(counters.totals(), (2, 3));
    }

    #[test]
    fn network_read_error_leaves_counter_and_error_unchanged() {
        struct FailedRead;
        impl AsyncRead for FailedRead {
            fn poll_read(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                _: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")))
            }
        }
        let counters = Arc::new(NetworkCounters::default());
        let mut stream = MeasuredStream::new(FailedRead, counters.clone());
        let mut storage = [0; 16];
        let mut buf = ReadBuf::new(&mut storage);
        let mut cx = Context::from_waker(std::task::Waker::noop());
        match Pin::new(&mut stream).poll_read(&mut cx, &mut buf) {
            Poll::Ready(Err(e)) => assert_eq!(e.kind(), io::ErrorKind::ConnectionReset),
            other => panic!("read error must pass through: {other:?}"),
        }
        assert_eq!(counters.totals(), (0, 0));
    }

    #[test]
    fn network_sampling_uses_elapsed_time_idle_zero_and_new_connection_reset() {
        let counters = Arc::new(NetworkCounters::default());
        counters.rx.store(100, Ordering::Relaxed); // handshake counted in totals, not burst rate
        let now = Instant::now();
        let mut sampler = NetworkSampler::new(counters.clone(), now);
        counters.rx.fetch_add(300, Ordering::Relaxed);
        counters.tx.store(50, Ordering::Relaxed);
        assert_eq!(sampler.sample(now).rx_bytes_per_second, 0.0);
        let sample = sampler.sample(now + Duration::from_secs(2));
        assert_eq!(
            sample,
            NetworkMetrics {
                rx_bytes: 400,
                tx_bytes: 50,
                rx_bytes_per_second: 150.0,
                tx_bytes_per_second: 25.0
            }
        );
        assert_eq!(
            sampler
                .sample(now + Duration::from_secs(3))
                .rx_bytes_per_second,
            0.0
        );
        let mut reconnect = NetworkSampler::new(Arc::new(NetworkCounters::default()), now);
        assert_eq!(
            reconnect.sample(now + Duration::from_secs(1)),
            NetworkMetrics::default()
        );
    }
}
