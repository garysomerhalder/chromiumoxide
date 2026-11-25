//! WebSocket transport (`--remote-debugging-port`, `connect_over_cdp`).

use async_tungstenite::tungstenite::Message;
use async_tungstenite::tungstenite::protocol::WebSocketConfig;
use futures::{SinkExt, StreamExt};

use crate::domain::error::{Error, Result};
use crate::ports::Transport;

/// Connects to a DevTools WebSocket URL.
pub async fn connect(url: &str) -> Result<Transport> {
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    let (ws, _) = async_tungstenite::tokio::connect_async_with_config(url, Some(config))
        .await
        .map_err(|e| Error::Transport(format!("cannot connect to {url}: {e}")))?;
    Ok(wrap(ws))
}

/// A DevTools WebSocket over an already open stream (an adb
/// `localabstract:` socket), upgraded at `url`'s path.
pub async fn over_stream(stream: tokio::net::TcpStream, url: &str) -> Result<Transport> {
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    let (ws, _) = async_tungstenite::tokio::client_async_with_config(url, stream, Some(config))
        .await
        .map_err(|e| Error::Transport(format!("cannot open the DevTools socket at {url}: {e}")))?;
    Ok(wrap(ws))
}

/// Text frames in and out, until the socket closes.
pub(crate) fn wrap<S>(ws: async_tungstenite::WebSocketStream<S>) -> Transport
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + Send + 'static,
{
    let (sink, stream) = ws.split();
    let sink = sink
        .sink_map_err(std::io::Error::other)
        .with(|text: String| async move { Ok::<_, std::io::Error>(Message::text(text)) });
    let stream = stream
        .take_while(|m| futures::future::ready(!matches!(m, Ok(Message::Close(_)) | Err(_))))
        .filter_map(|m| async move {
            match m {
                Ok(Message::Text(t)) => Some(Ok(t.as_str().to_owned())),
                Ok(Message::Binary(b)) => {
                    Some(String::from_utf8(b.to_vec()).map_err(std::io::Error::other))
                }
                _ => None,
            }
        });
    Transport {
        sink: Box::pin(sink),
        stream: stream.boxed(),
    }
}
