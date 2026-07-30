use crate::geometry::{CellMask, PortMask, CellDescriptor};
use smallvec::SmallVec;

/// Attributes of a pre-rasterized glyph from an Atlas.
pub struct GlyphDescriptor {
    pub token: String,
    pub cell_width: u8,
    pub mask: CellMask,
    pub ports: PortMask,
    pub orientation: [f32; 8],
    pub density: f32,
    pub centroid: [f32; 2],
    pub curvature: f32,
    pub stroke_count: u8,
}

/// Weights used to calculate the score when matching a Cell to a Glyph.
pub struct MatchWeights {
    pub mask: f32,
    pub topology: f32,
    pub orientation: f32,
    pub density: f32,
    pub centroid: f32,
    pub curvature: f32,
    pub complexity: f32,
}

impl Default for MatchWeights {
    fn default() -> Self {
        // Default Line Art preset
        Self {
            mask: 1.0,
            topology: 2.0,
            orientation: 0.5,
            density: 0.2,
            centroid: 0.5,
            curvature: 0.2,
            complexity: 0.1,
        }
    }
}

/// Represents a potential matched glyph for a specific cell.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub glyph_index: usize,
    pub local_score: f32,
}
