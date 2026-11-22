//! `--remote-debugging-pipe` transport: NUL-terminated JSON messages over a
//! pair of byte streams (the browser reads fd 3 and writes fd 4).

use tokio::io::{AsyncRead, AsyncWrite};

use crate::ports::Transport;

/// Wraps a reader (browser → us) and a writer (us → browser).
pub fn pipe_transport<R, W>(_reader: R, _writer: W) -> Transport
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn messages_are_split_on_nul_across_chunks() {
        let (mut browser_out, our_in) = tokio::io::duplex(64);
        let (our_out, _browser_in) = tokio::io::duplex(64);
        let mut t = pipe_transport(our_in, our_out);
        browser_out.write_all(b"{\"a\":1}\0{\"b\"").await.unwrap();
        browser_out.write_all(b":2}\0").await.unwrap();
        assert_eq!(t.stream.next().await.unwrap().unwrap(), "{\"a\":1}");
        assert_eq!(t.stream.next().await.unwrap().unwrap(), "{\"b\":2}");
        drop(browser_out);
        assert!(t.stream.next().await.is_none());
    }

    #[tokio::test]
    async fn large_messages_survive() {
        let (mut browser_out, our_in) = tokio::io::duplex(1024);
        let (our_out, _browser_in) = tokio::io::duplex(64);
        let mut t = pipe_transport(our_in, our_out);
        let big = "x".repeat(300_000);
        let writer = tokio::spawn(async move {
            browser_out.write_all(big.as_bytes()).await.unwrap();
            browser_out.write_all(b"\0").await.unwrap();
        });
        assert_eq!(t.stream.next().await.unwrap().unwrap().len(), 300_000);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn sent_messages_are_nul_terminated() {
        let (_browser_out, our_in) = tokio::io::duplex(64);
        let (our_out, mut browser_in) = tokio::io::duplex(64);
        let mut t = pipe_transport(our_in, our_out);
        t.sink.send("{\"id\":1}".to_owned()).await.unwrap();
        t.sink.send("{\"id\":2}".to_owned()).await.unwrap();
        let mut buf = vec![0u8; 18];
        browser_in.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"{\"id\":1}\0{\"id\":2}\0");
    }
}
