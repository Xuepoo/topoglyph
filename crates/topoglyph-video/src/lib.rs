//! Video-to-`.tglyph` animation conversion. Native-only (uses `ffmpeg` via
//! `vectomancy-video`, which can't target `wasm32`), so this crate is
//! intentionally *not* part of the wasm-facing dependency graph (see
//! `topoglyph-docs/TODO.md` 0.5.0 and the comment in the workspace
//! `Cargo.toml`).
//!
//! A video is treated as nothing more than a sequence of images: each
//! decoded frame is run through the exact same
//! smoothing -> grid-clipping -> glyph-matching pipeline the still-image CLI
//! path uses (`topoglyph_vectomancy::adapter`, `topoglyph_core::clipping`,
//! `topoglyph_core::matching`), then the resulting [`TextCanvas`] sequence
//! is handed to [`topoglyph_output::animation::TglyphAnimation`] for
//! delta-encoding into the final `.tglyph` text document.

use std::collections::BTreeMap;
use std::io::{Seek, Write};
use std::sync::{mpsc, Arc, Condvar, Mutex};

use image::DynamicImage;
use rayon::prelude::*;
use topoglyph_atlas::atlas::GlyphAtlas;
use topoglyph_core::canvas::TextCanvas;
use topoglyph_core::geometry::GridOptions;
use topoglyph_core::matching::{MatchOptions, MatchWeights};
use topoglyph_core::{clipping, matching};
use topoglyph_output::animation::{TglyphAnimation, TglyphError};
use topoglyph_output::stream::{AnimationFormat, StreamError, StreamingEncoder};

pub mod audio;
use topoglyph_vectomancy::adapter::{self, SmoothingOptions};

