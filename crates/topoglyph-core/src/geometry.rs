use bitflags::bitflags;

pub const MAX_AUTO_COLUMNS: usize = 600;
pub const MAX_AUTO_ROWS: usize = 300;

/// Defines the grid dimensions and subcell resolution for mapping.
///
/// Leaving both `columns` and `rows` unset derives a resolution-aware grid
/// from the source image while preserving its physical aspect ratio. Fully
/// automatic grids never upscale the source and are capped at
/// [`MAX_AUTO_COLUMNS`] by [`MAX_AUTO_ROWS`].
pub struct GridOptions {
    pub columns: Option<usize>,
    pub rows: Option<usize>,
    pub cell_aspect_ratio: f32,
    pub subcell_width: u8,
    pub subcell_height: u8,
}

impl GridOptions {
    /// Resolves the effective text grid for a source image.
    ///
    /// An explicit dimension is always honored. If exactly one dimension is
    /// explicit, the other is derived from the source and cell aspect ratios.
    /// The automatic caps apply only when neither dimension is explicit.
    pub fn resolve_dimensions(&self, source_dimensions: (u32, u32)) -> (usize, usize) {
        let source_width = f64::from(source_dimensions.0.max(1));
        let source_height = f64::from(source_dimensions.1.max(1));
        let cell_aspect_ratio = {
            let ratio = f64::from(self.cell_aspect_ratio);
            if ratio.is_finite() && ratio > 0.0 {
                ratio
            } else {
                0.5
            }
        };

        match (self.columns, self.rows) {
            (Some(columns), Some(rows)) => (columns.max(1), rows.max(1)),
            (Some(columns), None) => {
                let columns = columns.max(1);
                let rows =
                    (columns as f64 * source_height / source_width * cell_aspect_ratio).round();
                (columns, (rows as usize).max(1))
            }
            (None, Some(rows)) => {
                let rows = rows.max(1);
                let columns =
                    (rows as f64 * source_width / source_height / cell_aspect_ratio).round();
                ((columns as usize).max(1), rows)
            }
            (None, None) => {
                let natural_rows = source_height * cell_aspect_ratio;
                let scale = 1.0_f64
                    .min(MAX_AUTO_COLUMNS as f64 / source_width)
                    .min(MAX_AUTO_ROWS as f64 / natural_rows);
                let columns = (source_width * scale).round() as usize;
                let rows = (natural_rows * scale).round() as usize;
                (
                    columns.clamp(1, MAX_AUTO_COLUMNS),
                    rows.clamp(1, MAX_AUTO_ROWS),
                )
            }
        }
    }
}

impl Default for GridOptions {
    fn default() -> Self {
        Self {
            columns: None,
            rows: None,
            cell_aspect_ratio: 0.5,
            subcell_width: 16,
            subcell_height: 32,
        }
    }
}

/// Bit mask representing a 16x32 subcell grid for a single character cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellMask {
    // 16 * 32 = 512 bits = 8 * 64
    pub words: [u64; 8],
}

impl CellMask {
    pub fn new() -> Self {
        Self { words: [0; 8] }
    }

    /// Calculate the XOR distance (number of differing bits) between two masks.
    pub fn xor_distance(&self, other: &Self) -> u32 {
        self.words
            .iter()
            .zip(&other.words)
            .map(|(x, y)| (x ^ y).count_ones())
            .sum()
    }

    /// Calculate the Jaccard distance (1.0 - Intersection over Union) between two masks.
    /// If both masks are empty, the distance is 0.0 (perfect match).
    pub fn iou_distance(&self, other: &Self) -> f32 {
        let mut intersection = 0;
        let mut union = 0;
        for (x, y) in self.words.iter().zip(&other.words) {
            intersection += (x & y).count_ones();
            union += (x | y).count_ones();
        }
        if union == 0 {
            return 0.0;
        }
        1.0 - (intersection as f32 / union as f32)
    }

    /// Reads the bit at flat index `bit_idx` (`y * width + x`). Out-of-range
    /// indices (beyond the mask's 512 bits) read as unset rather than
    /// panicking, so callers can iterate a caller-supplied `width`/`height`
    /// without needing to separately validate it fits the fixed-size mask.
    #[inline]
    pub fn get_bit(&self, bit_idx: usize) -> bool {
        let word = bit_idx / 64;
        let offset = bit_idx % 64;
        match self.words.get(word) {
            Some(w) => (w >> offset) & 1 != 0,
            None => false,
        }
    }

    /// Reads the bit at 2D coordinate `(x, y)` in a mask laid out row-major
    /// with the given `width`.
    #[inline]
    pub fn get(&self, x: usize, y: usize, width: usize) -> bool {
        self.get_bit(y * width + x)
    }

    /// Total number of set bits in the mask.
    #[inline]
    pub fn popcount(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }
}

impl Default for CellMask {
    fn default() -> Self {
        Self::new()
    }
}

bitflags! {
    /// 8-direction ports indicating where geometry crosses the cell boundary.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PortMask: u16 {
        const N  = 1 << 0;
        const NE = 1 << 1;
        const E  = 1 << 2;
        const SE = 1 << 3;
        const S  = 1 << 4;
        const SW = 1 << 5;
        const W  = 1 << 6;
        const NW = 1 << 7;
    }
}

/// Features extracted from a single cell for matching against GlyphAtlas.
#[derive(Debug, Clone)]
pub struct CellDescriptor {
    pub mask: CellMask,
    pub ports: PortMask,
    pub orientation: [f32; 8],
    pub density: f32,
    pub centroid: [f32; 2],
    pub curvature: f32,
    pub stroke_count: u8,
    pub color: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::GridOptions;

    #[test]
    fn default_grid_tracks_square_source_resolution() {
        let options = GridOptions {
            cell_aspect_ratio: 0.6,
            ..Default::default()
        };

        assert_eq!(options.columns, None);
        assert_eq!(options.resolve_dimensions((250, 250)), (250, 150));
    }

    #[test]
    fn automatic_grid_caps_landscape_and_portrait_sources() {
        let options = GridOptions {
            cell_aspect_ratio: 0.6,
            ..Default::default()
        };

        assert_eq!(options.resolve_dimensions((1280, 577)), (600, 162));
        assert_eq!(options.resolve_dimensions((400, 800)), (250, 300));
    }

    #[test]
    fn automatic_grid_does_not_upscale_small_sources() {
        let options = GridOptions::default();

        assert_eq!(options.resolve_dimensions((32, 16)), (32, 8));
    }

    #[test]
    fn one_explicit_dimension_derives_the_other_from_aspect() {
        let width_only = GridOptions {
            columns: Some(120),
            cell_aspect_ratio: 0.5,
            ..Default::default()
        };
        let height_only = GridOptions {
            rows: Some(40),
            cell_aspect_ratio: 0.5,
            ..Default::default()
        };

        assert_eq!(width_only.resolve_dimensions((100, 100)), (120, 60));
        assert_eq!(height_only.resolve_dimensions((100, 100)), (80, 40));
    }

    #[test]
    fn two_explicit_dimensions_are_used_exactly() {
        let options = GridOptions {
            columns: Some(120),
            rows: Some(40),
            ..Default::default()
        };

        assert_eq!(options.resolve_dimensions((100, 200)), (120, 40));
    }
}
