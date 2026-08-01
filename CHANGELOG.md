# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
