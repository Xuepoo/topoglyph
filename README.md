# TopoGlyph

English | [简体中文](README.zh-CN.md)

**Topology + Glyph: Quantizing Image Geometric Topology into Text Glyphs.**

TopoGlyph is a high-performance mathematical engine within the Vectomancy ecosystem. Unlike traditional converters that simply "map pixel brightness to ASCII", TopoGlyph is a completely new typographical system based on geometric topology mapping. It performs deep subpixel extraction on vector or raster images, clips the contours into high-resolution 16×32 Bit Masks, and then matches them against a Glyph Atlas using a 6-factor score (mask/topology/orientation/density/centroid/curvature) refined over several Neighbor Relaxation rounds. This identifies the most structurally accurate characters, ultimately reconstructing a pure text image complete with ANSI truecolor.

## Core Architecture

The project utilizes a multi-crate Workspace structure, divided into:

- `topoglyph-core`: Defines the fundamental 16×32 `CellMask`, 8-port `PortMask`, the Top-K + Neighbor Relaxation glyph matcher, and subgrid clipping algorithms based on Liang-Barsky exact segment clipping.
- `topoglyph-atlas`: Manages the glyph library. Ships with a built-in 17-glyph Unicode line-drawing set (full-length lines, rounded corners, T-junctions, and half-length stubs), plus real TrueType/OpenType font rasterization for custom character pools — including CJK ideographs and Emoji, with correct double-width `cell_width` metadata via East Asian Width data.
- `topoglyph-vectomancy`: The adapter layer bridging `vectomancy-geometry` and `vectomancy-raster`. It provides the input data source, parsing PNG/JPG raster images (Zhang-Suen skeleton extraction) or mock JSON scenes into a standardized `PolylineScene`, then applies RDP simplification + Chaikin smoothing before grid clipping.
- `topoglyph-output`: The renderer, with Plain Text, ANSI Truecolor, HTML, Debug SVG, and JSON Debug encoders, plus the `.tglyph` frame-differential text animation format.
- `topoglyph-video` (optional, native-only; enabled by the CLI's default `video` feature): Converts video files to `.tglyph` text animations via FFmpeg, treating a video as nothing more than a sequence of images run through the same still-image pipeline.
- `topoglyph-cli`: The command-line interface entry point, providing `render`, `atlas inspect`, `video`, and `play` subcommands.

## How to Run

Build the CLI (binary name is `topoglyph`, not `topoglyph-cli`):

```bash
cargo build --release
./target/release/topoglyph /path/to/image.png -W 120 -H 60 > output.txt
```

Or run it directly without building first:

```bash
cargo run --bin topoglyph -- /path/to/image.png -W 120 -H 60 > output.txt
```

### Subcommands

- `topoglyph render <image> [options]` (also the implicit default when no subcommand is given, so `topoglyph <image>` works too): converts a still image to text art. Key options: `-W/-H` (grid size), `-C/--charset lines|ascii|blocks|braille|custom`, `--font <path>` + `--custom-chars "..."` for a custom glyph pool, `--glyph-mode set|weighted`, `--preset line-art|han-emoji`, `--output-format text|html|debug-svg|json`, `--invert`, `--tolerance`/`--chaikin-iters` for pre-smoothing, `--top-k`/`--relaxation-rounds` for match tuning.
- `topoglyph atlas inspect [options]`: dumps a glyph atlas's glyph count, index bucket sizes, and per-glyph features as JSON, without rendering any image.
- `topoglyph video <input.mp4> -o <output.tglyph> [options]`: converts a video file to a `.tglyph` text animation. Colors are off by default (`--color` to opt in) — the `.tglyph` format is a plain-text, frame-differential sequence (first frame in full, subsequent frames record only the cells that changed), so it compresses well with a generic tool like `gzip` on top.
- `topoglyph play <animation.tglyph> [--loop] [--no-color]`: plays back a `.tglyph` animation in the terminal using ANSI cursor-reset (`\x1b[H`) for flicker-free playback at the recorded frame rate.

### 🚨 Why do I see gibberish like `\x1b[38;2;...` in VS Code?

By default, TopoGlyph outputs terminal escape codes with **ANSI Truecolor** (to preserve the colors of the image in pure text).

Since standard text editors (like VS Code or Notepad) parse these as normal strings by default, you will see the raw escape codes (e.g., ` [38;2;255;123;50m`), which stretches and breaks the layout structure.

**There are three correct ways to view the output:**

1. **Directly in the Terminal (Recommended)**
   Use the `cat` command directly in your terminal. Most modern terminals (like Ghostty, Kitty, Alacritty, etc.) natively support rendering Truecolor:
   ```bash
   cat output.txt
   ```
2. **Run directly in the VS Code Integrated Terminal**
   Typing the `cat` command above in the integrated terminal at the bottom of VS Code will also display the colors correctly.
3. **View the text via a VS Code Extension**
   If you want to directly open the `.txt` file in VS Code and see the colors, you need to install an extension like **"ANSI Colors"** or **"Log File Highlighter"**. Once installed, VS Code will correctly parse normal files containing color sequences and hide those ugly escape codes.

Pass `--no-color` to `render`/`play`, or `-C ascii`/`-C blocks`/`-C braille` instead of the default `-C lines`, if you'd rather work with plain, uncolored text.

## Roadmap

Check out `topoglyph-docs/TODO.md` in the parent directory for the latest development plans!
