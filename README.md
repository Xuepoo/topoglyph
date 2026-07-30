# TopoGlyph

English | [简体中文](README.zh-CN.md)

**Topology + Glyph: Quantizing Image Geometric Topology into Text Glyphs.**

TopoGlyph is a high-performance mathematical engine within the Vectomancy ecosystem. Unlike traditional converters that simply "map pixel brightness to ASCII", TopoGlyph is a completely new typographical system based on geometric topology mapping. It performs deep subpixel extraction on vector or raster images, clips the contours into high-resolution 16×32 Bit Masks, and then matches them against a Glyph Atlas using XOR Distance and Port Hamming Distance. This identifies the most structurally accurate characters, ultimately reconstructing a pure text image complete with ANSI truecolor.

## Core Architecture

The project utilizes a multi-crate Workspace structure, divided into:

- `topoglyph-core`: Defines the fundamental 16×32 `CellMask`, 8-port `PortMask`, ultra-fast matcher based on XOR distance, and subgrid clipping algorithms based on DDA / Liang-Barsky.
- `topoglyph-atlas`: Manages the font glyph library, pre-rasterizing glyphs into Masks for searching (The 0.1.0 MVP version currently ships with 9 minimalist built-in Unicode line drawing characters. Future versions will support real font rasterization).
- `topoglyph-vectomancy`: The adapter layer bridging `vectomancy-geometry` and `vectomancy-raster`. It provides the input data source, parsing PNG/JPG and vector polygons into a standardized `PolylineScene`.
- `topoglyph-output`: The renderer, featuring capabilities such as ANSI Truecolor text output.
- `topoglyph-cli`: The command-line interface entry point.

## How to Run

You can use the CLI to convert an image into a text image containing ANSI terminal escape sequences:

```bash
cargo run -p topoglyph-cli -- /path/to/image.png > output.txt
```

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

## Roadmap
Check out `topoglyph-docs/TODO.md` in the parent directory for the latest development plans!
