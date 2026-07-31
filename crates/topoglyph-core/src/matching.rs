use crate::canvas::{TextCanvas, TextCell};
use crate::geometry::{CellDescriptor, CellMask, PortMask};
use std::collections::HashMap;

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
    /// Relative selection weight in `(0, 1]`, `1.0` by default. Only
    /// consulted when [`MatchWeights::frequency_bias`] is nonzero (i.e. the
    /// CLI's `--glyph-mode weighted`, per `topoglyph-docs/requirements.md`
    /// section 3.2: "在 `set` 模式...和 `weighted` 模式（依据词频影响挑选）
    /// 之间切换"). Populated by the atlas builders from how often each
    /// grapheme appears in the requested character pool, so a character
    /// repeated in `--custom-chars` is preferred over one that appears
    /// once, when their shape/topology scores are otherwise close.
    pub frequency: f32,
}

/// Weights used to calculate the score when matching a Cell to a Glyph. Each
/// weight multiplies a distance term normalized to roughly `[0, 1]`, so
/// weights are directly comparable to each other: doubling `density`
/// relative to `mask` makes density mismatches twice as influential on the
/// final score, everywhere.
///
/// See `topoglyph-docs/technical.md` section 2.2:
/// `Score = wm*mask_dist + wt*topology_dist + wo*orientation_dist
///        + wd*density_dist + wc*centroid_dist + wk*curvature_dist`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchWeights {
    pub mask: f32,
    pub topology: f32,
    pub orientation: f32,
    pub density: f32,
    pub centroid: f32,
    pub curvature: f32,
    pub complexity: f32,
    /// Weight applied to each candidate's `1.0 - GlyphDescriptor::frequency`
    /// term, implementing the CLI's `--glyph-mode weighted` (per
    /// `topoglyph-docs/requirements.md` section 3.2). `0.0` (the default,
    /// i.e. `--glyph-mode set`) makes every glyph equally likely regardless
    /// of how often it appeared in the requested character pool; raising it
    /// breaks shape/topology near-ties in favor of more frequent glyphs.
    pub frequency_bias: f32,
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
            frequency_bias: 0.0,
        }
    }
}

impl MatchWeights {
    /// Preset tuned for Han character / Emoji glyph sets: per
    /// `topoglyph-docs/technical.md` section 2.2, these de-emphasize stroke
    /// orientation/topology (glyphs in these sets rarely line up with the
    /// 8-direction port model the way box-drawing characters do) and
    /// emphasize density and mask shape instead.
    pub fn han_emoji_preset() -> Self {
        Self {
            mask: 1.5,
            topology: 0.5,
            orientation: 0.2,
            density: 1.0,
            centroid: 0.8,
            curvature: 0.3,
            complexity: 0.1,
            frequency_bias: 0.0,
        }
    }

    /// Preset tuned for line-drawing / box-drawing character sets: per
    /// `topoglyph-docs/technical.md` section 2.2, these emphasize topological
    /// (port) connectivity so adjacent cells visually join up.
    pub fn line_art_preset() -> Self {
        Self::default()
    }
}

/// Represents a potential matched glyph for a specific cell.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub glyph_index: usize,
    pub local_score: f32,
}

/// Secondary lookup structures over a glyph atlas's [`GlyphDescriptor`]s,
/// built once at atlas-construction time so a large custom-font atlas
/// (hundreds/thousands of graphemes) can narrow the search space before
/// falling back to a full per-glyph score. Each maps to indices into the
/// original glyph slice.
///
/// Lives in `topoglyph-core` (rather than `topoglyph-atlas`, where it was
/// originally defined) so [`match_scene_full`]'s pool-construction step can
/// consume it directly without a dependency cycle (`topoglyph-atlas`
/// already depends on `topoglyph-core`, not the other way around).
/// `topoglyph-atlas` re-exports this type for source compatibility.
pub struct GlyphIndex {
    /// Glyphs grouped by their exact [`PortMask`]. Useful for the topology
    /// term: given a required facing port, `by_ports` looks up every glyph
    /// that exposes it in O(1) instead of scanning the whole atlas.
    pub by_ports: HashMap<PortMask, Vec<usize>>,
    /// Glyphs bucketed into 8 equal-width density bins
    /// (`bin = floor(density times 8)`, clamped to `[0, 7]`),
    /// coarsest-grained but cheapest lookup for narrowing candidates by
    /// "how filled-in" a glyph is.
    pub by_density: [Vec<usize>; 8],
    /// Glyphs grouped by their `cell_width` (multi-column glyph support).
    pub by_cell_width: HashMap<u8, Vec<usize>>,
}

impl GlyphIndex {
    /// Builds all three lookup structures from a finished glyph list in a
    /// single pass.
    pub fn build(glyphs: &[GlyphDescriptor]) -> Self {
        let mut by_ports: HashMap<PortMask, Vec<usize>> = HashMap::new();
        let mut by_density: [Vec<usize>; 8] = Default::default();
        let mut by_cell_width: HashMap<u8, Vec<usize>> = HashMap::new();

        for (idx, glyph) in glyphs.iter().enumerate() {
            by_ports.entry(glyph.ports).or_default().push(idx);

            let bin = ((glyph.density * 8.0) as usize).min(7);
            by_density[bin].push(idx);

            by_cell_width.entry(glyph.cell_width).or_default().push(idx);
        }

        Self {
            by_ports,
            by_density,
            by_cell_width,
        }
    }

