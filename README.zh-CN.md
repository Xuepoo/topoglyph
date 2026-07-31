# TopoGlyph

[English](README.md) | [简体中文](README.zh-CN.md)

**Topology + Glyph：将图像的几何拓扑量化成文字 Glyph。**

TopoGlyph 是 Vectomancy 生态下的一款高性能数学引擎。它并非传统的“按像素亮度映射 ASCII”的转换器，而是一个基于几何拓扑映射的全新排版系统。它通过对矢量或光栅图像进行深度的子像素提取、将轮廓裁切（Clipping）为 16×32 的高分辨率 Bit Mask，然后与字体图集 (Glyph Atlas) 进行 6 因子评分（掩码/拓扑/方向/密度/质心/曲率）并经过多轮邻接松弛（Neighbor Relaxation）比对，找到结构上最匹配的字符，最终重建出带有 ANSI 真彩色的纯文本图像。

## 核心功能

- **几何拓扑映射**：超越基于亮度的简单 ASCII 转换。通过子像素提取、16×32 Bit Mask 和 6 因子评分系统（掩码、拓扑、方向、密度、质心、曲率）来精确寻找结构最匹配的字符。
- **Top-K 与邻接松弛**：在多轮邻接松弛（Neighbor Relaxation）中优化匹配结果，确保局部的拓扑连续性，实现高保真文本重建。
- **自定义字体图集 (Glyph Atlas)**：内置 17 种 Unicode 线条字形（全长线、圆角、T 字交叉、半长线段），同时支持真实的 TrueType/OpenType 字体栅格化。你可以自由构建自定义字符池（包括中日韩汉字与 Emoji），系统会通过东亚宽度数据自动处理双宽字符的 `cell_width` 元数据。
- **多格式输出与动画**：支持纯文本、ANSI 真彩色、HTML、Debug SVG 以及 JSON Debug 格式导出。同时引入了高度可压缩的 `.tglyph` 帧差分文本动画格式，用于视频转换。

## 安装方式

通过 crates.io 安装：

```bash
cargo install topoglyph-cli
```

或从源码编译（需要预先安装 Rust 工具链）：

```bash
git clone https://github.com/Xuepoo/vectomancy.git
cd vectomancy/topoglyph
cargo build --release
```

通过容器运行 (Docker/Podman)：

```bash
# 本地构建容器镜像
docker build -t topoglyph .
# 挂载当前目录并直接运行
docker run --rm -v $(pwd):/data topoglyph -- /data/input.png -W 120 -H 60 > /data/output.txt
```

## CLI 基础用法

命令行提供了多个子命令，用于渲染图片、检查字形图集以及处理视频。

### 1. 渲染子命令 (render)

把静态图片转换为文字画（这也是不带子命令时的隐式默认行为）：

```bash
topoglyph render <image> [OPTIONS]
```

常用选项：
- `-W, --width <WIDTH>` / `-H, --height <HEIGHT>`: 设置输出网格的尺寸。
- `-C, --charset <CHARSET>`: 设置输出字符集 (`lines`, `ascii`, `blocks`, `braille`, `custom`)。
- `--font <PATH>` & `--custom-chars <STRING>`: 指定自定义 TTF/OTF 字体文件和用于匹配的字符池。
- `--glyph-mode <MODE>`: 选择字形映射模式 (`set` 或 `weighted`)。
- `--preset <PRESET>`: 应用配置预设 (`line-art`, `han-emoji`)。
- `--output-format <FORMAT>`: 输出格式 (`text`, `html`, `debug-svg`, `json`)。
- `--invert`: 反转采样色彩。
- `--tolerance`, `--chaikin-iters`: 控制前置平滑处理。
- `--top-k`, `--relaxation-rounds`: 调整匹配精细度和邻接松弛的质量。

### 2. 检查字库子命令 (atlas inspect)

输出字库的字形数量、索引桶大小以及每个字形的特征值 JSON 摘要（不渲染任何图片）：

```bash
topoglyph atlas inspect [OPTIONS]
```

### 3. 视频处理子命令 (video)

把视频文件转换为 `.tglyph` 文本动画。颜色默认关闭（需显式传入 `--color` 开启）。`.tglyph` 是一种纯文本的帧差分序列（第一帧全量，后续帧只记录变化的格子），因此在外层叠加 `gzip` 等通用压缩效果极佳：

```bash
topoglyph video <input.mp4> -o <output.tglyph> [OPTIONS]
```

### 4. 播放子命令 (play)

在终端里使用 ANSI 光标复位（`\x1b[H`）无闪烁地按记录的帧率直接回放 `.tglyph` 动画：

```bash
topoglyph play <animation.tglyph> [--loop] [--no-color]
```

## 常见问题排查 (FAQ)

**Q: 为什么我在 VS Code 中打开输出文本，看到的是 `\x1b[38;2;...` 这样的乱码？**
**A:** 默认情况下，TopoGlyph 输出的是带有 **ANSI 真彩色 (Truecolor)** 的终端转义码，这是为了在纯文本中带上图片的颜色。由于常规的文本编辑器（如 VS Code、记事本）默认把它们当作普通字符串解析，所以你会看到这些转义代码，从而导致排版被拉伸和错乱。

正确的查看方式有三种：
1. **终端直接查看（推荐）：** 绝大部分现代终端都原生支持渲染 Truecolor，直接在终端执行 `cat output.txt` 即可。
2. **VS Code 终端直接运行：** 在 VS Code 底部的 Integrated Terminal 里直接输入 `cat output.txt` 也能正常看到颜色。
3. **VS Code 插件：** 如果希望直接点开 `.txt` 文件并看到颜色，请安装 **"ANSI Colors"** 或 **"Log File Highlighter"** 插件，VS Code 就能正确解析颜色序列并隐藏转义代码。

*(如果只想获取无颜色的纯文本，可以在运行 `render`/`play` 时加上 `--no-color` 选项，或者把字符集换成 `-C ascii`/`-C blocks`/`-C braille`。)*

## 核心架构设计

项目采用多 Crate Workspace 结构，划分为：

- `topoglyph-core`：定义底层的 16×32 `CellMask`，8 端口 `PortMask`，基于 Top-K 候选池 + 邻接松弛的字形匹配器，以及基于 Liang-Barsky 精确线段裁剪的子网格切割算法。
- `topoglyph-atlas`：管理字形库，内置 17 种 Unicode 线条字形，并处理字体栅格化逻辑。
- `topoglyph-vectomancy`：适配层，桥接 `vectomancy-geometry` 和 `vectomancy-raster` 作为数据源，网格切割前执行 RDP 简化和 Chaikin 平滑处理。
- `topoglyph-output`：渲染器，导出纯文本、ANSI 真彩色、HTML、Debug SVG、JSON Debug 编码器，以及 `.tglyph` 帧差分文本动画格式。
- `topoglyph-video`：通过 FFmpeg 把视频文件转换为 `.tglyph` 文本动画。
- `topoglyph-cli`：命令行程序入口。

## 许可证

本项目采用 MIT 许可证。