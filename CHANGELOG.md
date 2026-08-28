# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.6] - 2026-08-28

### Added
- **feat(codec): .tglyph binary v3 TGLYPHB3 – unsigned gap, adaptive Full/Sparse/Bitmap, global color palette, v2 backward compat.** `topoglyph_output::binary` now writes `TGLYPHB3` by default: changed-cell gaps are unsigned varints (instead of v2 zigzag, doubling the 1-byte range from 0..63 to 0..127), per-frame adaptive `SparseDelta` / `BitmapDelta` / `Full` (`BitmapDelta` uses LSB row-major bitset `varint(bitset_len)+bitset bytes` for mid-density ~12-50% when `changed>cell_count/8 && sparse>bitmap`, `Full` at `changed*2>cell_count`), and a global RGB palette for `include_color` (1-byte palette index vs 1+3 B). `BinaryFrameReader`/`TglyphAnimation::from_bytes` auto-detect `TGLYPHB2` (zigzag, no palette/type) and `TGLYPHB3`; streaming encoder (`stream.rs`) emits `TGLYPHB3` with `palette_len=0` fallback to legacy 1+3 B color until a global pass is feasible. `is_binary` matches either magic.

### Changed
- **deps: vectomancy-geometry/raster/video 7.1.2/7.0.0 -> 8.1.0 (ffmpeg-next 9.0).** Unified to a single `8.1.0` to avoid duplicate `StyledPath` copies in mixed 7.x/8.x graphs (supersedes dependabot #25/#26). `vectomancy-video 8.1.0` moves `ffmpeg-next 8.1 -> 9.0` (libavutil 61) fixing the FFmpeg 9 link error noted in CTX-0003; `topoglyph-video` now carries an explicit `ffmpeg-next = "9.0"` dep (previously transitive).

## [0.3.4] - 2026-08-01

### Changed
- **`topoglyph play` now paces video frames against the actual audio playback position instead of a software wall clock, when an audio sidecar is playing.** 0.3.2 and 0.3.3 fixed the recorded `fps` and a compounding software-sleep drift respectively, but a purely `Instant`-based pacer can only ever approximate the real audio device's clock -- resampling, buffer underrun/overrun, and the audio device's own clock rate all drift a software clock relative to actual audio playback (in either direction), and no amount of wall-clock-side tuning removes that gap, since the two clocks are physically different. `topoglyph play` now uses `rodio::Player` (rather than adding the decoder straight to the mixer) so it can read `Player::get_pos()` -- the audio thread's own tracked decode position, sampled every 5ms -- and schedules each video frame against that instead of `Instant::now()` whenever audio is present. Falls back to the previous absolute wall-clock anchor when `--no-audio` is passed or no sidecar exists. This is the same audio-master-clock approach real video players use, and removes the whole class of clock-drift bug rather than fixing another symptom of it.

## [0.3.3] - 2026-08-01

### Fixed
- **`topoglyph play` scheduled each frame's delay relative to that frame's own start**, so `std::thread::sleep`'s inherent overshoot (OS scheduler granularity, typically a few milliseconds) compounded linearly across every frame instead of being corrected -- a long animation's video track would drift progressively later than any audio sidecar (which stays exact via the OS audio clock) the longer playback ran, even with a correct recorded `fps`. Each pass now schedules frame *N*'s deadline as `pass_start + N * frame_duration`, so a late frame's overshoot no longer pushes every subsequent frame later by the same amount. Verified against a 6572-frame, 30fps `bad-apple.mp4` conversion: wall-clock playback time is 219.15s against an expected `6572/30 = 219.07s` and the audio sidecar's 219.15s (previously this class of drift, on top of 0.3.2's fps fix, could still leave video and audio audibly out of sync by the end of a long clip).

## [0.3.2] - 2026-08-01

### Fixed
- **`topoglyph video --fps` defaulted to a hardcoded `24.0` regardless of the source's actual frame rate.** Since the encoder records every decoded frame verbatim with no resampling/frame-dropping, a recorded `fps` that doesn't match the source desyncs `topoglyph play` from any audio sidecar over the length of the clip -- e.g. a 30fps, 6572-frame source (219.1s actual/audio duration) played back at the old 24fps default in `6572/24 ≈ 273.8s`, drifting the video track roughly 55s longer than the audio by the end. `--fps` is now optional and defaults to the source's own average frame rate (probed via the new `topoglyph_video::probe_frame_rate`, without decoding any frames), falling back to `24.0` only when that can't be determined. Re-encoding an affected `.tglyph` file with this version fixes existing desync.

