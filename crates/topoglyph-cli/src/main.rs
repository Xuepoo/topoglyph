use clap::{Args as ClapArgs, CommandFactory, Parser, Subcommand};
use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use topoglyph::atlas::atlas::{AtlasOptions, GlyphAtlas, GlyphIndex};
use topoglyph::core::clipping;
use topoglyph::core::geometry::GridOptions;
use topoglyph::core::matching::{self, MatchOptions, MatchWeights};
use topoglyph::input::adapter::{self, SmoothingOptions};
use topoglyph::output::encoder::{
    AnsiEncoder, DebugSvgEncoder, HtmlEncoder, JsonDebugEncoder, PlainTextEncoder, TextEncoder,
};
#[cfg(feature = "video")]
use topoglyph::video::FrameRenderOptions;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Generate shell completion script for the given shell and print to stdout.
    #[arg(long, value_name = "SHELL")]
    generate_completions: Option<clap_complete::Shell>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render an image to text art.
    Render(RenderArgs),
    /// Inspect a glyph atlas and print its glyph features as JSON.
    Atlas(AtlasArgs),
    /// Convert a video into a compact `.tglyph` text animation.
    #[cfg(feature = "video")]
    Video(VideoArgs),
    /// Play back a `.tglyph` text animation in the terminal.
    Play(PlayArgs),
}

#[derive(ClapArgs, Debug, Clone)]
struct AtlasArgs {
    #[command(subcommand)]
    action: AtlasAction,
}

#[derive(Subcommand, Debug, Clone)]
enum AtlasAction {
    /// Build the requested atlas and print a summary of its glyphs and
    /// index buckets as JSON.
    Inspect {
        /// Charset to inspect: 'lines', 'ascii', 'blocks', 'braille', 'custom'.
        #[arg(short = 'C', long, default_value = "lines")]
        charset: String,

        /// Custom characters to use when charset is 'custom'.
        #[arg(long, default_value = "")]
        custom_chars: String,

        /// Path to TTF/OTF font file (required unless charset is 'lines').
        #[arg(long)]
        font: Option<String>,
    },
}

#[derive(ClapArgs, Debug, Clone)]
struct RenderArgs {
    /// Input file path (image)
    input: String,

    /// Width of the output text grid. Omit both width and height to derive a
    /// resolution-aware grid from the source, capped at 120×60 cells.
    #[arg(short = 'W', long)]
    width: Option<usize>,

    /// Height of the output text grid. When exactly one grid dimension is
    /// provided, the other is derived from the source and cell aspect ratios.
    #[arg(short = 'H', long)]
    height: Option<usize>,

    /// Charset to use: 'lines', 'ascii', 'blocks', 'braille', 'custom'
    #[arg(short = 'C', long, default_value = "lines")]
    charset: String,

    /// Custom characters to use when charset is 'custom'
    #[arg(long, default_value = "")]
    custom_chars: String,

    /// Path to TTF/OTF font file (required for rasterization)
    #[arg(long)]
    font: Option<String>,

    /// Enable plain text mode (no colors). Ignored when --output-format is
    /// something other than 'text'.
    #[arg(long, default_value_t = false)]
    no_color: bool,

    /// Invert sampled path colors (#rrggbb -> #(255-r)(255-g)(255-b)).
    ///
    /// Note: this only affects color, not glyph selection. Skeleton
    /// extraction runs on Sobel edge magnitude, which is invariant to a
    /// global brightness inversion of the source image (|grad(255-I)| ==
    /// |grad(I)|), so there is no meaningful "invert brightness" step to
    /// apply on top of this — the topology/shape the glyphs are matched
    /// against is unaffected either way.
    #[arg(long, default_value_t = false)]
    invert: bool,

    /// Output format: 'text' (plain or ANSI, per --no-color), 'html',
    /// 'debug-svg', or 'json' (full per-cell match data, including scores).
    #[arg(long, default_value = "text")]
    output_format: String,

    /// RDP simplification tolerance applied to the extracted skeleton before
    /// grid clipping. `0` disables simplification. See
    /// vectomancy-docs/parameter_tuning_guide.md for tuning guidance.
    #[arg(long, default_value_t = 0.5)]
    tolerance: f64,

