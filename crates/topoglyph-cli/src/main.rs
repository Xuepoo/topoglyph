use clap::{Args as ClapArgs, Parser, Subcommand};
use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use topoglyph::atlas::atlas::{AtlasOptions, GlyphAtlas, GlyphIndex};
use topoglyph::core::clipping;
use topoglyph::core::geometry::GridOptions;
use topoglyph::core::matching::{self, MatchOptions, MatchWeights};
use topoglyph::input::adapter::{self, SmoothingOptions};
use topoglyph::output::animation::TglyphAnimation;
use topoglyph::output::encoder::{
    AnsiEncoder, DebugSvgEncoder, HtmlEncoder, JsonDebugEncoder, PlainTextEncoder, TextEncoder,
};
#[cfg(feature = "video")]
use topoglyph::video::FrameRenderOptions;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render an image to text art (the default when no subcommand is given;
    /// see [`main`]'s subcommand-inference shim).
    Render(RenderArgs),
    /// Inspect a glyph atlas: dump its glyph count, index bucket sizes, and
    /// per-glyph features, without rendering any image. See
    /// `topoglyph-docs/TODO.md` 0.4.0 ("CLI 添加 `atlas inspect` 子命令").
    Atlas(AtlasArgs),
    /// Convert a video file to a `.tglyph` text animation (frame-differential
    /// text art sequence, not an actual video file). See
    /// `topoglyph-docs/TODO.md` 0.5.0.
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

    /// Width of the output text grid
    #[arg(short = 'W', long, default_value_t = 160)]
    width: usize,

    /// Height of the output text grid
    #[arg(short = 'H', long, default_value_t = 80)]
    height: usize,

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

    /// Glyph match weighting preset: 'line-art' (favors topology/port
    /// connectivity, good for box-drawing charsets) or 'han-emoji' (favors
    /// density/mask shape, good for CJK/emoji charsets). See
    /// topoglyph-docs/technical.md section 2.2.
    #[arg(long, default_value = "line-art")]
    preset: String,

    /// Size of each cell's shape-ranked Top-K candidate pool used during
    /// Neighbor Relaxation. See topoglyph-docs/technical.md section 2.3.
    #[arg(long, default_value_t = 8)]
    top_k: usize,

    /// Number of Neighbor Relaxation passes (recommended range: 3-5).
    #[arg(long, default_value_t = 3)]
    relaxation_rounds: usize,

    /// Glyph selection mode for the character pool: 'set' (every glyph is
    /// equally likely; picked purely by shape/topology score) or 'weighted'
    /// (glyphs that repeat more often in --custom-chars are preferred when
    /// shape/topology scores are close). Only affects --charset custom with
    /// a --font; the built-in 'lines'/'ascii'/'blocks'/'braille' charsets
    /// have no meaningful repeat frequency to weight by. See
    /// topoglyph-docs/requirements.md section 3.2.
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

    /// Width of the output text grid.
    #[arg(short = 'W', long, default_value_t = 120)]
    width: usize,

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

    /// RDP simplification tolerance, per-frame (see `RenderArgs::tolerance`).
    #[arg(long, default_value_t = 0.5)]
    tolerance: f64,

    /// Chaikin smoothing iterations, per-frame (see
    /// `RenderArgs::chaikin_iters`).
    #[arg(short = 'c', long, default_value_t = 1)]
    chaikin_iters: usize,

    /// Glyph match weighting preset: 'line-art' or 'han-emoji'.
    #[arg(long, default_value = "line-art")]
    preset: String,

    /// Top-K candidate pool size (see `RenderArgs::top_k`).
    #[arg(long, default_value_t = 8)]
    top_k: usize,

    /// Neighbor Relaxation rounds (see `RenderArgs::relaxation_rounds`).
    #[arg(long, default_value_t = 3)]
    relaxation_rounds: usize,

    /// Output frame rate recorded in the `.tglyph` header. This is purely
    /// timing metadata for playback; every decoded video frame becomes one
    /// output frame regardless of this value (no resampling/frame
    /// dropping).
    #[arg(long, default_value_t = 24.0)]
    fps: f32,

    /// Sample and record path colors in the animation. Off by default —
    /// per `topoglyph-docs/TODO.md`, color is opt-in since most `.tglyph`
    /// use cases (terminal ASCII-art playback) want the smaller, colorless
    /// file.
    #[arg(long, default_value_t = false)]
    color: bool,
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
        rows: Some(args.height),
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

    let animation = topoglyph::video::convert_video_file(
        std::path::Path::new(&args.input),
        &atlas,
        &options,
        args.fps,
        args.color,
    )
    .map_err(|e| format!("Failed to convert video: {e}"))?;

    let text = animation.to_text();
    std::fs::write(&args.output, &text)
        .map_err(|e| format!("Failed to write '{}': {e}", args.output))?;

    Ok(format!(
        "Wrote {} frames ({} bytes) to {}",
        animation.frames.len(),
        text.len(),
        args.output
    ))
}

/// Plays back a decoded `.tglyph` animation in the terminal: clears the
/// screen once, then for each frame moves the cursor back to the top-left
/// (`\x1b[H`) and overwrites it in place rather than scrolling, so playback
/// doesn't produce a wall of stacked frames (`topoglyph-docs/TODO.md` 0.5.0:
/// "使用 ANSI `\x1b[H` 游标复位实现终端内极高帧率的无闪烁动画播放").
fn run_play(args: PlayArgs) -> Result<(), String> {
    let text = std::fs::read_to_string(&args.input)
        .map_err(|e| format!("Failed to read '{}': {e}", args.input))?;
    let animation =
        TglyphAnimation::decode(&text).map_err(|e| format!("Failed to parse animation: {e}"))?;

    if animation.frames.is_empty() {
        return Ok(());
    }

    let frame_duration = if animation.fps > 0.0 {
        Duration::from_secs_f32(1.0 / animation.fps)
    } else {
        Duration::from_secs_f32(1.0 / 24.0)
    };

    let mut stdout = std::io::stdout();
    // Clear the screen once up front; subsequent frames only reposition the
    // cursor rather than clearing again, which is what avoids visible
    // flicker at high frame rates.
    let _ = write!(stdout, "\x1b[2J\x1b[H");

    loop {
        for canvas in &animation.frames {
            let frame_start = Instant::now();

            let encoded = if args.no_color {
                PlainTextEncoder::new().encode(canvas)
            } else {
                AnsiEncoder::new().encode(canvas)
            }
            .map_err(|e| format!("Failed to encode frame: {e}"))?;

            let _ = write!(stdout, "\x1b[H");
            let _ = stdout.write_all(&encoded);
            let _ = stdout.flush();

            let elapsed = frame_start.elapsed();
            if elapsed < frame_duration {
                std::thread::sleep(frame_duration - elapsed);
            }
        }
        if !args.r#loop {
            break;
        }
    }

    Ok(())
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

    match cli.command {
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
