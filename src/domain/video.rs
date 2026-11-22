//! Video recording rules (Playwright's `recordVideo`).

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