    /// Number of Chaikin corner-cutting smoothing iterations applied after
    /// RDP simplification. `0` disables smoothing.
    #[arg(short = 'c', long, default_value_t = 1)]
    chaikin_iters: usize,

    /// Glyph match weighting preset: 'line-art' favors topology and port
    /// connectivity; 'han-emoji' favors density and mask shape.
    #[arg(long, default_value = "line-art")]
    preset: String,

    /// Size of each cell's shape-ranked candidate pool.
    #[arg(long, default_value_t = 8)]
    top_k: usize,

    /// Number of Neighbor Relaxation passes (recommended range: 3-5).
    #[arg(long, default_value_t = 3)]
    relaxation_rounds: usize,

    /// Glyph selection mode for custom font pools: 'set' treats every glyph
    /// equally; 'weighted' prefers characters repeated in --custom-chars
    /// when shape scores are close.
    #[arg(long, default_value = "set")]
    glyph_mode: String,
}

#[cfg(feature = "video")]
#[derive(ClapArgs, Debug, Clone)]
struct VideoArgs {
    /// Input video file path (any container/codec ffmpeg can decode).
    input: String,

    /// Output `.tglyph` animation file path.
    #[arg(short = 'o', long)]
    output: String,

    /// Width of the output text grid. Omit both width and height to derive a
    /// resolution-aware grid from the source, capped at 120×60 cells.
    #[arg(short = 'W', long)]
    width: Option<usize>,

    /// Height of the output text grid (omit to auto-calculate from the video
    /// frame's aspect ratio; recommended — avoids the 4:3→1:1 squish).
    #[arg(short = 'H', long)]
    height: Option<usize>,

    /// Charset to use: 'lines', 'ascii', 'blocks', 'braille', 'custom'.
    #[arg(short = 'C', long, default_value = "lines")]
    charset: String,

    /// Custom characters to use when charset is 'custom'.
    #[arg(long, default_value = "")]
    custom_chars: String,

    /// Path to TTF/OTF font file (required unless charset is 'lines').
    #[arg(long)]
    font: Option<String>,

    /// RDP simplification tolerance applied to each frame.
    #[arg(long, default_value_t = 0.5)]
    tolerance: f64,

    /// Chaikin smoothing iterations applied to each frame.
    #[arg(short = 'c', long, default_value_t = 1)]
    chaikin_iters: usize,

    /// Glyph match weighting preset: 'line-art' or 'han-emoji'.
    #[arg(long, default_value = "line-art")]
    preset: String,

    /// Number of glyph candidates retained for matching each cell.
    #[arg(long, default_value_t = 8)]
    top_k: usize,

    /// Number of neighbor-relaxation passes.
    #[arg(long, default_value_t = 3)]
    relaxation_rounds: usize,

    /// Output frame rate recorded in the `.tglyph` header. This is purely
    /// timing metadata for playback; every decoded video frame becomes one
    /// output frame regardless of this value (no resampling/frame
    /// dropping). Omit to use the source video's own average frame rate
    /// (falls back to 24 if that can't be probed); recording a rate that
    /// doesn't match the source desyncs playback from any audio sidecar.
    #[arg(long)]
    fps: Option<f32>,

    /// Sample and record path colors. Disabled by default for smaller output.
    #[arg(long, default_value_t = false)]
    color: bool,

    /// Number of parallel frame-rendering workers. Defaults to all
    /// available CPU threads.
    #[arg(short = 'j', long)]
    threads: Option<std::num::NonZeroUsize>,

    /// Write human-readable text instead of compact binary output.
    #[arg(long, default_value_t = false)]
    text_format: bool,
}

#[derive(ClapArgs, Debug, Clone)]
struct PlayArgs {
    /// Input `.tglyph` animation file path.
    input: String,

    /// Loop playback indefinitely instead of stopping after the last frame.
    #[arg(long, default_value_t = false)]
    r#loop: bool,

