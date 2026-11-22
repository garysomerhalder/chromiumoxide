//! An in-memory transport: the other end is driven by test code.

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};

use crate::ports::Transport;

/// The browser side of an in-memory transport.
pub struct Peer {
    /// Messages the client sent.
    pub outgoing: mpsc::UnboundedReceiver<String>,
    /// Push messages to the client.
    pub incoming: mpsc::UnboundedSender<std::io::Result<String>>,
}

/// Creates a connected (transport, peer) pair.
pub fn pair() -> (Transport, Peer) {
    let (out_tx, out_rx) = mpsc::unbounded::<String>();
    let (in_tx, in_rx) = mpsc::unbounded::<std::io::Result<String>>();
    let sink = out_tx.sink_map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e));
    (
        Transport {
            sink: Box::pin(sink),
            stream: in_rx.boxed(),
        },
        Peer {
            outgoing: out_rx,
            incoming: in_tx,
        },
    )
}
