use topoglyph_core::matching::GlyphDescriptor;
use std::collections::HashMap;
use topoglyph_core::geometry::PortMask;

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
    pub fn from_text(text: &str, options: &AtlasOptions) -> Result<Self, String> {
        // TODO: Implement grapheme cluster iteration and rasterization
        Err("Not yet implemented".to_string())
    }
}