    /// Disable ANSI color escapes even if the animation has recorded color
    /// (equivalent to `render`'s `--no-color`).
    #[arg(long, default_value_t = false)]
    no_color: bool,

    /// Disable audio playback even if a sidecar `<input>.wav` exists
    /// (written by `topoglyph video`).
    #[arg(long, default_value_t = false)]
    no_audio: bool,
}

fn resolve_frequency_bias(glyph_mode: &str) -> Result<f32, String> {
    match glyph_mode {
        "set" => Ok(0.0),
        // 1.0 puts the frequency term on the same order of magnitude as the
        // mask term (MatchWeights::mask defaults to 1.0), enough to break
        // shape/topology near-ties without overriding a clearly better
        // shape match.
        "weighted" => Ok(1.0),
        other => Err(format!(
            "Invalid --glyph-mode '{other}': expected 'set' or 'weighted'"
        )),
    }
}

fn resolve_preset(name: &str) -> Result<MatchWeights, String> {
    match name {
        "line-art" => Ok(MatchWeights::line_art_preset()),
        "han-emoji" => Ok(MatchWeights::han_emoji_preset()),
        other => Err(format!(
            "Invalid preset '{other}': expected 'line-art' or 'han-emoji'"
        )),
    }
}

fn build_atlas(
    charset: &str,
    custom_chars: &str,
    font: &Option<String>,
) -> Result<GlyphAtlas, String> {
    if charset == "lines" {
        GlyphAtlas::from_text("", &AtlasOptions::default())
            .map_err(|e| format!("Failed to build built-in glyph atlas: {e}"))
    } else if font.is_none() && charset != "custom" {
        let glyphs = match charset {
            "ascii" => topoglyph::atlas::precomputed::build_ascii_glyphs(),
            "blocks" => topoglyph::atlas::precomputed::build_blocks_glyphs(),
            "braille" => topoglyph::atlas::precomputed::build_braille_glyphs(),
            _ => return Err(format!("Invalid charset specified: '{charset}'")),
        };
        let index = GlyphIndex::build(&glyphs);
        Ok(GlyphAtlas {
            font_id: format!("precomputed_{charset}"),
            glyphs,
            index,
        })
    } else {
        let chars = if charset == "custom" {
            custom_chars.to_string()
        } else {
            GlyphAtlas::get_charset_string(charset)
                .ok_or_else(|| format!("Invalid charset specified: '{charset}'"))?
                .to_string()
        };

        let font_path = font
            .clone()
            .ok_or_else(|| "A --font must be provided for custom text rasterization".to_string())?;
        let font_bytes = std::fs::read(&font_path)
            .map_err(|e| format!("Failed to read font file '{font_path}': {e}"))?;
        GlyphAtlas::from_custom_font(&chars, &font_bytes, &AtlasOptions::default())
            .map_err(|e| format!("Failed to rasterize font atlas: {e}"))
    }
}

