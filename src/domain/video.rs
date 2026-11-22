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

/// The encoder ffmpeg is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    /// `libvpx-vp9` (Playwright 1.64+).
    Vp9,
    /// `vp8`, for ffmpeg builds without VP9 (Playwright 1.63's).
    Vp8,
}

/// `"name" must be a positive number`, Playwright's check of `fps`.
pub fn validate_fps(name: &str, fps: Option<u32>) -> Result<(), String> {
    unimplemented!()
}

/// The CSS of the action title (`style.title`, after the deprecated `fontSize`).
pub fn action_title_style(font_size: Option<u32>, title: Option<&str>) -> Option<String> {
    unimplemented!()
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
        let mut t = FrameTimeline::new(0, 25);
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

    /// Playwright 1.64's `fps` option.
    #[test]
    fn timeline_follows_the_frame_rate() {
        let mut t = FrameTimeline::new(0, 50);
        t.push(0);
        assert_eq!(t.frames_before(100), 5);
        t.push(100);
        assert_eq!(t.padding_at_close(100), 50);
    }

    /// Playwright 1.64 encodes VP9 (`FfmpegVideoRecorder`), scaling bitrate
    /// and threads with the pixel rate.
    #[test]
    fn ffmpeg_arguments_vp9_like_playwright() {
        let args = ffmpeg_args(VideoCodec::Vp9, 800, 450, 25, "/v/a.webm");
        assert_eq!(
            args.join(" "),
            "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 \
             -analyzeduration 0 -c:v mjpeg -framerate 25 -i pipe:0 -y -an -r 25 -c:v libvpx-vp9 \
             -qmin 0 -qmax 50 -crf 8 -deadline realtime -cpu-used 8 -lag-in-frames 0 -b:v 1000k \
             -threads 1 -tile-columns 0 -row-mt 1 -vf pad=800:450:0:0:gray,crop=800:450:0:0 /v/a.webm"
        );
        // 1280x720 at 60 fps: 6.144 times the 800x450@25 pixel rate.
        let args = ffmpeg_args(VideoCodec::Vp9, 1280, 720, 60, "/v/b.webm").join(" ");
        assert!(
            args.contains("-r 60 -c:v libvpx-vp9")
                && args.contains("-b:v 6144k -threads 2 -tile-columns 1"),
            "{args}"
        );
    }

    /// An ffmpeg without libvpx-vp9 (Playwright 1.63's build) records VP8 as before.
    #[test]
    fn ffmpeg_arguments_vp8_fallback() {
        let args = ffmpeg_args(VideoCodec::Vp8, 800, 450, 25, "/v/a.webm");
        assert_eq!(
            args.join(" "),
            "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 \
             -analyzeduration 0 -c:v mjpeg -framerate 25 -i pipe:0 -y -an -r 25 -c:v vp8 \
             -qmin 0 -qmax 50 -crf 8 -deadline realtime -speed 8 -b:v 1M -threads 1 \
             -vf pad=800:450:0:0:gray,crop=800:450:0:0 /v/a.webm"
        );
    }

    #[test]
    fn fps_must_be_positive() {
        assert_eq!(validate_fps("recordVideo.fps", Some(30)), Ok(()));
        assert_eq!(validate_fps("fps", None), Ok(()));
        assert_eq!(
            validate_fps("recordVideo.fps", Some(0)),
            Err("\"recordVideo.fps\" must be a positive number, got 0".to_owned())
        );
    }

    /// Playwright 1.64 `showActions`: the deprecated `fontSize` becomes a
    /// `font-size` declaration in front of `style.title`.
    #[test]
    fn action_title_style_merges_font_size() {
        assert_eq!(
            action_title_style(Some(16), Some("color: red")),
            Some("font-size: 16px; color: red".to_owned())
        );
        assert_eq!(
            action_title_style(Some(16), None),
            Some("font-size: 16px; ".to_owned())
        );
        assert_eq!(
            action_title_style(None, Some("color: red")),
            Some("color: red".to_owned())
        );
        assert_eq!(action_title_style(None, None), None);
    }
}
