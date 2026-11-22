//! `--remote-debugging-pipe` transport: NUL-terminated JSON messages over a
//! pair of byte streams (the browser reads fd 3 and writes fd 4).

use std::io;

use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::ports::Transport;

/// Wraps a reader (browser → us) and a writer (us → browser).
pub fn pipe_transport<R, W>(reader: R, writer: W) -> Transport
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let stream = futures::stream::unfold(
        BufReader::with_capacity(1 << 16, reader),
        |mut r| async move {
            let mut buf = Vec::new();
            match r.read_until(0, &mut buf).await {
                Ok(0) => None,
                Ok(_) => {
                    if buf.last() == Some(&0) {
                        buf.pop();
                    } else {
                        // EOF in the middle of a message: the browser went away.
                        return None;
                    }
                    let text = String::from_utf8(buf)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
                    Some((text, r))
                }
                Err(e) => Some((Err(e), r)),
            }
        },
    );
    let sink = futures::sink::unfold(writer, |mut w, message: String| async move {
        w.write_all(message.as_bytes()).await?;
        w.write_all(b"\0").await?;
        w.flush().await?;
        Ok::<_, io::Error>(w)
    });
    Transport {
        sink: Box::pin(sink),
        stream: stream.boxed(),
    }
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