fn run_render(args: RenderArgs) -> Result<Vec<u8>, String> {
    // 1. Read Input
    let bytes =
        std::fs::read(&args.input).map_err(|e| format!("Failed to read '{}': {e}", args.input))?;

    // 2. Decode to a smoothed PolylineScene: skeleton extraction (Zhang-Suen,
    // via vectomancy-raster) followed by RDP simplification + Chaikin
    // smoothing so pixel-grid jitter doesn't fragment the subcell mask.
    let smoothing = SmoothingOptions {
        tolerance: args.tolerance,
        chaikin_iters: args.chaikin_iters,
    };
    let mut scene = adapter::raster_to_smoothed_scene(&bytes, true, &smoothing)
        .map_err(|e| format!("Failed to decode image: {e}"))?;

    if args.invert {
        scene = adapter::invert_scene_colors(&scene);
    }

    // 3. Setup Subcell grid clipping (Liang-Barsky segment clipping)
    let grid_opts = GridOptions {
        columns: args.width,
        rows: args.height,
        ..Default::default()
    };
    let (out_cols, out_rows, cell_descriptors) = clipping::process_scene(&scene, &grid_opts);

    // 4. Generate GlyphAtlas
    let atlas = build_atlas(&args.charset, &args.custom_chars, &args.font)?;

    // 5. Match glyphs using the 6-factor scoring formula (mask/topology/
    // orientation/density/centroid/curvature) over a Top-K candidate pool
    // refined across several Neighbor Relaxation rounds, weighted per the
    // selected preset, plus an optional frequency bias (--glyph-mode).
    let mut weights = resolve_preset(&args.preset)?;
    weights.frequency_bias = resolve_frequency_bias(&args.glyph_mode)?;
    let match_options = MatchOptions {
        top_k: args.top_k,
        relaxation_rounds: args.relaxation_rounds,
    };
    let canvas = matching::match_scene_indexed(
        out_cols,
        out_rows,
        &cell_descriptors,
        &atlas.glyphs,
        Some(&atlas.index),
        &weights,
        &match_options,
    );

    // 6. Encode and output
    match args.output_format.as_str() {
        "text" => {
            if args.no_color {
                PlainTextEncoder::new()
                    .encode(&canvas)
                    .map_err(|e| format!("Failed to encode output: {e}"))
            } else {
                AnsiEncoder::new()
                    .encode(&canvas)
                    .map_err(|e| format!("Failed to encode output: {e}"))
            }
        }
        "html" => HtmlEncoder::new()
            .encode(&canvas)
            .map_err(|e| format!("Failed to encode output: {e}")),
        "debug-svg" => DebugSvgEncoder::default()
            .encode(&canvas)
            .map_err(|e| format!("Failed to encode output: {e}")),
        "json" => JsonDebugEncoder::new()
            .encode(&canvas)
            .map_err(|e| format!("Failed to encode output: {e}")),
        other => Err(format!(
            "Invalid --output-format '{other}': expected 'text', 'html', 'debug-svg', or 'json'"
        )),
    }
}

fn run_atlas(args: AtlasArgs) -> Result<Vec<u8>, String> {
    match args.action {
        AtlasAction::Inspect {
            charset,
            custom_chars,
            font,
        } => {
            let atlas = build_atlas(&charset, &custom_chars, &font)?;

            let summary = serde_json::json!({
                "font_id": atlas.font_id,
                "glyph_count": atlas.glyphs.len(),
                "index": {
                    "by_ports_bucket_count": atlas.index.by_ports.len(),
                    "by_density_bucket_sizes": atlas.index.by_density.iter().map(Vec::len).collect::<Vec<_>>(),
                    "by_cell_width_bucket_count": atlas.index.by_cell_width.len(),
                },
                "glyphs": atlas.glyphs.iter().map(|g| serde_json::json!({
                    "token": g.token,
                    "cell_width": g.cell_width,
                    "ports": format!("{:?}", g.ports),
                    "density": g.density,
                    "curvature": g.curvature,
                    "centroid": g.centroid,
                    "orientation": g.orientation,
                    "stroke_count": g.stroke_count,
                    "frequency": g.frequency,
                })).collect::<Vec<_>>(),
            });

            serde_json::to_vec_pretty(&summary)
                .map_err(|e| format!("Failed to serialize atlas summary: {e}"))
        }
    }
}

