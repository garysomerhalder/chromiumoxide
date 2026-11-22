//! ffmpeg video encoder (Playwright's ffmpeg build when installed).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::io::AsyncWriteExt;

use crate::domain::video::{VideoCodec, ffmpeg_args};
use crate::ports::video::{VideoEncoder, VideoSink};

/// Finds ffmpeg: `FFMPEG_PATH`, Playwright's browser folders, then `PATH`.
pub fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FFMPEG_PATH").map(PathBuf::from)
        && p.exists()
    {
        return Some(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        roots.push(PathBuf::from(p));
    }
    roots.push(PathBuf::from("/opt/pw-browsers"));
    if let Some(h) = &home {
        roots.push(h.join(".cache/ms-playwright"));
        roots.push(h.join("Library/Caches/ms-playwright"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("ms-playwright"));
    }
    for root in roots {
        let Ok(dir) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<PathBuf> = dir
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("ffmpeg"))
            .flat_map(|e| {
                ["ffmpeg-linux", "ffmpeg-mac", "ffmpeg-win64.exe"].map(|n| e.path().join(n))
            })
            .filter(|p| p.exists())
            .collect();
        found.sort();
        if let Some(p) = found.pop() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| {
            d.join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            })
        })
        .find(|p| p.exists())
}

/// VP9 when this ffmpeg has `libvpx-vp9` (Playwright 1.64's builds), else
/// VP8 (Playwright 1.63's). Probed once per executable.
fn codec_of(ffmpeg: &std::path::Path) -> VideoCodec {
    static PROBED: std::sync::Mutex<Vec<(PathBuf, VideoCodec)>> = std::sync::Mutex::new(Vec::new());
    if let Some((_, c)) = PROBED.lock().unwrap().iter().find(|(p, _)| p == ffmpeg) {
        return *c;
    }
    let vp9 = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("libvpx-vp9"));
    let codec = if vp9 {
        VideoCodec::Vp9
    } else {
        VideoCodec::Vp8
    };
    PROBED.lock().unwrap().push((ffmpeg.to_owned(), codec));
    codec
}

/// Encodes with an ffmpeg process.
#[derive(Debug, Default, Clone, Copy)]
pub struct FfmpegEncoder;

struct FfmpegSink {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
}

impl VideoEncoder for FfmpegEncoder {
    fn start(
        &self,
        output: &Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> BoxFuture<'static, io::Result<Box<dyn VideoSink>>> {
        let output = output.to_owned();
        async move {
            let ffmpeg = find_ffmpeg().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "ffmpeg not found (install it with `npx playwright install ffmpeg` or set FFMPEG_PATH)",
                )
            })?;
            if let Some(dir) = output.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut child = tokio::process::Command::new(&ffmpeg)
                .args(ffmpeg_args(
                    codec_of(&ffmpeg),
                    width,
                    height,
                    fps,
                    &output.to_string_lossy(),
                ))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()?;
            let stdin = child.stdin.take();
            Ok(Box::new(FfmpegSink { child, stdin }) as Box<dyn VideoSink>)
        }
        .boxed()
    }
}

impl VideoSink for FfmpegSink {
    fn write_frame<'a>(&'a mut self, jpeg: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        async move {
            match self.stdin.as_mut() {
                Some(stdin) => stdin.write_all(jpeg).await,
                None => Ok(()),
            }
        }
        .boxed()
    }

    fn finish(mut self: Box<Self>) -> BoxFuture<'static, io::Result<()>> {
        async move {
            if let Some(mut stdin) = self.stdin.take() {
                let _ = stdin.shutdown().await;
            }
            self.child.wait().await?;
            Ok(())
        }
        .boxed()
    }
}