## [0.3.1] - 2026-08-01

### Fixed
- **0.3.0's Auto grid caps (600x300) regressed sizing for anything at or above roughly 480px wide**, producing a near-pixel-for-pixel grid instead of a downsampled one: `.tglyph` output ballooned 5-16x, lines were too wide for a real terminal (breaking even `cat`), and `topoglyph play` could exhaust memory materializing every frame at the inflated cell count on longer videos. Automatic caps are back to the terminal-sane `120x60` (matching the historical fixed 120-column default; one character cell corresponds to several source pixels, not one).
- **`topoglyph play` fully materialized every decoded frame (`Vec<TextCanvas>`) before playback started**, so a long `.tglyph` animation's entire decoded size had to fit in memory at once. `topoglyph_output::animation` now exposes `TextFrameReader`/`crate::binary::BinaryFrameReader`, which decode one frame at a time (keeping only the previous frame needed for delta application); `play` streams through these instead of `TglyphAnimation::from_bytes`, so playback memory stays bounded regardless of animation length. `TglyphAnimation::decode`/`crate::binary::decode` (full materialization) are retained and now build on top of the same streaming readers.
- **`AnsiEncoder` appended an unconditional trailing color reset after the last row's newline**, which became its own phantom line once split on `.lines()` in `topoglyph-cli`'s playback loop -- every colored `.tglyph` frame printed one extra row, causing terminal playback to visibly drift down over time. The reset now lands before the final newline and only when a color is still active.

### Added
- **`topoglyph video` conversion progress bar**: shows a determinate progress bar (frames written / total, with ETA) when the input's frame count can be probed from its container metadata, falling back to an indeterminate spinner otherwise. Backed by `indicatif`; frame count is probed via `topoglyph_video::probe_frame_count` without decoding any frames.

## [0.3.0] - 2026-08-01

### Changed
- **Resolution-aware Auto grids:** Leaving both output dimensions unset now derives the text grid from the source pixel dimensions without upscaling, preserves physical aspect through the configured cell ratio, and caps automatic output at 600 columns × 300 rows. This replaces the fixed 120-column default across `topoglyph-core`, still-image CLI rendering, and video conversion. Supplying one dimension derives the other from aspect; supplying both keeps an exact fixed grid.
- **`GridOptions::columns` and `FrameRenderOptions::columns` are now optional:** `None` selects the shared adaptive resolver. This is a breaking API change for direct Rust consumers; wrap fixed widths in `Some(...)`.

## [0.2.3] - 2026-08-01

### Changed
- **Bounded-memory video conversion:** `topoglyph video` now streams decoded frames through a bounded render pipeline and writes frame deltas directly to the `.tglyph` output. It retains only in-flight frames and the previous rendered canvas instead of every decoded image and full `TextCanvas`; `--threads` now controls a local render pool and rejects zero.
- **Compact M4A audio sidecars:** AAC source audio is remuxed without re-encoding; other codecs are transcoded to 128 kbit/s AAC. New conversions write `<output>.tglyph.m4a` instead of uncompressed PCM WAV, while `topoglyph play` retains read-only compatibility with existing `.tglyph.wav` sidecars.
- **User-facing CLI help:** Removed internal Rust type names and development-document references from `--help` output, and documented `-j, --threads` directly.

## [0.2.2] - 2026-08-01