#[cfg(feature = "video")]
fn run_video(args: VideoArgs) -> Result<String, String> {
    let atlas = build_atlas(&args.charset, &args.custom_chars, &args.font)?;

    let mut weights = resolve_preset(&args.preset)?;
    weights.frequency_bias = 0.0; // no --glyph-mode for video yet; set-mode default

    let options = FrameRenderOptions {
        columns: args.width,
        rows: args.height,
        smoothing: SmoothingOptions {
            tolerance: args.tolerance,
            chaikin_iters: args.chaikin_iters,
        },
        weights,
        match_options: MatchOptions {
            top_k: args.top_k,
            relaxation_rounds: args.relaxation_rounds,
        },
        sample_color: args.color,
    };

    let threads = args.threads.map_or_else(
        || std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        std::num::NonZero::get,
    );
    let format = if args.text_format {
        topoglyph::output::stream::AnimationFormat::Text
    } else {
        topoglyph::output::stream::AnimationFormat::Binary
    };
    // Default to the source's own average frame rate rather than an
    // arbitrary constant: the encoder records every decoded frame with no
    // resampling, so a recorded `.tglyph` fps that doesn't match the
    // source's actual rate makes `topoglyph play` drift out of sync with
    // any audio sidecar (audio duration is fixed; video "duration" is
    // frame_count / fps).
    let fps = args.fps.unwrap_or_else(|| {
        topoglyph::video::probe_frame_rate(std::path::Path::new(&args.input)).unwrap_or(24.0)
    });

    let output_file = std::fs::File::create(&args.output)
        .map_err(|e| format!("Failed to create '{}': {e}", args.output))?;

    // Determinate when the container reports a usable frame count (most
    // well-formed video files); falls back to an indeterminate spinner
    // (still shows elapsed time and a moving indicator) for inputs where
    // it can't be probed, e.g. some streamed/live-recorded sources.
    let progress = match topoglyph::video::probe_frame_count(std::path::Path::new(&args.input)) {
        Some(total) => indicatif::ProgressBar::new(total),
        None => indicatif::ProgressBar::new_spinner(),
    };
    progress.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} frames ({eta})",
        )
        .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
        .progress_chars("#>-"),
    );

    let conversion = topoglyph::video::convert_video_file_to_writer(
        std::path::Path::new(&args.input),
        &atlas,
        &options,
        topoglyph::video::VideoOutputOptions {
            fps,
            include_color: args.color,
            threads,
            format,
        },
        output_file,
        |written| progress.set_position(written as u64),
    );
    progress.finish_and_clear();
    let (summary, output_file) = match conversion {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_file(&args.output);
            return Err(format!("Failed to convert video: {error}"));
        }
    };
    drop(output_file);
    let byte_len = std::fs::metadata(&args.output)
        .map_err(|e| format!("Failed to inspect '{}': {e}", args.output))?
        .len();

    let output_path = std::path::Path::new(&args.output);
    let audio_path = topoglyph::video::audio::sidecar_audio_path(output_path);
    let legacy_wav_path = topoglyph::video::audio::sidecar_wav_path(output_path);
    let _ = std::fs::remove_file(&audio_path);
    let _ = std::fs::remove_file(&legacy_wav_path);
    let audio_mode = topoglyph::video::audio::extract_audio_to_m4a(
        std::path::Path::new(&args.input),
        &audio_path,
    )
    .map_err(|e| format!("Failed to extract audio: {e}"))?;
    let audio_message = match audio_mode {
        topoglyph::video::audio::AudioExtractMode::NoAudio => String::new(),
        topoglyph::video::audio::AudioExtractMode::Remuxed => {
            format!(", audio remuxed to {}", audio_path.display())
        }
        topoglyph::video::audio::AudioExtractMode::Transcoded => {
            format!(", audio encoded to {}", audio_path.display())
        }
    };

    Ok(format!(
        "Wrote {} frames at {}fps ({} bytes, {}x{}) to {}{}",
        summary.frame_count,
        fps,
        byte_len,
        summary.width,
        summary.height,
        args.output,
        audio_message
    ))
}

