use topoglyph_core::matching::GlyphDescriptor;
use std::collections::HashMap;
use topoglyph_core::geometry::{PortMask, CellMask};

pub struct AtlasOptions {
    pub mask_width: u8,
    pub mask_height: u8,
    pub normalize_scale: bool,
    pub normalize_position: bool,
    pub include_whitespace: bool,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            mask_width: 16,
            mask_height: 32,
            normalize_scale: true,
            normalize_position: true,
            include_whitespace: false,
        }
    }
}

pub struct GlyphIndex {
    pub by_ports: HashMap<PortMask, Vec<usize>>,
    pub by_density: [Vec<usize>; 8],
    pub by_cell_width: HashMap<u8, Vec<usize>>,
}

pub struct GlyphAtlas {
    pub font_id: String,
    pub glyphs: Vec<GlyphDescriptor>,
    pub index: GlyphIndex,
}

impl GlyphAtlas {
    pub fn from_text(_text: &str, _options: &AtlasOptions) -> Result<Self, String> {
        // Build a minimal built-in line atlas for testing MVP
        let mut glyphs = Vec::new();

        // Macro to easily add lines
        macro_rules! add_glyph {
            ($token:expr, $ports:expr, $draw_fn:expr) => {
                let mut mask = CellMask::new();
                $draw_fn(&mut mask);
                glyphs.push(GlyphDescriptor {
                    token: $token.to_string(),
                    cell_width: 1,
                    mask,
                    ports: $ports,
                    orientation: [0.0; 8],
                    density: 0.1,
                    centroid: [0.0; 2],
                    curvature: 0.0,
                    stroke_count: 1,
                });
            };
        }

        let hw = 8isize; // half width (16/2)
        let hh = 16isize; // half height (32/2)

        // "─" Horizontal
        add_glyph!("─", PortMask::W | PortMask::E, |m: &mut CellMask| {
            draw_line_mask(0, hh, 15, hh, m);
        });

        // "│" Vertical
        add_glyph!("│", PortMask::N | PortMask::S, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, 31, m);
        });

        // "╱"
        add_glyph!("╱", PortMask::SW | PortMask::NE, |m: &mut CellMask| {
            draw_line_mask(0, 31, 15, 0, m);
        });

        // "╲"
        add_glyph!("╲", PortMask::NW | PortMask::SE, |m: &mut CellMask| {
            draw_line_mask(0, 0, 15, 31, m);
        });

        // "╭"
        add_glyph!("╭", PortMask::S | PortMask::E, |m: &mut CellMask| {
            draw_line_mask(hw, 31, hw, hh, m);
            draw_line_mask(hw, hh, 15, hh, m);
        });

        // "╮"
        add_glyph!("╮", PortMask::S | PortMask::W, |m: &mut CellMask| {
            draw_line_mask(hw, 31, hw, hh, m);
            draw_line_mask(hw, hh, 0, hh, m);
        });

        // "╰"
        add_glyph!("╰", PortMask::N | PortMask::E, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, hh, m);
            draw_line_mask(hw, hh, 15, hh, m);
        });

        // "╯"
        add_glyph!("╯", PortMask::N | PortMask::W, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, hh, m);
            draw_line_mask(hw, hh, 0, hh, m);
        });

        // "┼"
        add_glyph!("┼", PortMask::N | PortMask::S | PortMask::E | PortMask::W, |m: &mut CellMask| {
            draw_line_mask(0, hh, 15, hh, m);
            draw_line_mask(hw, 0, hw, 31, m);
        });
        
        let index = GlyphIndex {
            by_ports: HashMap::new(),
            by_density: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            by_cell_width: HashMap::new(),
        };

        Ok(Self {
            font_id: "builtin_lines".to_string(),
            glyphs,
            index,
        })
    }
}

// Simple Bresenham to build the built-in atlas masks
fn draw_line_mask(mut x0: isize, mut y0: isize, x1: isize, y1: isize, mask: &mut CellMask) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && x0 < 16 && y0 >= 0 && y0 < 32 {
            let bit_idx = (y0 * 16 + x0) as usize;
            mask.words[bit_idx / 64] |= 1 << (bit_idx % 64);
        }
        if x0 == x1 && y0 == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
