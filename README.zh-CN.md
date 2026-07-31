# TopoGlyph

[English](README.md) | 简体中文

**Topology + Glyph：将图像的几何拓扑量化成文字 Glyph。**

TopoGlyph 是 Vectomancy 生态下的一款高性能数学引擎。它并非传统的“按像素亮度映射 ASCII”的转换器，而是一个基于几何拓扑映射的全新排版系统。它通过对矢量或光栅图像进行深度的子像素提取、将轮廓裁切（Clipping）为 16×32 的高分辨率 Bit Mask，然后与字体图集 (Glyph Atlas) 进行 6 因子评分（掩码/拓扑/方向/密度/质心/曲率）并经过多轮邻接松弛（Neighbor Relaxation）比对，找到结构上最匹配的字符，最终重建出带有 ANSI 真彩色的纯文本图像。

## 核心架构设计

项目采用多 Crate Workspace 结构，划分为：

- `topoglyph-core`：定义底层的 16×32 `CellMask`，8 端口 `PortMask`，基于 Top-K 候选池 + 邻接松弛的字形匹配器，以及基于 Liang-Barsky 精确线段裁剪的子网格切割算法。
- `topoglyph-atlas`：管理字形库。内置 17 种 Unicode 线条字形（全长线、圆角、T 字交叉、半长线段），同时支持真实 TrueType/OpenType 字体栅格化以构建自定义字符池——包括中日韩汉字与 Emoji，并通过东亚宽度数据正确标注双宽字符的 `cell_width` 元数据。
- `topoglyph-vectomancy`：适配层，桥接 `vectomancy-geometry` 和 `vectomancy-raster`，提供输入数据源，将 PNG/JPG 光栅图像（Zhang-Suen 骨架提取）或模拟 JSON 场景解析为标准的 `PolylineScene`，网格切割前还会做 RDP 简化 + Chaikin 平滑处理。
- `topoglyph-output`：渲染器，包含纯文本、ANSI 真彩色、HTML、Debug SVG、JSON Debug 编码器，以及 `.tglyph` 帧差分文本动画格式。
- `topoglyph-video`（可选，仅原生环境；CLI 默认 `video` feature 已开启）：通过 FFmpeg 把视频文件转换为 `.tglyph` 文本动画，把视频当作纯粹的图片帧序列，逐帧走跟静态图片一样的处理管线。
- `topoglyph-cli`：命令行入口，提供 `render`、`atlas inspect`、`video`、`play` 四个子命令。

## 如何运行

构建 CLI（二进制名是 `topoglyph`，不是 `topoglyph-cli`）：

```bash
cargo build --release
./target/release/topoglyph /path/to/image.png -W 120 -H 60 > output.txt
```

或者不预先构建，直接运行：

```bash
cargo run --bin topoglyph -- /path/to/image.png -W 120 -H 60 > output.txt
```

### 子命令

- `topoglyph render <image> [选项]`（不带子命令时的隐式默认行为，所以 `topoglyph <image>` 也能直接用）：把静态图片转换为文字画。关键选项：`-W/-H`（网格尺寸）、`-C/--charset lines|ascii|blocks|braille|custom`、`--font <路径>` + `--custom-chars "..."` 用于自定义字符池、`--glyph-mode set|weighted`、`--preset line-art|han-emoji`、`--output-format text|html|debug-svg|json`、`--invert`、`--tolerance`/`--chaikin-iters` 控制前置平滑、`--top-k`/`--relaxation-rounds` 控制匹配质量。
- `topoglyph atlas inspect [选项]`：输出字库的字形数量、索引桶大小、每个字形的特征值 JSON 摘要，不渲染任何图片。
- `topoglyph video <input.mp4> -o <output.tglyph> [选项]`：把视频文件转换为 `.tglyph` 文本动画。颜色默认关闭（`--color` 显式开启）——`.tglyph` 是纯文本的帧差分序列（第一帧全量，后续帧只记录变化的格子），因此在外层叠加 `gzip` 等通用压缩效果也不错。
- `topoglyph play <animation.tglyph> [--loop] [--no-color]`：在终端里用 ANSI 光标复位（`\x1b[H`）无闪烁地按记录的帧率回放 `.tglyph` 动画。

### 🚨 为什么 VS Code 中看到的是乱码 `\x1b[38;2;...`？

TopoGlyph 默认输出的是带有 **ANSI 真彩色 (Truecolor)** 的终端转义码（为了在纯文本中带上图片的颜色）。

由于常规的文本编辑器（如 VS Code、记事本）默认把它们当作普通字符串解析，所以你会看到很多类似 ` [38;2;255;123;50m` 这样的源码，从而导致结构被拉伸和错乱。

**正确的查看方式有三种：**

1. **终端直接查看（推荐）**
   在终端中使用 `cat` 命令直接打印，大部分现代终端（如 Ghostty, Kitty, Alacritty 等）都原生支持渲染 Truecolor：
   ```bash
   cat output.txt
   ```
2. **VS Code 终端直接运行**
   在 VS Code 底部的 Integrated Terminal 里直接输入上述 `cat` 命令，也是能正常看到颜色的。
3. **VS Code 安装插件查看文本**
   如果你希望直接在 VS Code 里点开 `.txt` 文件并看到颜色，你需要安装类似 **"ANSI Colors"** 这样的插件，安装后，VS Code 就能正确解析包含颜色序列的普通文件，并隐藏那些丑陋的转义代码。

如果想直接用纯文本（不带颜色），给 `render`/`play` 加上 `--no-color`，或者把默认的 `-C lines` 换成 `-C ascii`/`-C blocks`/`-C braille`。

## 路线图 (Roadmap)

查看上级目录 `topoglyph-docs/TODO.md` 获取最新的开发计划！