/// Plays back a decoded `.tglyph` animation in the terminal: clears the
/// screen once, then for each frame moves the cursor back to the top-left
/// (`\x1b[H`) and overwrites it in place rather than scrolling, so playback
/// doesn't produce a wall of stacked frames (`topoglyph-docs/TODO.md` 0.5.0:
/// "使用 ANSI `\x1b[H` 游标复位实现终端内极高帧率的无闪烁动画播放").
///
/// The animation is centered in the terminal (both horizontally and
/// vertically) rather than pinned to the top-left corner, and if a sidecar
/// `<input>.wav` exists (written by `topoglyph video`, see
/// `topoglyph_video::audio`) it's played back through the default audio
/// device in sync with the frame loop (0.2.2: 播放的时候应该带有音频播放，
/// 视频应该居中播放).
fn run_play(args: PlayArgs) -> Result<(), String> {
    let bytes =
        std::fs::read(&args.input).map_err(|e| format!("Failed to read '{}': {e}", args.input))?;

    // Only the header (width/height/fps/frame_count) is read up front; each
    // pass below streams frames one at a time via `PlaybackFrames` instead
    // of materializing every decoded `TextCanvas` (see that type's docs).
    // This is what lets `play` handle arbitrarily long animations (a full
    // `bad-apple.mp4` conversion is thousands of frames) without exhausting
    // memory decoding the whole thing before the first frame ever draws.
    let header =
        PlaybackFrames::new(&bytes).map_err(|e| format!("Failed to parse animation: {e}"))?;
    let (width, height, fps, frame_count) = (
        header.width(),
        header.height(),
        header.fps(),
        header.frame_count(),
    );
    drop(header);

    if frame_count == 0 {
        return Ok(());
    }

    let frame_duration = if fps > 0.0 {
        Duration::from_secs_f32(1.0 / fps)
    } else {
        Duration::from_secs_f32(1.0 / 24.0)
    };

    // Left/top padding that centers the fixed WIDTH x HEIGHT grid within
    // whatever the terminal's current size is. Recomputed once up front
    // (not per-frame): resizing mid-playback would misalign the `\x1b[H`
    // cursor-reset trick's assumption of a stable frame origin, so this
    // matches a real video player's behavior of sizing to the terminal at
    // launch, not live-resizing.
    let (term_cols, term_rows) = terminal_size::terminal_size()
        .map(|(terminal_size::Width(w), terminal_size::Height(h))| (w as usize, h as usize))
        .unwrap_or((width, height));
    let left_pad = " ".repeat(term_cols.saturating_sub(width) / 2);
    let top_pad = "\n".repeat(term_rows.saturating_sub(height) / 2);

    // Prefer the compact M4A sidecar written by current versions, while
    // retaining read-only support for legacy `.tglyph.wav` pairs.
    let input_path = std::path::Path::new(&args.input);
    let m4a_path = topoglyph::video::audio::sidecar_audio_path(input_path);
    let wav_path = topoglyph::video::audio::sidecar_wav_path(input_path);
    let audio_path = if m4a_path.is_file() {
        Some(m4a_path)
    } else if wav_path.is_file() {
        Some(wav_path)
    } else {
        None
    };
    // Use `rodio::Player` (not a bare `Decoder` added straight to the
    // mixer) so we can query `Player::get_pos()`: the actual decoded
    // playback position the audio thread is at, sampled every 5ms. Video
    // frame pacing below is driven off this when audio is present, rather
    // than a software `Instant`-based clock -- a wall clock can only ever
    // approximate the audio device's real clock (resampling, buffer
    // underruns/overruns, and device clock rate itself all drift relative
    // to `Instant`, in either direction, and no amount of software-clock
    // tuning removes that: the two clocks are physically different).
    // Syncing video to the actual audio position instead is what every
    // real video player does and eliminates this whole class of bug.
    let _audio_stream_handle;
    let audio_player = if !args.no_audio {
        if let Some(audio_path) = audio_path {
            match std::fs::File::open(&audio_path)
                .map_err(|e| e.to_string())
                .and_then(|file| rodio::Decoder::try_from(file).map_err(|e| e.to_string()))
                .and_then(|source| {
                    rodio::DeviceSinkBuilder::open_default_sink()
                        .map_err(|e| e.to_string())
                        .map(|handle| (handle, source))
                }) {
                Ok((handle, source)) => {
                    let player = rodio::Player::connect_new(handle.mixer());
                    player.append(source);
                    _audio_stream_handle = Some(handle);
                    Some(player)
                }
                Err(e) => {
                    eprintln!(
                        "warning: failed to play audio sidecar '{}': {e}",
                        audio_path.display()
                    );
                    _audio_stream_handle = None;
                    None
                }
            }
        } else {
            _audio_stream_handle = None;
            None
        }
    } else {
        _audio_stream_handle = None;
        None
    };

    let mut stdout = std::io::stdout();
    // Clear the screen once up front; subsequent frames only reposition the
    // cursor rather than clearing again, which is what avoids visible
    // flicker at high frame rates.
    let _ = write!(stdout, "\x1b[2J\x1b[H{top_pad}");

    loop {
        // Re-open a fresh streaming reader each pass: the readers only move
        // forward, so looping means starting over from the same in-memory
        // `bytes` rather than re-reading the file.
        let mut frames =
            PlaybackFrames::new(&bytes).map_err(|e| format!("Failed to parse animation: {e}"))?;

        // Pace frames against the actual audio playback position
        // (`Player::get_pos()`) when a sidecar is playing, instead of a
        // software `Instant`-based clock. A wall clock only ever
        // approximates the audio device's real clock -- resampling,
        // buffer underruns/overruns, and the device's own clock rate all
        // drift `Instant` relative to actual audio playback, in either
        // direction, and that drift is invisible to (and uncorrectable
        // by) a purely software-timed loop. Syncing to the real audio
        // position is what every real video player does and eliminates
        // this whole class of bug rather than chasing another symptom of
        // it. Falls back to a wall-clock anchor (`pass_start + index *
        // frame_duration`) when there's no audio to sync to.
        let pass_start = Instant::now();
        let mut frame_index: u64 = 0;

        while let Some(canvas) = frames
            .next_frame()
            .map_err(|e| format!("Failed to parse animation: {e}"))?
        {
            let encoded = if args.no_color {
                PlainTextEncoder::new().encode(&canvas)
            } else {
                AnsiEncoder::new().encode(&canvas)
            }
            .map_err(|e| format!("Failed to encode frame: {e}"))?;

            // Reset to the top of the padded frame area (below top_pad,
            // which was written once and never overwritten) and re-indent
            // every row with left_pad so the grid stays horizontally
            // centered too.
            let cursor_row = 1 + top_pad.matches('\n').count();
            let _ = write!(stdout, "\x1b[{cursor_row};1H");
            let encoded_text = String::from_utf8_lossy(&encoded);
            for line in encoded_text.lines() {
                let _ = writeln!(stdout, "{left_pad}{line}");
            }
            let _ = stdout.flush();

            frame_index += 1;
            let target = frame_duration * frame_index as u32;
            let elapsed = match &audio_player {
                Some(player) => player.get_pos(),
                None => pass_start.elapsed(),
            };
            if elapsed < target {
                std::thread::sleep(target - elapsed);
            }
        }
        if !args.r#loop {
            break;
        }
    }

    Ok(())
}

