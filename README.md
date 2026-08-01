# TopoGlyph

[English](README.md) | [简体中文](README.zh-CN.md)

**Topology + Glyph: Quantizing Image Geometric Topology into Text Glyphs.**

TopoGlyph is a high-performance mathematical engine within the Vectomancy ecosystem. Unlike traditional converters that simply "map pixel brightness to ASCII", TopoGlyph is a completely new typographical system based on geometric topology mapping. It performs deep subpixel extraction on vector or raster images, clips the contours into high-resolution 16×32 Bit Masks, and then matches them against a Glyph Atlas using a 6-factor score (mask/topology/orientation/density/centroid/curvature) refined over several Neighbor Relaxation rounds. This identifies the most structurally accurate characters, ultimately reconstructing a pure text image complete with ANSI truecolor.

## Features

- **Geometric Topology Mapping**: Transcends simple brightness-based ASCII conversion by utilizing subpixel extraction, 16×32 Bit Masks, and a 6-factor scoring system (mask, topology, orientation, density, centroid, curvature) to find structurally accurate characters.
- **Top-K & Neighbor Relaxation**: Refines matches over multiple rounds of neighbor relaxation to ensure local topological continuity and high-fidelity text reconstruction.
- **Custom Glyph Atlas**: Ships with a built-in 17-glyph Unicode line-drawing set (full-length lines, rounded corners, T-junctions, and half-length stubs) and supports true TrueType/OpenType font rasterization. This enables custom character pools, including CJK ideographs and Emojis, complete with correct `cell_width` metadata based on East Asian Width data.
- **Multi-format Output & Animation**: Renders outputs as Plain Text, ANSI Truecolor, HTML, Debug SVG, or JSON Debug formats. It also introduces `.tglyph`, a highly compressible frame-differential text animation format for videos.

## Installation

Install via crates.io:

```bash
cargo install topoglyph-cli
```

Or build from source (requires the Rust toolchain):

```bash
git clone https://github.com/Xuepoo/vectomancy.git
cd vectomancy/topoglyph
cargo build --release
```

Run via Container (Docker/Podman):

```bash
# Build the container image locally
docker build -t topoglyph .
# Run directly with volume mounting
docker run --rm -v $(pwd):/data topoglyph -- /data/input.png -W 120 -H 60 > /data/output.txt
```

## CLI Usage

The CLI entry point provides several subcommands for rendering images, inspecting atlases, and processing videos.

### 1. Render Subcommand

Convert a static image to text art (also the implicit default when no subcommand is given):

```bash
topoglyph render <image> [OPTIONS]
```

Key Options:
- `-W, --width <WIDTH>` / `-H, --height <HEIGHT>`: Output grid dimensions. Omit both to derive a resolution-aware grid from the source without upscaling, capped at 600 columns × 300 rows. Set one dimension to derive the other from the source and cell aspect ratios; set both for an exact fixed grid.
- `-C, --charset <CHARSET>`: Output character set (`lines`, `ascii`, `blocks`, `braille`, `custom`).
- `--font <PATH>` & `--custom-chars <STRING>`: Specify a custom TTF/OTF font and character pool for mapping.
- `--glyph-mode <MODE>`: Select character mapping mode (`set` or `weighted`).
- `--preset <PRESET>`: Apply configuration presets (`line-art`, `han-emoji`).
- `--output-format <FORMAT>`: Output format (`text`, `html`, `debug-svg`, `json`).
- `--invert`: Invert color sampling.
- `--tolerance`, `--chaikin-iters`: Control pre-smoothing.
- `--top-k`, `--relaxation-rounds`: Adjust match tuning and neighbor relaxation quality.

### 2. Atlas Inspect Subcommand

Dump a glyph atlas's glyph count, index bucket sizes, and per-glyph features as JSON (does not render any image):

```bash
topoglyph atlas inspect [OPTIONS]
```

### 3. Video Subcommand

Convert a video file to a `.tglyph` text animation. Colors are off by default (`--color` to opt in). Video frames use the same resolution-aware Auto grid as still images when `--width` and `--height` are omitted. The `.tglyph` format is a plain-text, frame-differential sequence that compresses extremely well with generic tools like `gzip`:

```bash
topoglyph video <input.mp4> -o <output.tglyph> [OPTIONS]
```

### 4. Play Subcommand

Playback a `.tglyph` animation directly in the terminal using ANSI cursor-reset sequences (`\x1b[H`) for flicker-free rendering at the recorded frame rate:

```bash
topoglyph play <animation.tglyph> [--loop] [--no-color]
```

## FAQ

**Q: Why do I see gibberish like `\x1b[38;2;...` when I open the output text file in VS Code?**
**A:** By default, TopoGlyph outputs terminal escape codes with **ANSI Truecolor** to preserve the colors of the original image in pure text. Standard text editors parse these as normal strings by default, exposing the raw escape codes (e.g., ` [38;2;255;123;50m`) which stretches and breaks the layout. 

There are three correct ways to view the output:
1. **Directly in the Terminal (Recommended):** Most modern terminals natively support rendering Truecolor. Simply use `cat output.txt`.
2. **VS Code Integrated Terminal:** Typing `cat output.txt` in the terminal at the bottom of VS Code will also display colors correctly.
3. **VS Code Extension:** Install an extension like "ANSI Colors" or "Log File Highlighter" so VS Code can parse color sequences and hide the escape codes.

*(If you prefer uncolored plain text, pass `--no-color` to `render`/`play`, or use `-C ascii`/`-C blocks`/`-C braille`.)*

## Core Architecture

The project utilizes a multi-crate Workspace structure, divided into:

- `topoglyph-core`: Defines the fundamental 16×32 `CellMask`, 8-port `PortMask`, the Top-K + Neighbor Relaxation glyph matcher, and subgrid clipping algorithms based on Liang-Barsky exact segment clipping.
- `topoglyph-atlas`: Manages the glyph library, ships with a built-in 17-glyph Unicode line-drawing set, and handles font rasterization.
- `topoglyph-vectomancy`: The adapter layer bridging `vectomancy-geometry` and `vectomancy-raster` for data sourcing, applying RDP simplification and Chaikin smoothing.
- `topoglyph-output`: The renderer, exporting Plain Text, ANSI Truecolor, HTML, Debug SVG, JSON Debug, and the `.tglyph` text animation format.
- `topoglyph-video`: Converts video files to `.tglyph` animations via FFmpeg.
- `topoglyph-cli`: The command-line interface entry point.

## License

This project is licensed under the MIT License.