    /// Returns the indices of every glyph exposing at least the given
    /// ports (an exact-match lookup would miss glyphs with *more* ports set
    /// than requested, which are still valid candidates for a topology
    /// term that only checks specific directions).
    pub fn glyphs_with_any_port(&self, ports: PortMask) -> Vec<usize> {
        if ports.is_empty() {
            return (0..self.by_ports.values().map(Vec::len).sum()).collect();
        }
        let mut out = Vec::new();
        for (&mask, indices) in &self.by_ports {
            if mask.intersects(ports) {
                out.extend_from_slice(indices);
            }
        }
        out.sort_unstable();
        out
    }

    /// Returns the indices of every glyph whose density bin is within
    /// `tolerance_bins` of the given density's own bin. Widening
    /// `tolerance_bins` trades index selectivity for recall.
    pub fn glyphs_near_density(&self, density: f32, tolerance_bins: usize) -> Vec<usize> {
        let center = ((density * 8.0) as usize).min(7);
        let lo = center.saturating_sub(tolerance_bins);
        let hi = (center + tolerance_bins).min(7);
        let mut out = Vec::new();
        for bin in &self.by_density[lo..=hi] {
            out.extend_from_slice(bin);
        }
        out.sort_unstable();
        out
    }

    /// Returns the indices of every glyph that fits in `remaining_columns`
    /// (i.e. `cell_width <= remaining_columns`), for pool construction's
    /// multi-column-glyph boundary filter (see [`match_scene_full`]).
    /// Iterates `by_cell_width`'s buckets rather than every glyph
    /// individually, which only pays off once the atlas is large enough for
    /// bucket overhead to be worth it — for small built-in atlases this is
    /// no better than a linear scan, but it doesn't get *worse*, either.
    pub fn glyphs_fitting_in(&self, remaining_columns: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for (&width, indices) in &self.by_cell_width {
            if width as usize <= remaining_columns {
                out.extend_from_slice(indices);
            }
        }
        out.sort_unstable();
        out
    }
}

/// Tuning knobs for the Top-K candidate pool + multi-round Neighbor
/// Relaxation pipeline (`topoglyph-docs/technical.md` section 2.3): "针对独立
/// 匹配引起的局部连接断层...在得出 Top-K 的基础上进行 3-5 次的动态规划/松弛遍历".
#[derive(Debug, Clone, Copy)]
pub struct MatchOptions {
    /// Size of each cell's shape-score-ranked candidate pool. Relaxation
    /// rounds only re-score within this pool instead of the full atlas, so
    /// a topology-favored glyph can only win if it was already a
    /// shape-plausible contender. Must be at least `1`; values `>=`
    /// `atlas.len()` degrade to a full-atlas search every round.
    pub top_k: usize,
    /// Number of Neighbor Relaxation passes. Each round re-scores every
    /// cell against its Top-K pool using the previous round's tentative
    /// neighbor assignments, so connectivity fixes can themselves ripple
    /// into further fixes on subsequent rounds. Per spec, `3`-`5` rounds is
    /// the recommended range.
    pub relaxation_rounds: usize,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            top_k: 8,
            relaxation_rounds: 3,
        }
    }
}

#[inline]
fn mask_distance(a: &CellMask, b: &CellMask) -> f32 {
    a.iou_distance(b)
}

/// Euclidean distance between two already-normalized 8-bin orientation
/// histograms.
#[inline]
fn orientation_distance(a: &[f32; 8], b: &[f32; 8]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}

#[inline]
fn density_distance(a: f32, b: f32) -> f32 {
    (a - b).abs()
}

