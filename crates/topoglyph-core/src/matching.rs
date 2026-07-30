use crate::geometry::{CellMask, PortMask, CellDescriptor};
use smallvec::SmallVec;
use crate::canvas::{TextCanvas, TextCell};

/// Attributes of a pre-rasterized glyph from an Atlas.
#[derive(Clone)]
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

pub fn match_scene(columns: usize, rows: usize, cells: &[CellDescriptor], atlas: &[GlyphDescriptor]) -> TextCanvas {
    let mut out_cells = Vec::with_capacity(cells.len());

    for cell in cells {
        // If empty cell, match with space
        if cell.mask.words.iter().all(|&w| w == 0) {
            out_cells.push(TextCell {
                token: " ".to_string(),
                score: 0.0,
                source_path: None,
                color: None,
            });
            continue;
        }

        let mut best_score = f32::MAX;
        let mut best_token = " ".to_string();

        for glyph in atlas {
            let mask_dist = cell.mask.xor_distance(&glyph.mask) as f32;
            // Simple port distance
            let cell_port_bits = cell.ports.bits();
            let glyph_port_bits = glyph.ports.bits();
            let port_dist = (cell_port_bits ^ glyph_port_bits).count_ones() as f32;

            // Basic score
            let score = mask_dist * 1.0 + port_dist * 5.0;

            if score < best_score {
                best_score = score;
                best_token = glyph.token.clone();
            }
        }

        out_cells.push(TextCell {
            token: best_token,
            score: best_score,
            source_path: None,
            color: cell.color.clone(),
        });
    }

    TextCanvas {
        width: columns,
        height: rows,
        cells: out_cells,
    }
}
