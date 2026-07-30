# TopoGlyph

**Topology + Glyph：将图像的几何拓扑量化成文字 Glyph。**

TopoGlyph 是 Vectomancy 生态下的一款高性能数学引擎。它并非传统的“按像素亮度映射 ASCII”的转换器，而是一个基于几何拓扑映射的全新排版系统。它通过对矢量或光栅图像进行深度的子像素提取、将轮廓裁切（Clipping）为 16×32 的高分辨率 Bit Mask，然后与字体图集 (Glyph Atlas) 进行异或距离（XOR Distance）和端口连接（Port Hamming Distance）比对，找到结构上最匹配的字符，最终重建出带有 ANSI 真彩色的纯文本图像。

## 核心架构设计

项目采用多 Crate Workspace 结构，划分为：

- `topoglyph-core`: 定义底层的 16×32 `CellMask`，8 端口 `PortMask`，基于异或（XOR）距离的超高速匹配器，以及基于 DDA / Liang-Barsky 的子网格裁剪算法。
- `topoglyph-atlas`: 管理字体字形库，将字形预光栅化为 Mask 以供搜索（0.1.0 MVP 版本目前内置了极简的 9 种 Unicode 线条，后续版本将支持真实字体）。
- `topoglyph-vectomancy`: 适配层，桥接 `vectomancy-geometry` 和 `vectomancy-raster`，提供输入数据源，将 PNG/JPG 和矢量多边形解析为标准的 `PolylineScene`。
- `topoglyph-output`: 渲染器，包含 ANSI Truecolor 文本输出等能力。
- `topoglyph-cli`: 命令行入口。

## 如何运行

可以使用 CLI 将一张图片转换为包含了 ANSI 终端转义序列的文本图像：

```bash
cargo run -p topoglyph-cli -- /path/to/image.png > output.txt
```

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

## 路线图 (Roadmap)
查看上级目录 `topoglyph-docs/TODO.md` 获取最新的开发计划！
