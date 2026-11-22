//! Video encoding port: JPEG frames in, a video file out.

use std::io;
use std::path::Path;

use futures::future::BoxFuture;

/// Starts encoders (ffmpeg in production).
pub trait VideoEncoder: Send + Sync {
    /// Starts writing a `width` x `height` video at `fps` to `output`.
    fn start(
        &self,
        output: &Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> BoxFuture<'static, io::Result<Box<dyn VideoSink>>>;
}

/// A running encoder: frames are written at the video's frame rate.
pub trait VideoSink: Send {
    fn write_frame<'a>(&'a mut self, jpeg: &'a [u8]) -> BoxFuture<'a, io::Result<()>>;
    /// Ends the input and waits for the file to be complete.
    fn finish(self: Box<Self>) -> BoxFuture<'static, io::Result<()>>;
}
