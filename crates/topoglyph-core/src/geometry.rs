use bitflags::bitflags;

/// Defines the grid dimensions and subcell resolution for mapping.
pub struct GridOptions {
    pub columns: usize,
    pub rows: Option<usize>,
    pub cell_aspect_ratio: f32,
    pub subcell_width: u8,
    pub subcell_height: u8,
}

impl Default for GridOptions {
    fn default() -> Self {
        Self {
            columns: 120,
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
