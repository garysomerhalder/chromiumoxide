//! Transport port: a bidirectional channel of raw CDP JSON messages.

use std::pin::Pin;

use futures::{Sink, Stream};

/// Outgoing half.
pub type MessageSink = Pin<Box<dyn Sink<String, Error = std::io::Error> + Send>>;
/// Incoming half.
pub type MessageStream = Pin<Box<dyn Stream<Item = std::io::Result<String>> + Send>>;

/// A connected transport (pipe, WebSocket, in-memory).
pub struct Transport {
    pub sink: MessageSink,
    pub stream: MessageStream,
}