/// Format-agnostic view over either streaming frame reader
/// (`topoglyph::output::animation::{TextFrameReader, BinaryFrameReader}`),
/// picked once based on the buffer's magic bytes (mirroring
/// `TglyphAnimation::from_bytes`'s dispatch) so `run_play` doesn't need to
/// match on the format at every call site.
enum PlaybackFrames<'a> {
    Text(topoglyph::output::animation::TextFrameReader<'a>),
    Binary(topoglyph::output::binary::BinaryFrameReader<'a>),
}

impl<'a> PlaybackFrames<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, topoglyph::output::animation::TglyphError> {
        if topoglyph::output::binary::is_binary(bytes) {
            Ok(Self::Binary(
                topoglyph::output::binary::BinaryFrameReader::new(bytes)?,
            ))
        } else {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                topoglyph::output::animation::TglyphError::MalformedHeader(
                    "magic",
                    "TGLYPHB2 or TOPOGLYPH-ANIM v1",
                    "<invalid UTF-8, and not v2 binary magic>".to_string(),
                )
            })?;
            Ok(Self::Text(
                topoglyph::output::animation::TextFrameReader::new(text)?,
            ))
        }
    }

    fn next_frame(
        &mut self,
    ) -> Result<
        Option<topoglyph::core::canvas::TextCanvas>,
        topoglyph::output::animation::TglyphError,
    > {
        match self {
            Self::Text(reader) => reader.next_frame(),
            Self::Binary(reader) => reader.next().transpose(),
        }
    }
}

