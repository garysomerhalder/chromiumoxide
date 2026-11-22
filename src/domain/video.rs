//! Video recording rules (Playwright's `recordVideo`).

use std::path::PathBuf;

use crate::domain::emulation::Viewport;

/// `recordVideo` context option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordVideo {
    /// Directory for the `.webm` files (one per page).
    pub dir: PathBuf,
    /// Frame size; by default the viewport scaled to fit 800x800.
    pub size: Option<(u32, u32)>,
}

impl RecordVideo {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        RecordVideo {
            dir: dir.into(),
            size: None,
        }
    }
}

/// Playwright's video size: explicit, else the viewport scaled down to fit
/// 800x800 (800x600 without a viewport); both even, as vp8 needs.
pub fn video_size(viewport: Option<Viewport>, explicit: Option<(u32, u32)>) -> (u32, u32) {
    let (w, h) = match (explicit, viewport) {
        (Some(s), _) => s,
        (None, Some(v)) => {
            let scale = (800.0 / v.width.max(v.height) as f64).min(1.0);
            (
                (v.width as f64 * scale).floor() as u32,
                (v.height as f64 * scale).floor() as u32,
            )
        }
        (None, None) => (800, 600),
    };
    (w & !1, h & !1)
}

/// Frames per second of recorded videos.
pub const FPS: u64 = 25;
const SLOT_MS: u64 = 1000 / FPS;

/// Maps screencast frames (which arrive only when the page changes) onto a
/// constant 25 fps timeline: each frame is written when the next one arrives,
/// repeated for the slots it covered.
#[derive(Debug, Clone)]
pub struct FrameTimeline {
    start_ms: u64,
    written: u64,
    has_frame: bool,
}

impl FrameTimeline {
    pub fn new(start_ms: u64) -> Self {
        FrameTimeline {
            start_ms,
            written: 0,
            has_frame: false,
        }
    }

    /// How many copies of the previous frame to write before a frame at `t_ms`.
    pub fn frames_before(&mut self, t_ms: u64) -> u64 {
        if !self.has_frame {
            return 0;
        }
        let slot = t_ms.saturating_sub(self.start_ms) / SLOT_MS;
        let n = slot.saturating_sub(self.written);
        self.written += n;
        n
    }

    /// Whether any frame arrived yet.
    pub fn has_frame(&self) -> bool {
        self.has_frame
    }

    /// Records that a frame arrived.
    pub fn push(&mut self, _t_ms: u64) {
        self.has_frame = true;
    }

    /// Copies of the last frame to write when recording stops: up to now, and
    /// at least one second (like Playwright).
    pub fn padding_at_close(&mut self, now_ms: u64) -> u64 {
        let due = self.frames_before(now_ms);
        let n = due.max(FPS);
        self.written += n - due;
        n
    }
}

/// ffmpeg arguments: MJPEG frames on stdin to a VP8 `.webm`.
pub fn ffmpeg_args(width: u32, height: u32, output: &str) -> Vec<String> {
    let mut args: Vec<String> = format!(
        "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 \
         -analyzeduration 0 -c:v mjpeg -framerate {FPS} -i pipe:0 -y -an -r {FPS} -c:v vp8 \
         -qmin 0 -qmax 50 -crf 8 -deadline realtime -speed 8 -b:v 1M -threads 1 \
         -vf pad={width}:{height}:0:0:gray,crop={width}:{height}:0:0"
    )
    .split_whitespace()
    .map(str::to_owned)
    .collect();
    args.push(output.to_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::emulation::Viewport;

    #[test]
    fn default_size_fits_800_like_playwright() {
        assert_eq!(video_size(Some(Viewport::new(1280, 720)), None), (800, 450));
        assert_eq!(video_size(Some(Viewport::new(390, 844)), None), (368, 800));
        assert_eq!(video_size(Some(Viewport::new(640, 480)), None), (640, 480));
        assert_eq!(video_size(None, None), (800, 600));
        // vp8 needs even sizes.
        assert_eq!(video_size(None, Some((301, 201))), (300, 200));
    }

    #[test]
    fn frames_fill_a_25_fps_timeline() {
        let mut t = FrameTimeline::new(0);
        assert_eq!(t.frames_before(0), 0);
        // A frame at 100 ms: the previous frame covers 0..100 (2.5 slots -> 2).
        t.push(0);
        assert_eq!(t.frames_before(100), 2);
        t.push(100);
        assert_eq!(t.frames_before(130), 1);
        t.push(130);
        // Closing pads at least a second of the last frame.
        assert_eq!(t.padding_at_close(140), 25);
    }

    #[test]
    fn ffmpeg_arguments() {
        let args = ffmpeg_args(800, 450, "/v/a.webm");
        assert_eq!(
            args.join(" "),
            "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 \
             -analyzeduration 0 -c:v mjpeg -framerate 25 -i pipe:0 -y -an -r 25 -c:v vp8 \
             -qmin 0 -qmax 50 -crf 8 -deadline realtime -speed 8 -b:v 1M -threads 1 \
             -vf pad=800:450:0:0:gray,crop=800:450:0:0 /v/a.webm"
        );
    }
}