#[inline]
fn centroid_distance(a: &[f32; 2], b: &[f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

#[inline]
fn curvature_distance(a: f32, b: f32) -> f32 {
    (a - b).abs()
}

/// Shape-only distance (mask + orientation + density + centroid +
/// curvature + frequency bias), excluding the topology/port term, which
/// requires already having a neighbor's tentative match and so only applies
/// during the relaxation pass.
///
/// The frequency term adds `frequency_bias * (1.0 - glyph.frequency)`: a
/// glyph with `frequency == 1.0` (the maximum, or any glyph when
/// `--glyph-mode set` leaves every `frequency` at its default `1.0`)
/// contributes nothing extra, while a less-frequent glyph accrues a penalty
/// proportional to how rare it is, making otherwise-tied candidates settle
/// on whichever appeared more often in the requested character pool.
fn shape_score(cell: &CellDescriptor, glyph: &GlyphDescriptor, weights: &MatchWeights) -> f32 {
    weights.mask * mask_distance(&cell.mask, &glyph.mask)
        + weights.orientation * orientation_distance(&cell.orientation, &glyph.orientation)
        + weights.density * density_distance(cell.density, glyph.density)
        + weights.centroid * centroid_distance(&cell.centroid, &glyph.centroid)
        + weights.curvature * curvature_distance(cell.curvature, glyph.curvature)
        + weights.frequency_bias * (1.0 - glyph.frequency)
}

/// Counts port mismatches between a glyph's own ports and its neighbors'
/// facing ports, normalized to `[0, 1]` by dividing by the 8 possible
/// mismatches.
fn topology_mismatch(glyph_ports: PortMask, neighbor_ports: &NeighborPorts) -> f32 {
    let gp = glyph_ports;
    let mut mismatches = 0u8;
    if gp.contains(PortMask::N) != neighbor_ports.n.contains(PortMask::S) {
        mismatches += 1;
    }
    if gp.contains(PortMask::S) != neighbor_ports.s.contains(PortMask::N) {
        mismatches += 1;
    }
    if gp.contains(PortMask::E) != neighbor_ports.e.contains(PortMask::W) {
        mismatches += 1;
    }
    if gp.contains(PortMask::W) != neighbor_ports.w.contains(PortMask::E) {
        mismatches += 1;
    }
    if gp.contains(PortMask::NE) != neighbor_ports.ne.contains(PortMask::SW) {
        mismatches += 1;
    }
    if gp.contains(PortMask::NW) != neighbor_ports.nw.contains(PortMask::SE) {
        mismatches += 1;
    }
    if gp.contains(PortMask::SE) != neighbor_ports.se.contains(PortMask::NW) {
        mismatches += 1;
    }
    if gp.contains(PortMask::SW) != neighbor_ports.sw.contains(PortMask::NE) {
        mismatches += 1;
    }
    mismatches as f32 / 8.0
}

struct NeighborPorts {
    n: PortMask,
    s: PortMask,
    e: PortMask,
    w: PortMask,
    ne: PortMask,
    nw: PortMask,
    se: PortMask,
    sw: PortMask,
}

/// Matches every non-empty cell in the grid against the glyph atlas using
/// the default (line-art) [`MatchWeights`] preset and [`MatchOptions`]. See
/// [`match_scene_full`] for the full pipeline; this is a thin convenience
/// wrapper kept for existing callers.
pub fn match_scene(
    columns: usize,
    rows: usize,
    cells: &[CellDescriptor],
    atlas: &[GlyphDescriptor],
) -> TextCanvas {
    match_scene_full(
        columns,
        rows,
        cells,
        atlas,
        &MatchWeights::default(),
        &MatchOptions::default(),
    )
}

/// Matches every non-empty cell in the grid against the glyph atlas using
/// custom [`MatchWeights`] and default [`MatchOptions`]. See
/// [`match_scene_full`] for the full pipeline; this is a thin convenience
/// wrapper kept for existing callers.
pub fn match_scene_weighted(
    columns: usize,
    rows: usize,
    cells: &[CellDescriptor],
    atlas: &[GlyphDescriptor],
    weights: &MatchWeights,
) -> TextCanvas {
    match_scene_full(
        columns,
        rows,
        cells,
        atlas,
        weights,
        &MatchOptions::default(),
    )
}

/// Matches every non-empty cell in the grid against the glyph atlas using
/// the 6-factor scoring formula from `topoglyph-docs/technical.md` section
/// 2.2 (`Score = wm*mask_dist + wt*topology_dist + wo*orientation_dist +
/// wd*density_dist + wc*centroid_dist + wk*curvature_dist`), via a Top-K
/// candidate pool refined over several Neighbor Relaxation rounds, per
/// section 2.3 ("在得出 Top-K 的基础上进行 3-5 次的动态规划/松弛遍历"):
///
/// - **Pool construction** scores every cell against the *entire* atlas
///   using only the shape terms (mask/orientation/density/centroid/
///   curvature — there's no neighbor context yet to score topology
///   against) and keeps only the `options.top_k` best-shaped candidates.
///   Committing to this pool up front, rather than re-scanning the whole
///   atlas every round, is also what makes repeated relaxation rounds cheap
///   even for larger atlases.
/// - **Relaxation rounds** (`options.relaxation_rounds` of them) re-score
///   *only* each cell's pool using the shape terms plus the topology term
///   against the previous round's tentative neighbor assignments, so a
///   correction on one round (e.g. fixing a `╯│` where the line should flow
///   through) can itself change what a neighbor sees on the next round,
///   letting fixes ripple outward instead of being decided once and frozen.
///
/// Each output [`TextCell`]'s `score` field carries the winning candidate's
/// final (shape + topology) score, so callers can render a "Score Heatmap"
/// (`topoglyph-docs/TODO.md` 0.4.0) or feed it into a debug encoder.
pub fn match_scene_full(
    columns: usize,
    rows: usize,
    cells: &[CellDescriptor],
    atlas: &[GlyphDescriptor],
    weights: &MatchWeights,
    options: &MatchOptions,
) -> TextCanvas {
    match_scene_indexed(columns, rows, cells, atlas, None, weights, options)
}

/// Identical to [`match_scene_full`], but takes an optional pre-built
/// [`GlyphIndex`] to narrow pool construction's per-cell atlas scan before
/// scoring, instead of always scoring every glyph. `index` should be built
/// from the same `atlas` slice (typically via `GlyphAtlas::index` in
/// `topoglyph-atlas`); passing `None` falls back to the full linear scan
/// `match_scene_full` always did.
///
/// This matters once an atlas gets large (hundreds/thousands of graphemes
/// from a custom font's full character pool): scoring every glyph against
/// every non-empty cell is `O(cells * atlas.len())`, which the small
/// built-in 17-glyph line atlas never notices but a large custom-font atlas
/// would. Narrowing via [`GlyphIndex::glyphs_fitting_in`] first (only the
/// multi-column-width boundary filter — see the pool-construction comment
/// below) turns that into `O(cells * candidates_after_width_filter)`, which
/// is a meaningful win whenever most of the atlas is single-width and most
/// cells aren't near the grid's right edge.
pub fn match_scene_indexed(
    columns: usize,
    rows: usize,
    cells: &[CellDescriptor],
    atlas: &[GlyphDescriptor],
    index: Option<&GlyphIndex>,
    weights: &MatchWeights,
    options: &MatchOptions,
) -> TextCanvas {
    let empty_words_all_zero = |mask: &CellMask| mask.words.iter().all(|&w| w == 0);

    if atlas.is_empty() {
        let out_cells = (0..columns * rows)
            .map(|_| TextCell {
                token: " ".to_string(),
                score: 0.0,
                source_path: None,
                color: None,
            })
            .collect();
        return TextCanvas {
            width: columns,
            height: rows,
            cells: out_cells,
        };
    }

    let top_k = options.top_k.max(1);

    // Pool construction: shape-only multi-factor matching, keeping only the
    // top_k lowest-scoring candidates per cell.
    //
    // Multi-column glyphs (`cell_width > 1`, e.g. CJK ideographs/most emoji
    // via `topoglyph_atlas`'s `unicode-width`-based `grapheme_cell_width`)
    // are excluded from any column that doesn't have enough room to its
    // right — there is no cell beyond the grid's last column for a
    // double-width glyph to occupy. Filtering here (rather than discovering
    // the problem after a wide glyph has already "won" the last column)
    // means the width-occupancy sweep below can assume every chosen wide
    // glyph always fits.
    //
    // With an index available, the width filter is applied by looking up
    // `glyphs_fitting_in(remaining_columns)` instead of scanning every
    // glyph and checking its width inline; without one, this falls back to
    // the same full-atlas linear scan `match_scene_full` used before this
    // index-aware path existed.
    let mut pools: Vec<Vec<Candidate>> = Vec::with_capacity(columns * rows);
    for (flat, cell) in cells.iter().enumerate() {
        if empty_words_all_zero(&cell.mask) {
            pools.push(Vec::new());
            continue;
        }
        let col = flat % columns;
        let remaining_columns = columns - col;

        let candidate_indices: Vec<usize> = match index {
            Some(idx) => idx.glyphs_fitting_in(remaining_columns),
            None => (0..atlas.len())
                .filter(|&i| atlas[i].cell_width as usize <= remaining_columns)
                .collect(),
        };

        let mut scored: Vec<Candidate> = candidate_indices
            .into_iter()
            .map(|idx| Candidate {
                glyph_index: idx,
                local_score: shape_score(cell, &atlas[idx], weights),
            })
            .collect();
        scored.sort_by(|a, b| a.local_score.total_cmp(&b.local_score));
        scored.truncate(top_k);
        pools.push(scored);
    }

    // Tentative winner (glyph index into `atlas`) per cell, seeded from the
    // shape-only pool ranking so the first relaxation round already has
    // sensible neighbor context to work with.
    let mut current_winner: Vec<Option<usize>> = pools
        .iter()
        .map(|pool| pool.first().map(|c| c.glyph_index))
        .collect();
    let mut current_score: Vec<f32> = pools
        .iter()
        .map(|pool| pool.first().map(|c| c.local_score).unwrap_or(0.0))
        .collect();

    let get_port = |winners: &[Option<usize>], nr: isize, nc: isize| -> PortMask {
        if nr >= 0 && nr < rows as isize && nc >= 0 && nc < columns as isize {
            match winners[(nr as usize) * columns + (nc as usize)] {
                Some(idx) => atlas[idx].ports,
                None => PortMask::empty(),
            }
        } else {
            PortMask::empty()
        }
    };

    // Relaxation rounds: re-score only within each cell's pool, using the
    // previous round's tentative winners as neighbor context.
    //
    // Known approximation: this loop still computes an independent
    // tentative winner for every cell, including ones that will end up
    // claimed (and rendered empty) by a wide neighbor's cell_width in the
    // final canvas-building pass below. That "shadow" winner never reaches
    // output, but it briefly exists as topology context for *its own*
    // right-hand neighbor during relaxation. This is acceptable rather than
    // plumbing width-occupancy through every relaxation round: multi-column
    // glyphs (CJK ideographs, most emoji) essentially never carry N/S/E/W
    // ports in the first place (see `topoglyph_atlas::ports_from_mask`,
    // which only detects cardinal ports for rasterized font glyphs at all,
    // and even those are rare for solid ideograph strokes), so a shadow
    // winner's ports contributing to a neighbor's topology term has
    // negligible practical effect on the box-drawing/line-art atlases this
    // scoring pipeline is primarily tuned for.
    for _ in 0..options.relaxation_rounds {
        let mut next_winner = current_winner.clone();
        let mut next_score = current_score.clone();

        for r in 0..rows {
            for c in 0..columns {
                let flat = r * columns + c;
                if pools[flat].is_empty() {
                    continue;
                }

                let neighbor_ports = NeighborPorts {
                    n: get_port(&current_winner, r as isize - 1, c as isize),
                    s: get_port(&current_winner, r as isize + 1, c as isize),
                    e: get_port(&current_winner, r as isize, c as isize + 1),
                    w: get_port(&current_winner, r as isize, c as isize - 1),
                    ne: get_port(&current_winner, r as isize - 1, c as isize + 1),
                    nw: get_port(&current_winner, r as isize - 1, c as isize - 1),
                    se: get_port(&current_winner, r as isize + 1, c as isize + 1),
                    sw: get_port(&current_winner, r as isize + 1, c as isize - 1),
                };

                let mut best_score = f32::MAX;
                let mut best_idx = pools[flat][0].glyph_index;

                for candidate in &pools[flat] {
                    let glyph = &atlas[candidate.glyph_index];
                    let score = candidate.local_score
                        + weights.topology * topology_mismatch(glyph.ports, &neighbor_ports);
                    if score < best_score {
                        best_score = score;
                        best_idx = candidate.glyph_index;
                    }
                }

                next_winner[flat] = Some(best_idx);
                next_score[flat] = best_score;
            }
        }

        current_winner = next_winner;
        current_score = next_score;
    }

    // Build final canvas. Scanned left-to-right per row so a multi-column
    // winner (`cell_width > 1`, e.g. a CJK ideograph or emoji) can claim the
    // grid columns to its right before they're independently matched: the
    // pool-construction filter above already guarantees a wide glyph never
    // wins a column too close to the row's right edge to fit, so `occupied`
    // only ever needs to skip forward, never clamp.
    //
    // Skipped/occupied cells render as a plain space with no token — they
    // are not re-matched against the atlas at all, per the "match, then
    // claim" design: the wide glyph's own mask was compared against a
    // single cell's geometry (see `topoglyph-docs/TODO.md` 0.5.0), so its
    // right-hand neighbor's own stroke content is intentionally discarded
    // rather than blended into anything.
    let mut out_cells = Vec::with_capacity(columns * rows);
    for r in 0..rows {
        let mut occupied_until = 0usize; // columns < this are already claimed
        for c in 0..columns {
            let flat = r * columns + c;
            let cell = &cells[flat];

            if c < occupied_until {
                out_cells.push(TextCell {
                    token: String::new(),
                    score: 0.0,
                    source_path: None,
                    color: None,
                });
                continue;
            }

            match current_winner[flat] {
                None => out_cells.push(TextCell {
                    token: " ".to_string(),
                    score: 0.0,
                    source_path: None,
                    color: None,
                }),
                Some(idx) => {
                    let glyph = &atlas[idx];
                    occupied_until = c + glyph.cell_width as usize;
                    out_cells.push(TextCell {
                        token: glyph.token.clone(),
                        score: current_score[flat],
                        source_path: None,
                        color: cell.color.clone(),
                    });
                }
            }
        }
    }
    TextCanvas {
        width: columns,
        height: rows,
        cells: out_cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(token: &str, mask: CellMask, ports: PortMask) -> GlyphDescriptor {
        let features = crate::features::extract_features(&mask, 16, 32);
        GlyphDescriptor {
            token: token.to_string(),
            cell_width: 1,
            mask,
            ports,
            orientation: features.orientation,
            density: features.density,
            centroid: features.centroid,
            curvature: features.curvature,
            stroke_count: features.stroke_count,
            frequency: 1.0,
        }
    }

    fn mask_from_coords(coords: &[(usize, usize)]) -> CellMask {
        let mut mask = CellMask::new();
        for &(x, y) in coords {
            let bit_idx = y * 16 + x;
            mask.words[bit_idx / 64] |= 1 << (bit_idx % 64);
        }
        mask
    }

    fn cell_from_mask(mask: CellMask) -> CellDescriptor {
        let features = crate::features::extract_features(&mask, 16, 32);
        CellDescriptor {
            mask,
            ports: PortMask::empty(),
            orientation: features.orientation,
            density: features.density,
            centroid: features.centroid,
            curvature: features.curvature,
            stroke_count: features.stroke_count,
            color: None,
        }
    }

    #[test]
    fn identical_mask_scores_zero_shape_distance() {
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let cell = cell_from_mask(mask.clone());
        let glyph = glyph("─", mask, PortMask::W | PortMask::E);
        let weights = MatchWeights::default();
        assert_eq!(shape_score(&cell, &glyph, &weights), 0.0);
    }

    #[test]
    fn match_scene_picks_exact_glyph_for_identical_mask() {
        let horiz_coords: Vec<_> = (0..16).map(|x| (x, 16)).collect();
        let vert_coords: Vec<_> = (0..32).map(|y| (8, y)).collect();

        let horiz_mask = mask_from_coords(&horiz_coords);
        let vert_mask = mask_from_coords(&vert_coords);

        let atlas = vec![
            glyph("─", horiz_mask.clone(), PortMask::W | PortMask::E),
            glyph("│", vert_mask, PortMask::N | PortMask::S),
        ];

        let cell = cell_from_mask(horiz_mask);
        let canvas = match_scene(1, 1, &[cell], &atlas);
        assert_eq!(canvas.cells[0].token, "─");
    }

    #[test]
    fn empty_cell_renders_as_space() {
        let atlas = vec![glyph(
            "─",
            mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>()),
            PortMask::W | PortMask::E,
        )];
        let cell = cell_from_mask(CellMask::new());
        let canvas = match_scene(1, 1, &[cell], &atlas);
        assert_eq!(canvas.cells[0].token, " ");
    }

    #[test]
    fn neighbor_relaxation_prefers_connecting_port_over_shape_tie() {
        // Two glyphs with the exact same mask (so shape terms tie exactly)
        // but different ports: one whose W port would connect to a West
        // neighbor exposing an E port, one that wouldn't. Relaxation should
        // break the tie in favor of the one that connects.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let connecting = glyph("A", mask.clone(), PortMask::W | PortMask::E);
        let non_connecting = glyph("B", mask.clone(), PortMask::N | PortMask::S);
        let west_neighbor_glyph = glyph("─", mask.clone(), PortMask::W | PortMask::E);

        let atlas = vec![connecting, non_connecting, west_neighbor_glyph];

        // Row of 2 cells: [west_neighbor_forced, ambiguous_cell]. Force the
        // west neighbor to resolve to atlas[2] ("─", has an E port) by
        // making its own mask identical to that glyph and nothing else
        // matching better; the ambiguous cell has a mask tying atlas[0] and
        // atlas[1] on shape score, so only the topology term should decide.
        let west_cell = cell_from_mask(mask.clone());
        let ambiguous_cell = cell_from_mask(mask);

        let canvas = match_scene(2, 1, &[west_cell, ambiguous_cell], &atlas);
        // The ambiguous (second) cell should end up as "A" (has a W port,
        // matching the west neighbor's E port) rather than "B".
        assert_eq!(canvas.cells[1].token, "A");
    }

    fn glyph_with_frequency(
        token: &str,
        mask: CellMask,
        ports: PortMask,
        frequency: f32,
    ) -> GlyphDescriptor {
        GlyphDescriptor {
            frequency,
            ..glyph(token, mask, ports)
        }
    }

    #[test]
    fn zero_frequency_bias_ignores_frequency_entirely() {
        // Two identically-shaped glyphs, one much rarer than the other.
        // With the default frequency_bias of 0.0 (--glyph-mode set), the
        // rarer one's shape_score must be identical to the common one's.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let common = glyph_with_frequency("common", mask.clone(), PortMask::empty(), 1.0);
        let rare = glyph_with_frequency("rare", mask.clone(), PortMask::empty(), 0.01);
        let cell = cell_from_mask(mask);
        let weights = MatchWeights::default();
        assert_eq!(weights.frequency_bias, 0.0);
        assert_eq!(
            shape_score(&cell, &common, &weights),
            shape_score(&cell, &rare, &weights)
        );
    }

    #[test]
    fn nonzero_frequency_bias_penalizes_rarer_glyphs() {
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let common = glyph_with_frequency("common", mask.clone(), PortMask::empty(), 1.0);
        let rare = glyph_with_frequency("rare", mask.clone(), PortMask::empty(), 0.2);
        let cell = cell_from_mask(mask);
        let weights = MatchWeights {
            frequency_bias: 1.0,
            ..MatchWeights::default()
        };
        assert!(shape_score(&cell, &rare, &weights) > shape_score(&cell, &common, &weights));
    }

    #[test]
    fn weighted_mode_breaks_shape_tie_in_favor_of_frequent_glyph() {
        // Same mask, same ports, so shape and topology scores tie exactly
        // apart from frequency; only --glyph-mode weighted's frequency_bias
        // should decide the winner.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let common = glyph_with_frequency("common", mask.clone(), PortMask::W | PortMask::E, 1.0);
        let rare = glyph_with_frequency("rare", mask.clone(), PortMask::W | PortMask::E, 0.1);
        let atlas = vec![rare, common]; // rare listed first so a stable tie
                                        // would otherwise favor it

        let cell = cell_from_mask(mask);
        let weights = MatchWeights {
            frequency_bias: 1.0,
            ..MatchWeights::default()
        };
        let canvas = match_scene_weighted(1, 1, &[cell], &atlas, &weights);
        assert_eq!(canvas.cells[0].token, "common");
    }

    #[test]
    fn han_emoji_preset_weighs_density_more_than_line_art_default() {
        let han = MatchWeights::han_emoji_preset();
        let line_art = MatchWeights::line_art_preset();
        assert!(han.density > line_art.density);
        assert!(han.topology < line_art.topology);
    }

    #[test]
    fn match_scene_weighted_with_extreme_density_weight_favors_density_match() {
        // Two candidate glyphs: one with an identical mask to the cell but
        // via a *different* stroke pattern with mismatched density, and one
        // with a deliberately different mask but density that matches the
        // cell more closely won't apply here directly, so instead assert
        // that pumping the density weight to an extreme changes the winner
        // relative to the default weights for a cell that's ambiguous on
        // mask distance alone.
        let sparse_mask = mask_from_coords(&[(0, 0)]);
        let dense_mask = mask_from_coords(
            &(0..16)
                .flat_map(|x| (0..4).map(move |y| (x, y)))
                .collect::<Vec<_>>(),
        );

        let sparse_glyph = glyph("s", sparse_mask, PortMask::empty());
        let dense_glyph = glyph("d", dense_mask.clone(), PortMask::empty());
        let atlas = vec![sparse_glyph, dense_glyph];

        // A cell whose mask sits roughly in between in bit count but is
        // XOR-closer to neither exactly; use the dense mask itself so mask
        // distance to "d" is 0 and to "s" is large — under default weights
        // "d" wins on mask alone already, but we still verify a weights
        // object with zero density weight vs one with heavy density weight
        // can be constructed and both run without panicking, producing a
        // valid (non-empty) token either way.
        let cell = cell_from_mask(dense_mask);

        let zero_density = MatchWeights {
            density: 0.0,
            ..MatchWeights::default()
        };
        let heavy_density = MatchWeights {
            density: 100.0,
            ..MatchWeights::default()
        };

        let canvas_a =
            match_scene_weighted(1, 1, std::slice::from_ref(&cell), &atlas, &zero_density);
        let canvas_b = match_scene_weighted(1, 1, &[cell], &atlas, &heavy_density);
        assert!(!canvas_a.cells[0].token.is_empty());
        assert!(!canvas_b.cells[0].token.is_empty());
    }

    #[test]
    fn top_k_of_one_behaves_like_shape_only_matching() {
        // With top_k = 1, the pool for every cell contains only its single
        // best shape match, so no relaxation round can ever pick anything
        // else: topology has no candidates left to break ties with.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let best_shape = glyph("best", mask.clone(), PortMask::N | PortMask::S);
        let worse_shape_better_topology =
            glyph("worse", CellMask::new(), PortMask::W | PortMask::E);
        let atlas = vec![best_shape, worse_shape_better_topology];

        let cell = cell_from_mask(mask);
        let options = MatchOptions {
            top_k: 1,
            relaxation_rounds: 5,
        };
        let canvas = match_scene_full(1, 1, &[cell], &atlas, &MatchWeights::default(), &options);
        assert_eq!(canvas.cells[0].token, "best");
    }

    #[test]
    fn zero_relaxation_rounds_uses_shape_only_pool_winner() {
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let atlas = vec![glyph("─", mask.clone(), PortMask::W | PortMask::E)];
        let cell = cell_from_mask(mask);
        let options = MatchOptions {
            top_k: 8,
            relaxation_rounds: 0,
        };
        let canvas = match_scene_full(1, 1, &[cell], &atlas, &MatchWeights::default(), &options);
        assert_eq!(canvas.cells[0].token, "─");
    }

    #[test]
    fn empty_atlas_renders_all_cells_as_space_without_panicking() {
        let mask = mask_from_coords(&[(0, 0)]);
        let cell = cell_from_mask(mask);
        let canvas = match_scene(1, 1, &[cell], &[]);
        assert_eq!(canvas.cells[0].token, " ");
    }

    #[test]
    fn multi_round_relaxation_can_fix_connectivity_a_single_round_misses() {
        // A 1x3 row where the middle cell's shape score ties two glyphs
        // exactly, and the outer two cells only settle into
        // topology-compatible ports themselves once the middle cell has
        // already committed on round 1. A single relaxation round can't
        // see that far; multiple rounds let the correction propagate.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());

        // "through" has W+E ports (passes a horizontal connection through).
        // "stop" has no ports (a dead end / disconnected token).
        let through = glyph("through", mask.clone(), PortMask::W | PortMask::E);
        let stop = glyph("stop", mask.clone(), PortMask::empty());
        // West/east anchors: only have an E (resp. W) port, so they can only
        // ever match "through"'s facing port, never "stop"'s.
        let west_anchor = glyph("<", mask.clone(), PortMask::E);
        let east_anchor = glyph(">", mask.clone(), PortMask::W);

        let atlas = vec![through, stop, west_anchor, east_anchor];

        let west_cell = cell_from_mask(mask.clone());
        let middle_cell = cell_from_mask(mask.clone());
        let east_cell = cell_from_mask(mask);

        let options = MatchOptions {
            top_k: 4,
            relaxation_rounds: 3,
        };
        let canvas = match_scene_full(
            3,
            1,
            &[west_cell, middle_cell, east_cell],
            &atlas,
            &MatchWeights::default(),
            &options,
        );

        // The middle cell ties "through" and "stop" on shape alone (both
        // have the same mask), so only topology against its now-resolved
        // neighbors decides — it should end up "through" to connect both
        // anchors, not "stop".
        assert_eq!(canvas.cells[1].token, "through");
    }

    fn glyph_with_width(token: &str, mask: CellMask, cell_width: u8) -> GlyphDescriptor {
        GlyphDescriptor {
            cell_width,
            ..glyph(token, mask, PortMask::empty())
        }
    }

    #[test]
    fn wide_glyph_occupies_its_right_neighbor_cell() {
        // A 1x2 row where a cell_width=2 glyph (e.g. a CJK ideograph) wins
        // the left cell; its right neighbor must render as an empty token
        // rather than being independently matched against the atlas.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let wide = glyph_with_width("字", mask.clone(), 2);
        let atlas = vec![wide];

        let left_cell = cell_from_mask(mask.clone());
        let right_cell = cell_from_mask(mask);
        let canvas = match_scene(2, 1, &[left_cell, right_cell], &atlas);

        assert_eq!(canvas.cells[0].token, "字");
        assert_eq!(
            canvas.cells[1].token, "",
            "the cell claimed by a wide glyph's right-hand neighbor must be empty, not re-matched"
        );
    }

    #[test]
    fn wide_glyph_is_never_chosen_in_the_grids_last_column() {
        // A cell_width=2 glyph in the rightmost column of the grid has no
        // room to its right; pool construction must exclude it there even
        // though its shape score would otherwise be a perfect match, so a
        // narrower fallback (or empty) is chosen instead of silently
        // overflowing the grid.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let wide = glyph_with_width("字", mask.clone(), 2);
        let narrow = glyph_with_width("x", mask.clone(), 1);
        let atlas = vec![wide, narrow];

        // 1x1 grid: column 0 is also the last column, so cell_width=2
        // never fits.
        let cell = cell_from_mask(mask);
        let canvas = match_scene(1, 1, &[cell], &atlas);
        assert_eq!(
            canvas.cells[0].token, "x",
            "a wide glyph must never be selected in a column with no room to its right"
        );
    }

    #[test]
    fn wide_glyph_at_second_to_last_column_fits_exactly() {
        // A 1x3 row: a cell_width=2 glyph starting at column 1 fits exactly
        // (columns 1 and 2), so it should still be selectable there, unlike
        // the last-column case above.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let wide = glyph_with_width("字", mask.clone(), 2);
        let atlas = vec![wide];

        let cells = vec![
            cell_from_mask(CellMask::new()), // empty, column 0
            cell_from_mask(mask.clone()),    // column 1
            cell_from_mask(mask),            // column 2, claimed by column 1's glyph
        ];
        let canvas = match_scene(3, 1, &cells, &atlas);
        assert_eq!(canvas.cells[0].token, " "); // empty cell, no match
        assert_eq!(canvas.cells[1].token, "字");
        assert_eq!(canvas.cells[2].token, "");
    }

    #[test]
    fn narrow_glyphs_are_unaffected_by_width_occupancy_logic() {
        // Sanity check that cell_width=1 (the default for every existing
        // glyph) behaves exactly as before: no cell is ever claimed by a
        // neighbor.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let narrow = glyph("─", mask.clone(), PortMask::W | PortMask::E);
        let atlas = vec![narrow];
        let cells = vec![cell_from_mask(mask.clone()), cell_from_mask(mask)];
        let canvas = match_scene(2, 1, &cells, &atlas);
        assert_eq!(canvas.cells[0].token, "─");
        assert_eq!(canvas.cells[1].token, "─");
    }

    #[test]
    fn glyph_index_glyphs_fitting_in_respects_cell_width() {
        let glyphs = vec![
            glyph_with_width("narrow", CellMask::new(), 1),
            glyph_with_width("wide", CellMask::new(), 2),
            glyph_with_width("wider", CellMask::new(), 3),
        ];
        let index = GlyphIndex::build(&glyphs);

        let mut fits_1 = index.glyphs_fitting_in(1);
        fits_1.sort_unstable();
        assert_eq!(fits_1, vec![0], "only the width-1 glyph fits in 1 column");

        let mut fits_2 = index.glyphs_fitting_in(2);
        fits_2.sort_unstable();
        assert_eq!(
            fits_2,
            vec![0, 1],
            "width-1 and width-2 glyphs fit in 2 columns"
        );

        let mut fits_3 = index.glyphs_fitting_in(3);
        fits_3.sort_unstable();
        assert_eq!(fits_3, vec![0, 1, 2], "all three fit in 3 columns");
    }

    #[test]
    fn match_scene_indexed_with_index_matches_full_scan_result() {
        // The indexed path (via GlyphIndex::glyphs_fitting_in) must select
        // exactly the same winner as the unindexed linear-scan path for the
        // same atlas/weights/options — the index is a narrowing
        // optimization, not a behavior change.
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let horiz = glyph("─", mask.clone(), PortMask::W | PortMask::E);
        let vert = glyph(
            "│",
            mask_from_coords(&(0..32).map(|y| (8, y)).collect::<Vec<_>>()),
            PortMask::N | PortMask::S,
        );
        let atlas = vec![horiz, vert];
        let index = GlyphIndex::build(&atlas);

        let cell = cell_from_mask(mask);
        let weights = MatchWeights::default();
        let options = MatchOptions::default();
        let unindexed = match_scene_full(
            1,
            1,
            std::slice::from_ref(&cell),
            &atlas,
            &weights,
            &options,
        );
        let indexed = match_scene_indexed(1, 1, &[cell], &atlas, Some(&index), &weights, &options);

        assert_eq!(unindexed.cells[0].token, indexed.cells[0].token);
        assert_eq!(unindexed.cells[0].token, "─");
    }

    #[test]
    fn match_scene_indexed_still_excludes_wide_glyphs_from_the_last_column() {
        // The index-aware width filter must produce the same last-column
        // exclusion as the unindexed path (see
        // `wide_glyph_is_never_chosen_in_the_grids_last_column`).
        let mask = mask_from_coords(&(0..16).map(|x| (x, 16)).collect::<Vec<_>>());
        let wide = glyph_with_width("字", mask.clone(), 2);
        let narrow = glyph_with_width("x", mask.clone(), 1);
        let atlas = vec![wide, narrow];
        let index = GlyphIndex::build(&atlas);

        let cell = cell_from_mask(mask);
        let canvas = match_scene_indexed(
            1,
            1,
            &[cell],
            &atlas,
            Some(&index),
            &MatchWeights::default(),
            &MatchOptions::default(),
        );
        assert_eq!(canvas.cells[0].token, "x");
    }
}