impl PlaybackFrames<'_> {
    fn width(&self) -> usize {
        match self {
            Self::Text(reader) => reader.width(),
            Self::Binary(reader) => reader.width(),
        }
    }

    fn height(&self) -> usize {
        match self {
            Self::Text(reader) => reader.height(),
            Self::Binary(reader) => reader.height(),
        }
    }

    fn fps(&self) -> f32 {
        match self {
            Self::Text(reader) => reader.fps(),
            Self::Binary(reader) => reader.fps(),
        }
    }

    fn frame_count(&self) -> usize {
        match self {
            Self::Text(reader) => reader.frame_count(),
            Self::Binary(reader) => reader.frame_count(),
        }
    }
}

/// Known top-level subcommands/flags. Anything else in `argv[1]` is treated
/// as an image path and implicitly routed to the `render` subcommand, so
/// `topoglyph-cli <image.png> ...` keeps working exactly as it did before
/// `render`/`atlas` subcommands existed.
const KNOWN_FIRST_ARGS: &[&str] = &[
    "render",
    "atlas",
    "video",
    "play",
    "help",
    "-h",
    "--help",
    "-V",
    "--version",
    "--generate-completions",
];

/// Rewrites `argv` so a bare `topoglyph-cli <image.png> ...` invocation
/// (no subcommand) becomes `topoglyph-cli render <image.png> ...` before
/// clap ever sees it. clap's derive API doesn't support "infer a default
/// subcommand" natively when that subcommand also needs a required
/// positional argument, so this is done as an explicit pre-processing step
/// instead of trying to express it as an `Option<Command>` +
/// `#[command(flatten)]` combo (which clap resolves by making `<INPUT>` a
/// top-level required positional, breaking `atlas inspect`).
fn with_inferred_subcommand(mut argv: Vec<String>) -> Vec<String> {
    let first_real_arg = argv.get(1).map(String::as_str);
    if let Some(arg) = first_real_arg {
        if !KNOWN_FIRST_ARGS.contains(&arg) {
            argv.insert(1, "render".to_string());
        }
    }
    argv
}

fn main() -> ExitCode {
    let argv = with_inferred_subcommand(std::env::args().collect());
    let cli = Cli::parse_from(argv);

    if let Some(shell) = cli.generate_completions {
        let mut cmd = Cli::command();
        clap_complete::generate(shell, &mut cmd, "topoglyph", &mut std::io::stdout());
        return ExitCode::SUCCESS;
    }

    let command = match cli.command {
        Some(c) => c,
        None => {
            eprintln!("error: a subcommand is required. Run `topoglyph --help` for usage.");
            return ExitCode::FAILURE;
        }
    };

    match command {
        // `play` streams frames directly to stdout as it plays, rather than
        // building one big `Vec<u8>` result like the other subcommands
        // below, so it's handled as its own arm instead of going through
        // the shared "collect bytes, then print once" path.
        Command::Play(args) => match run_play(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        #[cfg(feature = "video")]
        Command::Video(args) => match run_video(args) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        other => {
            let result = match other {
                Command::Render(args) => run_render(args),
                Command::Atlas(args) => run_atlas(args),
                #[cfg(feature = "video")]
                Command::Video(_) => unreachable!("handled above"),
                Command::Play(_) => unreachable!("handled above"),
            };

            match result {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => {
                        println!("{}", text);
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: encoder produced invalid UTF-8: {e}");
                        ExitCode::FAILURE
                    }
                },
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn all_help_contains_only_public_user_facing_language() {
        fn collect_help(mut command: clap::Command, help: &mut String) {
            help.push_str(&command.render_long_help().to_string());
            for subcommand in command.get_subcommands().cloned().collect::<Vec<_>>() {
                collect_help(subcommand, help);
            }
        }

        let mut help = String::new();
        collect_help(Cli::command(), &mut help);

        assert!(!help.contains("topoglyph-docs"));
        assert!(!help.contains("TODO.md"));
        assert!(!help.contains("RenderArgs::"));
    }

    #[test]
    fn video_threads_must_be_positive() {
        let parsed = Cli::try_parse_from([
            "topoglyph",
            "video",
            "input.mp4",
            "--output",
            "output.tglyph",
            "--threads",
            "0",
        ]);

        assert!(parsed.is_err());
    }
}