### Added
- **Audio support for `topoglyph video`/`topoglyph play`**: `topoglyph video` now decodes the source video's audio track (via `ffmpeg-next`'s safe decoder + resampler API) into a sidecar `<output>.tglyph.wav` file next to the `.tglyph` animation (any standard-conformant player can open the WAV directly; `.tglyph` itself stays audio-free, since it's a small purpose-built text-cell format, not a general media container). `topoglyph play` automatically detects and plays a matching `<input>.wav` sidecar through the default output device (via `rodio`) in sync with the frame loop; pass `--no-audio` to opt out. Silent sources (GIFs, muted recordings) simply produce no sidecar rather than an error.
- **Compact binary `.tglyph` v2 format (`topoglyph_output::binary`)**: Real animation content analysis (a full `bad-apple.mp4` conversion, 6572 frames at 120x45) found that v1 text delta lines (`<row>,<col>,<token>`) spend ~90% of their bytes on decimal row/col digits and comma separators, with actual token data only ~10% — and only 19 distinct tokens ever appear across the whole animation despite the 5400-cell grid. v2 exploits both: every distinct token gets a small dictionary index (varint-encoded) instead of repeating its UTF-8 bytes, and changed-cell positions are a zigzag-varint *delta from the previous changed cell's flat index* within the frame instead of two decimal numbers. Measured on that same animation: **21.0MB -> 4.9MB (23% of the original size)**, byte-for-byte identical decoded content verified against the v1 text output. `topoglyph video` now writes this format by default; pass `--text-format` to opt back into the original human-readable v1 text `.tglyph` (e.g. for inspecting with `cat`/a text editor). `topoglyph play` and `TglyphAnimation::from_bytes` auto-detect either format via magic bytes, so existing v1 `.tglyph` files keep working unmodified.

### Fixed
- **`topoglyph play` rendered pinned to the terminal's top-left corner instead of centered**, and pinned the web renderer's output the same way regardless of the panel's actual size (including fullscreen). `play` now computes horizontal/vertical padding from the current terminal size (via `terminal_size`) once at launch and centers the fixed WIDTH x HEIGHT grid within it; the web renderer's `.editor-content` is now a centering flex container instead of top-left-anchored padding, fixing both the normal and fullscreen view.
- **The web renderer's `<cols>x<rows> · <ms>ms` dimension readout (`#output-meta`) never became visible after rendering.** `app.js` set its `textContent` but never cleared the `display:none` the element started with, and the element itself had no CSS at all (so even when visible it rendered as unstyled inline text instead of a fixed readout). Fixed both: `app.js` now clears `display:none` after populating it, and it's now pinned to the bottom-left corner of the editor panel via CSS.

## [0.2.1] - 2026-08-01

### Fixed
- **Fixed aspect-ratio distortion and a video-conversion OOM crash (root cause)**: `topoglyph_core::clipping::process_scene` computed the output grid's aspect ratio from `scene.bounds` — the extracted skeleton's *content* bounding box — instead of `scene.dimensions`, the source image/frame's actual pixel size. Two consequences:
  1. **`topoglyph video` could crash with a multi-terabyte allocation** (`memory allocation of 37013760000000 bytes failed`) whenever a frame's skeleton degenerated to a near-zero-width/height bbox (e.g. a blank or near-blank frame): `bounds.max_x - bounds.min_x` clamped to `1e-5`, so dividing by it sent the computed aspect ratio — and therefore `columns * rows` — into the billions.
  2. **Per-frame "zoom/pan" jitter** in both the web renderer and `topoglyph video`: since `bounds` is however much of the frame the skeleton happens to occupy, and that varies frame to frame even though the source video's pixel dimensions never change, auto-sizing from it made the subject appear to randomly grow/shrink/shift between frames instead of staying anchored to one fixed frame.

  Fixed by anchoring both the aspect-ratio calculation and the coordinate-to-grid mapping on `scene.dimensions` (with a `(1, 1)` fallback only for the degenerate zero-dimension case), so the output grid always reflects the source image/frame's real proportions and stays fixed across every frame of a conversion.
- **`topoglyph render`'s fixed default height (80) and `topoglyph video`'s fixed default height (60) forced every non-matching-aspect-ratio input into a distorted grid.** `--height`/`-H` is now optional on both subcommands (previously required an explicit override to avoid distortion); omitting it lets the engine auto-calculate the correct height from the input's aspect ratio, matching the auto-fit `--width` behavior below.
- **`topoglyph render`/`topoglyph video`'s fixed default widths (160/120, inconsistent with each other) no longer adapt to the terminal.** `--width`/`-W` is now optional on both subcommands; omitting it auto-fits the current terminal's column count (via the new `terminal_size` dependency), falling back to `120` when stdout isn't a TTY (piped to a file or another process).

### Added
- **`topoglyph video --threads`/`-j <N>`**: Limits the number of CPU threads used for parallel per-frame conversion (backed by a scoped `rayon::ThreadPoolBuilder`, applied once before any conversion work runs). Previously conversion always used every available core via rayon's global default, which could pin the whole machine at 100% CPU for the duration of a conversion with no way to leave headroom for other work.

## [0.2.0] - 2026-07-31

### Added
- **Expanded built-in line atlas**: Increased from 17 to 22 glyphs. Added the diagonal cross (`╳`) and sharp corners (`┌`, `┐`, `└`, `┘`) to provide more variety and geometric matching options for the default `lines` charset.
- **Embedded Minimal Glyphs (CLI)**: Added zero-configuration, "out-of-the-box" support for standard charsets (`ascii`, `blocks`, `braille`). Pre-computed mask and feature data are now embedded directly into the binary, completely removing the requirement for users to manually supply a `--font` path for these core charsets.

### Changed
- **Core Matching Metric (Sparsity Bias Fix)**: Replaced the raw XOR distance `(a ^ b).count_ones()` with the Jaccard distance / Intersection over Union (IoU) metric for the `mask_distance` feature. This fixes a severe systemic "sparsity bias" where sparse glyphs like half-lines (`╶`, `╴`) overwhelmingly won out over structurally correct dense glyphs (like corners or full lines) due to missing normalization.

## [0.1.0] - 2026-07-31

First real release. Converts a raster image into topology-matched
text art via subcell grid analysis, with a small built-in glyph atlas
and support for custom TTF/OTF fonts, video-to-`.tglyph` animation
conversion, and multiple output formats.

### Added

- **Video / `.tglyph` animation support**: New `topoglyph-video` crate (native-only, behind the `video` cargo feature) converts a video file to a `.tglyph` text animation via FFmpeg, treating a video as nothing more than a sequence of images run through the same still-image pipeline (parallelized per-frame with `rayon`). The `.tglyph` format itself (`topoglyph_output::animation::TglyphAnimation`) is a plain-text, frame-differential sequence: the first frame is written in full, and every subsequent frame records only the cells that changed. Color is off by default (`--color` opts in); the plain-text format compresses well with a generic tool like `gzip` on top, so no custom binary/base-N encoding was added.
- **CLI `video`/`play` subcommands**: `topoglyph video <input> -o <output.tglyph> [options]` encodes a video; `topoglyph play <animation.tglyph> [--loop] [--no-color]` plays one back in the terminal using ANSI cursor-reset (`\x1b[H`) for flicker-free playback at the recorded frame rate.
- **`--invert`**: Inverts sampled path colors (`#rrggbb` -> `#(255-r)(255-g)(255-b)`). Brightness inversion was intentionally not implemented on top of this: skeleton extraction runs on Sobel edge magnitude, which is invariant to a global brightness inversion of the source image, so glyph selection is unaffected either way.
- **`--glyph-mode set|weighted`**: Custom character pools (`--charset custom --font ...`) now track each grapheme's relative frequency in `--custom-chars`. Under `weighted` mode, more frequent characters are preferred when shape/topology scores are otherwise close; `set` mode (the default) ignores frequency entirely.
- **CJK / Emoji glyph support**: `topoglyph-atlas` now computes each custom-font grapheme's real terminal display width via `unicode-width`'s East Asian Width data, so CJK ideographs and most Emoji correctly report `cell_width = 2` instead of the previous hardcoded `1`. Verified against real fonts (Noto Sans SC, Segoe UI Emoji) and real image input.
- **Top-K candidate pool + multi-round Neighbor Relaxation**: `match_scene_full` now builds a per-cell Top-K shape-ranked candidate pool, then refines it across several Neighbor Relaxation rounds (per `topoglyph-docs/technical.md` section 2.3), replacing the earlier single full-atlas rescan pass.
- **`GlyphIndex` real indexing**: `by_ports`/`by_density`/`by_cell_width` are now actually built and queryable (`glyphs_with_any_port`, `glyphs_near_density`) instead of always being empty.
- **6-factor glyph matching**: `MatchWeights`/`shape_score`/`topology_mismatch` now implement the full `Score = wm*mask_dist + wt*topology_dist + wo*orientation_dist + wd*density_dist + wc*centroid_dist + wk*curvature_dist` formula from `topoglyph-docs/technical.md` section 2.2, including `line_art_preset()`/`han_emoji_preset()`.
- **Debug/HTML encoders**: Added `JsonDebugEncoder` (full per-cell match data including scores), `HtmlEncoder`, and `DebugSvgEncoder` alongside the existing Plain Text/ANSI encoders.
- **Expanded built-in line atlas**: Grew from 9 to 17 glyphs (added T-junctions and half-length stroke stubs), fixing a pathology where any branching or partial-length stroke had no better match than a full 2-port line or the single 4-port cross.
- **`topoglyph-cli render`/`atlas inspect` subcommand structure**: The CLI now has explicit `render`/`atlas` subcommands (plus `video`/`play` above), with argv pre-processing so `topoglyph <image.png> ...` still works without a subcommand.
- **Multi-column (CJK/Emoji) glyph layout**: `cell_width` is no longer just metadata. Pool construction excludes any glyph that wouldn't fit in the remaining columns of its row, and the final canvas-building pass lets a wide winner's right-hand neighbor cell(s) render empty rather than being independently matched — a CJK ideograph or Emoji now actually spans two grid columns instead of being squeezed into one.
- **`GlyphIndex`-backed pool construction**: `match_scene_indexed` (a new sibling of `match_scene_full`) takes an optional `&GlyphIndex` and narrows the per-cell candidate scan via `glyphs_fitting_in`'s `by_cell_width` buckets instead of linearly scanning the whole atlas. `GlyphIndex` moved from `topoglyph-atlas` to `topoglyph-core::matching` (re-exported from `topoglyph-atlas` for source compatibility) so the consumer and the type live on the same side of the dependency graph. `topoglyph-cli` and `topoglyph-video` both use the indexed path now.

### Changed

- **Skeleton extraction quality**: `vectomancy-raster::decode_raster_memory` (used by `topoglyph-vectomancy`'s raster adapter) now uses full Zhang-Suen thinning plus endpoint/loop-aware path tracing, replacing a simplified greedy walk that stopped at the first branch point and produced fragmented, noisy skeletons.
- **RDP + Chaikin smoothing before grid clipping**: `topoglyph_vectomancy::adapter::smooth_scene` applies RDP simplification and Chaikin corner-cutting to extracted skeleton paths before they reach `topoglyph_core::clipping`, removing pixel-grid jitter that previously fragmented the subcell mask.
- **Liang-Barsky exact segment clipping**: `topoglyph_core::clipping` replaced a Bresenham pixel-walk (which silently dropped any segment portion crossing outside the canvas) with exact Liang-Barsky line clipping against each cell's boundary.

### Fixed

- CLI error handling: all `unwrap()`/`expect()` calls in `topoglyph-cli` were replaced with `Result` propagation and readable error messages (previously e.g. `-C blocks` without `--font` would panic with a raw Rust backtrace).
- Release/packaging metadata (`Cargo.toml` workspace inheritance, `[[bin]]` name, `Dockerfile`, `nfpm.yaml`, `release.yml`'s crates.io publish order) was copied wholesale from the `vectomancy` repository template and didn't match TopoGlyph's actual crate layout — `cargo build --bin topoglyph` would have failed outright (no such binary target existed; the crate produced `topoglyph-cli` instead). Fixed by adding an explicit `[[bin]] name = "topoglyph"`, introducing `[workspace.package]`/`workspace.dependencies` version inheritance so every internal path dependency carries the `version` crates.io requires, rewriting the `Dockerfile`'s `COPY`/build paths to match the real `crates/topoglyph-*` layout, and rewriting `release.yml`'s `publish-crates` job to follow the real dependency order (`core` -> `atlas`/`output`/`vectomancy` -> facade -> `video` -> `cli`).
- `Cargo.toml`'s `[workspace]` had no `default-members`, so plain `cargo test`/`cargo clippy`/`cargo build` (as run by `.github/workflows/ci.yml`, with no `--workspace`/`-p`) only covered the root `topoglyph` facade package — which has no tests of its own — silently running zero tests in CI despite `cargo test --workspace` reporting dozens passing locally. Fixed by adding an explicit `default-members` list covering every crate.
- Removed the repo-committed `.cargo/config.toml` (`rustflags = ["-C", "target-cpu=native", ...]`). This was originally added to work around a local wasm build issue, but the actual fix needed was a linker choice (`mold`), which belongs in a developer's own global Cargo config (`$CARGO_HOME/config.toml`), not the repository — `target-cpu=native` itself is not required for any `wasm32-unknown-unknown` build in this workspace (verified by building `topoglyph-core`/`-atlas`/`-output`/`-vectomancy` for that target with a clean `CARGO_HOME` and no repo-level Cargo config), and shipping it in the repo would have made `release.yml`'s cross-platform binaries potentially crash (`SIGILL`) on any user CPU lacking whatever instruction-set extensions the CI runner's CPU happens to support.

## [0.0.0] - pre-release scaffold

Never tagged/published; kept here only as a historical record of the
starting point the `[0.1.0]` work above built on.

### Added

- Initial `topoglyph-core`/`topoglyph-atlas`/`topoglyph-output`/`topoglyph-vectomancy`/`topoglyph-cli` crate layout.
- Built-in 9-glyph line-drawing atlas (full-length lines, rounded corners, cross).
- Basic raster-to-text-art pipeline: raster decode -> subcell grid clipping (Bresenham) -> mask+port matching -> Plain Text/ANSI output.
