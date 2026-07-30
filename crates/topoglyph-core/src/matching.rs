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
    let mut best_indices = vec![0usize; columns * rows];
    let empty_words_all_zero = |mask: &CellMask| mask.words.iter().all(|&w| w == 0);

    // Pass 1: Pure XOR Distance matching
    for r in 0..rows {
        for c in 0..columns {
            let cell = &cells[r * columns + c];
            if empty_words_all_zero(&cell.mask) {
                continue;
            }

            let mut best_score = f32::MAX;
            let mut best_idx = 0;

            for (idx, glyph) in atlas.iter().enumerate() {
                let mut dist = 0;
                for i in 0..8 {
                    dist += (cell.mask.words[i] ^ glyph.mask.words[i]).count_ones();
                }
                let score = dist as f32;
                if score < best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }
            best_indices[r * columns + c] = best_idx;
        }
    }

    // Pass 2: Neighbor Relaxation (1 iteration)
    let relaxation_weight = 30.0; // Penalty for port mismatch
    let mut new_indices = best_indices.clone();

    for r in 0..rows {
        for c in 0..columns {
            let cell = &cells[r * columns + c];
            if empty_words_all_zero(&cell.mask) { continue; }

            // Look up neighbor ports
            let get_port = |nr: isize, nc: isize| -> PortMask {
                if nr >= 0 && nr < rows as isize && nc >= 0 && nc < columns as isize {
                    let n_idx = best_indices[(nr as usize) * columns + (nc as usize)];
                    if empty_words_all_zero(&cells[(nr as usize) * columns + (nc as usize)].mask) {
                        PortMask::empty()
                    } else {
                        atlas[n_idx].ports
                    }
                } else {
                    PortMask::empty()
                }
            };

            let port_n = get_port(r as isize - 1, c as isize);
            let port_s = get_port(r as isize + 1, c as isize);
            let port_e = get_port(r as isize, c as isize + 1);
            let port_w = get_port(r as isize, c as isize - 1);
            let port_ne = get_port(r as isize - 1, c as isize + 1);
            let port_nw = get_port(r as isize - 1, c as isize - 1);
            let port_se = get_port(r as isize + 1, c as isize + 1);
            let port_sw = get_port(r as isize + 1, c as isize - 1);

            let mut best_score = f32::MAX;
            let mut best_idx = 0;

            for (idx, glyph) in atlas.iter().enumerate() {
                let mut dist = 0;
                for i in 0..8 {
                    dist += (cell.mask.words[i] ^ glyph.mask.words[i]).count_ones();
                }
                
                // Calculate port mismatches
                let mut mismatches = 0;
                let gp = glyph.ports;
                if gp.contains(PortMask::N) != port_n.contains(PortMask::S) { mismatches += 1; }
                if gp.contains(PortMask::S) != port_s.contains(PortMask::N) { mismatches += 1; }
                if gp.contains(PortMask::E) != port_e.contains(PortMask::W) { mismatches += 1; }
                if gp.contains(PortMask::W) != port_w.contains(PortMask::E) { mismatches += 1; }
                if gp.contains(PortMask::NE) != port_ne.contains(PortMask::SW) { mismatches += 1; }
                if gp.contains(PortMask::NW) != port_nw.contains(PortMask::SE) { mismatches += 1; }
                if gp.contains(PortMask::SE) != port_se.contains(PortMask::NW) { mismatches += 1; }
                if gp.contains(PortMask::SW) != port_sw.contains(PortMask::NE) { mismatches += 1; }

                let score = dist as f32 + mismatches as f32 * relaxation_weight;
                if score < best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }
            new_indices[r * columns + c] = best_idx;
        }
    }

    // Build final canvas
    let mut out_cells = Vec::with_capacity(columns * rows);
    for r in 0..rows {
        for c in 0..columns {
            let cell = &cells[r * columns + c];
            if empty_words_all_zero(&cell.mask) {
                out_cells.push(TextCell {
                    token: " ".to_string(),
                    score: 0.0,
                    source_path: None,
                    color: None,
                });
            } else {
                let idx = new_indices[r * columns + c];
                out_cells.push(TextCell {
                    token: atlas[idx].token.clone(),
                    score: 0.0,
                    source_path: None,
                    color: cell.color.clone(),
                });
            }
        }
    }
    TextCanvas {
        width: columns,
        height: rows,
        cells: out_cells,
    }
}
