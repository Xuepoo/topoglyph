use crate::geometry::CellMask;

/// The 8 compass directions used both by [`crate::geometry::PortMask`] and by
/// the orientation histogram below, in the same order so bin `i` here lines
/// up conceptually with `PortMask` bit `i`.
const DIRS: [(i32, i32); 8] = [
    (0, -1),  // N
    (1, -1),  // NE
    (1, 0),   // E
    (1, 1),   // SE
    (0, 1),   // S
    (-1, 1),  // SW
    (-1, 0),  // W
    (-1, -1), // NW
];

/// Geometric features extracted from a rasterized [`CellMask`] bitmap.
///
/// These are computed identically for both `CellDescriptor` (subcell masks
/// produced by [`crate::clipping`] from actual stroke geometry) and
/// `GlyphDescriptor` (subcell masks produced by the atlas from built-in line
/// glyphs or rasterized font glyphs), so the two are directly comparable in
/// [`crate::matching::match_scene`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtractedFeatures {
    /// Normalized (sums to 1, or all-zero if the mask is empty) histogram of
    /// local stroke direction, one bin per [`DIRS`] direction.
    pub orientation: [f32; 8],
    /// Fraction of subcells that are set, in `[0, 1]`.
    pub density: f32,
    /// Center of mass of the set subcells, normalized to `[0, 1] x [0, 1]`.
    /// Defaults to the cell center `[0.5, 0.5]` for an empty mask.
    pub centroid: [f32; 2],
    /// How non-uniform the stroke direction is, in `[0, 1]`: `0` for a
    /// perfectly straight single-direction stroke, approaching `1` as the
    /// direction spreads evenly across multiple bins (corners, curves,
    /// intersections).
    pub curvature: f32,
    /// Number of 8-connected components in the mask, capped at `u8::MAX`.
    pub stroke_count: u8,
}

/// Computes all [`ExtractedFeatures`] for a `width x height` subcell mask in
/// one pass (the orientation scan and connected-component labeling both
/// require single, dedicated sweeps, but density/centroid can piggyback on
/// the same pixel iteration).
pub fn extract_features(mask: &CellMask, width: usize, height: usize) -> ExtractedFeatures {
    let orientation = orientation_histogram(mask, width, height);
    let (density, centroid) = density_and_centroid(mask, width, height);
    let curvature = curvature_from_orientation(&orientation);
    let stroke_count = count_strokes(mask, width, height);

    ExtractedFeatures {
        orientation,
        density,
        centroid,
        curvature,
        stroke_count,
    }
}

/// Builds a direction histogram by, for every set subcell, checking which of
/// its 8 neighbors are also set and accumulating a hit in the matching
/// [`DIRS`] bin. A straight horizontal stroke lights up the E/W bins; a
/// vertical stroke lights up N/S; a diagonal lights up NE/SW or NW/SE, and
/// so on. The result is normalized so the bins sum to 1 (an empty mask
/// yields an all-zero histogram rather than dividing by zero).
fn orientation_histogram(mask: &CellMask, width: usize, height: usize) -> [f32; 8] {
    let mut hist = [0.0f32; 8];

    for y in 0..height {
        for x in 0..width {
            if !mask.get(x, y, width) {
                continue;
            }
            for (i, (dx, dy)) in DIRS.iter().enumerate() {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx >= 0
                    && ny >= 0
                    && (nx as usize) < width
                    && (ny as usize) < height
                    && mask.get(nx as usize, ny as usize, width)
                {
                    hist[i] += 1.0;
                }
            }
        }
    }

    let total: f32 = hist.iter().sum();
    if total > 0.0 {
        for bin in &mut hist {
            *bin /= total;
        }
    }
    hist
}

/// Fraction of set subcells (`density`) and their normalized center of mass
/// (`centroid`), computed together since both only need the list of set
/// coordinates. An empty mask has density `0.0` and a centroid pinned to the
/// cell's geometric center `[0.5, 0.5]` (a neutral default that doesn't bias
/// distance comparisons toward a corner).
fn density_and_centroid(mask: &CellMask, width: usize, height: usize) -> (f32, [f32; 2]) {
    let mut count = 0u32;
    let mut sum_x = 0u64;
    let mut sum_y = 0u64;

    for y in 0..height {
        for x in 0..width {
            if mask.get(x, y, width) {
                count += 1;
                sum_x += x as u64;
                sum_y += y as u64;
            }
        }
    }

    let total_cells = (width * height).max(1) as f32;
    let density = count as f32 / total_cells;

    if count == 0 {
        return (density, [0.5, 0.5]);
    }

    let mean_x = sum_x as f32 / count as f32;
    let mean_y = sum_y as f32 / count as f32;
    let centroid = [
        if width > 1 {
            mean_x / (width - 1) as f32
        } else {
            0.5
        },
        if height > 1 {
            mean_y / (height - 1) as f32
        } else {
            0.5
        },
    ];

    (density, centroid)
}

/// Curvature as `1 - (dominant bin fraction)`: a mask whose orientation
/// histogram is concentrated in one or two opposite bins (a straight stroke)
/// scores near `0`; a mask whose direction hits are spread across many bins
/// (a corner, curve, or intersection) scores closer to `1`. An empty
/// histogram (no set subcells) is defined as `0.0` curvature rather than
/// `1.0`, since "no stroke" isn't "maximally curved".
fn curvature_from_orientation(orientation: &[f32; 8]) -> f32 {
    let total: f32 = orientation.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let max_bin = orientation.iter().cloned().fold(0.0f32, f32::max);
    (1.0 - max_bin / total).clamp(0.0, 1.0)
}