#[derive(Debug, thiserror::Error)]
pub enum VideoConvertError {
    #[error("failed to open video: {0}")]
    Decode(String),
    #[error("failed to encode a frame to PNG for the raster pipeline: {0}")]
    FrameEncode(String),
    #[error("failed to convert a decoded frame: {0}")]
    FrameConvert(String),
    #[error("no frames were decoded from the input")]
    NoFrames,
    #[error(transparent)]
    Animation(#[from] TglyphError),
    #[error("thread count must be at least 1")]
    InvalidThreadCount,
    #[error("failed to build render thread pool: {0}")]
    ThreadPool(#[from] rayon::ThreadPoolBuildError),
    #[error("video render pipeline disconnected")]
    PipelineDisconnected,
    #[error("video render worker panicked")]
    WorkerPanic,
    #[error(transparent)]
    Stream(#[from] StreamError),
}

/// Options controlling how each decoded video frame is turned into a
/// [`TextCanvas`], mirroring the still-image CLI's render options
/// (`topoglyph-cli render`'s `RenderArgs`).
#[derive(Debug, Clone)]
pub struct FrameRenderOptions {
    pub columns: usize,
    pub rows: Option<usize>,
    pub smoothing: SmoothingOptions,
    pub weights: MatchWeights,
    pub match_options: MatchOptions,
    pub sample_color: bool,
}

impl Default for FrameRenderOptions {
    fn default() -> Self {
        Self {
            columns: 120,
            rows: None,
            smoothing: SmoothingOptions::default(),
            weights: MatchWeights::default(),
            match_options: MatchOptions::default(),
            sample_color: false,
        }
    }
}

/// Renders one decoded frame (already an in-memory image, not yet a
/// `.tglyph` frame) into a [`TextCanvas`], reusing the exact same
/// skeleton-extraction -> smoothing -> clipping -> matching pipeline the
/// still-image CLI path uses. Frames are independent of each other in this
/// step (no temporal smoothing/interpolation), so callers can run this in
/// parallel across frames — see [`convert_frames`].
pub fn render_frame(
    image: &DynamicImage,
    atlas: &GlyphAtlas,
    options: &FrameRenderOptions,
) -> Result<TextCanvas, VideoConvertError> {
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| VideoConvertError::FrameEncode(e.to_string()))?;

    let scene = adapter::raster_to_smoothed_scene(&bytes, options.sample_color, &options.smoothing)
        .map_err(VideoConvertError::FrameConvert)?;

    let grid_opts = GridOptions {
        columns: options.columns,
        rows: options.rows,
        ..Default::default()
    };
    let (out_cols, out_rows, cell_descriptors) = clipping::process_scene(&scene, &grid_opts);

    let canvas = matching::match_scene_indexed(
        out_cols,
        out_rows,
        &cell_descriptors,
        &atlas.glyphs,
        Some(&atlas.index),
        &options.weights,
        &options.match_options,
    );

    Ok(canvas)
}

/// Renders every frame in `images` (in order) into a [`TextCanvas`]
/// sequence, in parallel (each frame's pipeline run is independent), then
/// assembles them into a delta-encoded [`TglyphAnimation`].
///
/// `fps` and `include_color` are passed straight through to
/// [`TglyphAnimation::encode`]; per `topoglyph-docs/TODO.md`, callers
/// should default `include_color` to `false` ("颜色默认关闭，由用户选择开
/// 启") and only pass `true` when the caller has explicitly opted in.
pub fn convert_frames(
    images: &[DynamicImage],
    atlas: &GlyphAtlas,
    options: &FrameRenderOptions,
    fps: f32,
    include_color: bool,
) -> Result<TglyphAnimation, VideoConvertError> {
    if images.is_empty() {
        return Err(VideoConvertError::NoFrames);
    }

    let canvases: Vec<TextCanvas> = images
        .par_iter()
        .map(|image| render_frame(image, atlas, options))
        .collect::<Result<_, _>>()?;

    TglyphAnimation::encode(&canvases, fps, include_color).map_err(VideoConvertError::Animation)
}

/// Decodes every frame of the video at `path` via `ffmpeg`
/// (`vectomancy_video`) into a `Vec<DynamicImage>`, then converts the
/// sequence to a `.tglyph` animation via [`convert_frames`].
///
/// `fps` should reflect the *output* frame rate the caller wants recorded
/// in the `.tglyph` header (players use it purely as timing metadata; this
/// function does not itself resample/drop frames to hit a target rate —
/// every decoded frame becomes one output frame).
pub fn convert_video_file(
    path: &std::path::Path,
    atlas: &GlyphAtlas,
    options: &FrameRenderOptions,
    fps: f32,
    include_color: bool,
) -> Result<TglyphAnimation, VideoConvertError> {
    let (receiver, join_handle) = vectomancy_video::decode_video_to_channel(path)
        .map_err(|e| VideoConvertError::Decode(e.to_string()))?;

    let mut images = Vec::new();
    for frame in receiver.iter() {
        let image = frame
            .to_image()
            .map_err(|e| VideoConvertError::FrameConvert(e.to_string()))?;
        images.push(image);
    }

    join_handle
        .join()
        .map_err(|_| VideoConvertError::Decode("decoder thread panicked".to_string()))?
        .map_err(|e| VideoConvertError::Decode(e.to_string()))?;

    convert_frames(&images, atlas, options, fps, include_color)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoConvertSummary {
    pub frame_count: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoOutputOptions {
    pub fps: f32,
    pub include_color: bool,
    pub threads: usize,
    pub format: AnimationFormat,
}

struct InFlightLimiter {
    available: Mutex<usize>,
    ready: Condvar,
}

impl InFlightLimiter {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            available: Mutex::new(limit),
            ready: Condvar::new(),
        })
    }

    fn acquire(self: &Arc<Self>) -> InFlightPermit {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *available == 0 {
            available = self
                .ready
                .wait(available)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *available -= 1;
        InFlightPermit {
            limiter: Arc::clone(self),
        }
    }
}

struct InFlightPermit {
    limiter: Arc<InFlightLimiter>,
}

impl Drop for InFlightPermit {
    fn drop(&mut self) {
        let mut available = self
            .limiter
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *available += 1;
        self.limiter.ready.notify_one();
    }
}

/// Converts a video directly into a seekable `.tglyph` writer without ever
/// materializing the whole video or animation in memory.
///
/// The decoder channel is bounded by `vectomancy-video`; this function adds a
/// second bound around rendered canvases. At most `threads` completed frames
/// can wait for an earlier frame before backpressure stops more rendering.
/// Ordered frames are written immediately and only the previous canvas is
/// retained for delta encoding.
pub fn convert_video_file_to_writer<W: Write + Seek>(
    path: &std::path::Path,
    atlas: &GlyphAtlas,
    render_options: &FrameRenderOptions,
    output_options: VideoOutputOptions,
    writer: W,
) -> Result<(VideoConvertSummary, W), VideoConvertError> {
    let VideoOutputOptions {
        fps,
        include_color,
        threads,
        format,
    } = output_options;
    if threads == 0 {
        return Err(VideoConvertError::InvalidThreadCount);
    }

    let (receiver, decoder_handle) = vectomancy_video::decode_video_to_channel(path)
        .map_err(|error| VideoConvertError::Decode(error.to_string()))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let limiter = InFlightLimiter::new(threads);
    let dictionary = atlas
        .glyphs
        .iter()
        .map(|glyph| glyph.token.clone())
        .collect::<Vec<_>>();

    let conversion_result = std::thread::scope(|scope| {
        let (result_sender, result_receiver) = mpsc::channel();
        let producer_limiter = Arc::clone(&limiter);
        let producer = scope.spawn(move || {
            pool.install(|| {
                receiver
                    .iter()
                    .enumerate()
                    .par_bridge()
                    .try_for_each(|(index, frame)| {
                        let permit = producer_limiter.acquire();
                        let result = frame
                            .to_image()
                            .map_err(|error| VideoConvertError::FrameConvert(error.to_string()))
                            .and_then(|image| render_frame(&image, atlas, render_options));
                        result_sender
                            .send((index, result, permit))
                            .map_err(|_| VideoConvertError::PipelineDisconnected)
                    })
            })
        });

        let mut pending = BTreeMap::new();
        let mut next_index = 0_usize;
        let mut writer = Some(writer);
        let mut encoder: Option<StreamingEncoder<W>> = None;
        let mut dimensions = None;

        for (index, result, permit) in result_receiver {
            pending.insert(index, (result, permit));
            while let Some((result, permit)) = pending.remove(&next_index) {
                let canvas = result?;
                if encoder.is_none() {
                    dimensions = Some((canvas.width, canvas.height));
                    encoder = Some(StreamingEncoder::new(
                        writer.take().expect("writer is consumed once"),
                        fps,
                        include_color,
                        format,
                        dictionary.clone(),
                    )?);
                }
                encoder
                    .as_mut()
                    .expect("encoder initialized by first frame")
                    .push_frame(canvas)?;
                drop(permit);
                next_index += 1;
            }
        }

        producer
            .join()
            .map_err(|_| VideoConvertError::WorkerPanic)??;

        let (width, height) = dimensions.ok_or(VideoConvertError::NoFrames)?;
        let writer = encoder.ok_or(VideoConvertError::NoFrames)?.finish()?;
        Ok((
            VideoConvertSummary {
                frame_count: next_index,
                width,
                height,
            },
            writer,
        ))
    });

    let decoder_result = decoder_handle
        .join()
        .map_err(|_| VideoConvertError::Decode("decoder thread panicked".to_string()))?;
    decoder_result.map_err(|error| VideoConvertError::Decode(error.to_string()))?;

    conversion_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use topoglyph_atlas::atlas::AtlasOptions;

    fn solid_image(w: u32, h: u32, rgb: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(w, h, Rgb(rgb)))
    }

    fn line_image(w: u32, h: u32) -> DynamicImage {
        let mut buf: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(w, h, Rgb([0, 0, 0]));
        for x in 0..w {
            buf.put_pixel(x, h / 2, Rgb([255, 255, 255]));
        }
        DynamicImage::ImageRgb8(buf)
    }

    #[test]
    fn render_frame_on_blank_image_produces_all_space_canvas() {
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let image = solid_image(32, 32, [10, 10, 10]);
        let options = FrameRenderOptions {
            columns: 4,
            rows: Some(4),
            ..Default::default()
        };
        let canvas = render_frame(&image, &atlas, &options).unwrap();
        assert_eq!(canvas.width, 4);
        assert_eq!(canvas.height, 4);
        assert!(canvas.cells.iter().all(|c| c.token == " "));
    }

    #[test]
    fn render_frame_on_striped_image_produces_nonspace_cells() {
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let image = line_image(64, 64);
        let options = FrameRenderOptions {
            columns: 8,
            rows: Some(8),
            ..Default::default()
        };
        let canvas = render_frame(&image, &atlas, &options).unwrap();
        assert!(canvas.cells.iter().any(|c| c.token != " "));
    }

    #[test]
    fn convert_frames_rejects_empty_input() {
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let err = convert_frames(&[], &atlas, &FrameRenderOptions::default(), 24.0, false);
        assert!(matches!(err, Err(VideoConvertError::NoFrames)));
    }

    #[test]
    fn convert_frames_produces_one_tglyph_frame_per_input_image() {
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let options = FrameRenderOptions {
            columns: 4,
            rows: Some(4),
            ..Default::default()
        };
        let images = vec![
            solid_image(32, 32, [10, 10, 10]),
            line_image(32, 32),
            solid_image(32, 32, [10, 10, 10]),
        ];
        let anim = convert_frames(&images, &atlas, &options, 24.0, false).unwrap();
        assert_eq!(anim.frames.len(), 3);
        assert_eq!(anim.fps, 24.0);
        assert!(!anim.include_color);
    }

    #[test]
    fn convert_frames_static_input_produces_a_valid_animation_with_empty_deltas() {
        // A "video" of the same still frame repeated: every delta section
        // should be empty once serialized (see
        // topoglyph_output::animation's own equivalent test), and decoding
        // the resulting text should round-trip.
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let options = FrameRenderOptions {
            columns: 4,
            rows: Some(4),
            ..Default::default()
        };
        let frame = line_image(32, 32);
        let images = vec![frame.clone(), frame.clone(), frame];
        let anim = convert_frames(&images, &atlas, &options, 30.0, false).unwrap();
        let text = anim.to_text();
        let decoded = TglyphAnimation::decode(&text).unwrap();
        assert_eq!(decoded.frames.len(), 3);
    }
}
