pub mod canvas;
pub mod clipping;
pub mod features;
pub mod geometry;
pub mod matching;

// Re-exported at the crate root for convenience: `GlyphIndex` originally
// lived in `topoglyph-atlas` before moving here so `match_scene_indexed`
// (in `matching`) could consume it without a dependency cycle.
pub use matching::GlyphIndex;