/// Counts 8-connected components of set subcells via iterative flood fill,
/// capped at `u8::MAX` (255 strokes in a single character cell is already
/// far beyond anything meaningful, so saturating here avoids a wider return
/// type for a case that can't occur in practice).
fn count_strokes(mask: &CellMask, width: usize, height: usize) -> u8 {
    if width == 0 || height == 0 {
        return 0;
    }
    let mut visited = vec![false; width * height];
    let mut count: u32 = 0;
    let mut stack = Vec::new();

    for start_y in 0..height {
        for start_x in 0..width {
            let start_idx = start_y * width + start_x;
            if visited[start_idx] || !mask.get(start_x, start_y, width) {
                continue;
            }

            count += 1;
            visited[start_idx] = true;
            stack.push((start_x, start_y));

            while let Some((x, y)) = stack.pop() {
                for (dx, dy) in DIRS {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || (nx as usize) >= width || (ny as usize) >= height {
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    let idx = ny * width + nx;
                    if !visited[idx] && mask.get(nx, ny, width) {
                        visited[idx] = true;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }

    count.min(u8::MAX as u32) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from_coords(coords: &[(usize, usize)], width: usize) -> CellMask {
        let mut mask = CellMask::new();
        for &(x, y) in coords {
            let bit_idx = y * width + x;
            mask.words[bit_idx / 64] |= 1 << (bit_idx % 64);
        }
        mask
    }

    #[test]
    fn empty_mask_has_zero_density_and_centered_centroid() {
        let mask = CellMask::new();
        let features = extract_features(&mask, 16, 32);
        assert_eq!(features.density, 0.0);
        assert_eq!(features.centroid, [0.5, 0.5]);
        assert_eq!(features.orientation, [0.0; 8]);
        assert_eq!(features.curvature, 0.0);
        assert_eq!(features.stroke_count, 0);
    }

    #[test]
    fn horizontal_line_has_east_west_dominant_orientation() {
        // A horizontal run of 6 pixels at y=16 in a 16x32 mask.
        let coords: Vec<_> = (5..11).map(|x| (x, 16)).collect();
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);

        let e = features.orientation[2]; // E
        let w = features.orientation[6]; // W
        let n = features.orientation[0]; // N
        let s = features.orientation[4]; // S
        assert!(
            e > 0.0 && w > 0.0,
            "E/W bins should be nonzero for a horizontal stroke"
        );
        assert!(
            e + w > n + s,
            "horizontal stroke should favor E/W bins over N/S bins"
        );
    }

    #[test]
    fn vertical_line_has_north_south_dominant_orientation() {
        let coords: Vec<_> = (10..20).map(|y| (8, y)).collect();
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);

        let n = features.orientation[0];
        let s = features.orientation[4];
        let e = features.orientation[2];
        let w = features.orientation[6];
        assert!(
            n > 0.0 && s > 0.0,
            "N/S bins should be nonzero for a vertical stroke"
        );
        assert!(
            n + s > e + w,
            "vertical stroke should favor N/S bins over E/W bins"
        );
    }

    #[test]
    fn straight_line_has_lower_curvature_than_a_corner() {
        let straight_coords: Vec<_> = (0..16).map(|x| (x, 16)).collect();
        let straight = mask_from_coords(&straight_coords, 16);
        let straight_features = extract_features(&straight, 16, 32);

        // An L-shaped corner: horizontal run plus a perpendicular vertical run
        // sharing an endpoint.
        let mut corner_coords: Vec<_> = (0..8).map(|x| (x, 16)).collect();
        corner_coords.extend((16..24).map(|y| (7, y)));
        let corner = mask_from_coords(&corner_coords, 16);
        let corner_features = extract_features(&corner, 16, 32);

        assert!(
            straight_features.curvature < corner_features.curvature,
            "straight stroke ({}) should have lower curvature than a corner ({})",
            straight_features.curvature,
            corner_features.curvature
        );
    }

    #[test]
    fn density_matches_set_bit_fraction() {
        let coords: Vec<_> = (0..16).map(|x| (x, 0)).collect(); // full row: 16 of 512 bits
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);
        assert!((features.density - 16.0 / 512.0).abs() < 1e-6);
    }

    #[test]
    fn centroid_is_offset_toward_populated_region() {
        // All set pixels clustered in the top-left corner.
        let coords: Vec<_> = (0..4).flat_map(|y| (0..4).map(move |x| (x, y))).collect();
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);
        assert!(features.centroid[0] < 0.5, "centroid x should skew left");
        assert!(features.centroid[1] < 0.5, "centroid y should skew top");
    }

    #[test]
    fn stroke_count_counts_disjoint_components() {
        // Two separate single-pixel blobs, far enough apart to not be
        // 8-connected to each other.
        let coords = [(0, 0), (10, 20)];
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);
        assert_eq!(features.stroke_count, 2);
    }

    #[test]
    fn stroke_count_treats_8_connected_pixels_as_one_component() {
        // A diagonal run is 8-connected end-to-end, so it must count as a
        // single stroke, not one component per pixel.
        let coords: Vec<_> = (0..5).map(|i| (i, i)).collect();
        let mask = mask_from_coords(&coords, 16);
        let features = extract_features(&mask, 16, 32);
        assert_eq!(features.stroke_count, 1);
    }
}
