use rusttype::{point, Font, Scale};
use std::collections::HashMap;
use topoglyph_core::features::extract_features;
use topoglyph_core::geometry::{CellMask, PortMask};
use topoglyph_core::matching::GlyphDescriptor;
// `GlyphIndex` now lives in `topoglyph-core::matching` (so
// `match_scene_indexed` can consume it without a dependency cycle); it's
// re-exported here for source compatibility with existing callers that
// wrote `topoglyph_atlas::atlas::GlyphIndex`.
pub use topoglyph_core::matching::GlyphIndex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

pub struct GlyphAtlas {
    pub font_id: String,
    pub glyphs: Vec<GlyphDescriptor>,
    pub index: GlyphIndex,
}

impl GlyphAtlas {
    pub fn from_text(_text: &str, _options: &AtlasOptions) -> Result<Self, String> {
        // Build a minimal built-in line atlas for testing MVP
        let mut glyphs = Vec::new();

        // Macro to easily add lines. Orientation/density/centroid/curvature/
        // stroke_count are computed from the drawn mask via
        // `topoglyph_core::features::extract_features`, the same function
        // used for scene cells in `topoglyph_core::clipping`, so glyph and
        // cell descriptors are directly comparable in `match_scene`.
        macro_rules! add_glyph {
            ($token:expr, $ports:expr, $draw_fn:expr) => {
                let mut mask = CellMask::new();
                $draw_fn(&mut mask);
                let features = extract_features(&mask, 16, 32);
                glyphs.push(GlyphDescriptor {
                    token: $token.to_string(),
                    cell_width: 1,
                    mask,
                    ports: $ports,
                    orientation: features.orientation,
                    density: features.density,
                    centroid: features.centroid,
                    curvature: features.curvature,
                    stroke_count: features.stroke_count,
                    // Built-in line glyphs are always equally weighted;
                    // frequency bias only applies to custom character pools
                    // (see `from_custom_font`'s `--glyph-mode weighted`
                    // support).
                    frequency: 1.0,
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
        add_glyph!(
            "┼",
            PortMask::N | PortMask::S | PortMask::E | PortMask::W,
            |m: &mut CellMask| {
                draw_line_mask(0, hh, 15, hh, m);
                draw_line_mask(hw, 0, hw, 31, m);
            }
        );

        // T-junctions: three ports, letting the topology term connect a
        // stem to a through-line without forcing either a full "┼" or a
        // shape mismatch. Without these, any cell whose stroke geometry
        // branches (rather than crossing symmetrically) had no better
        // option than the nearest 2-port glyph, which either drops the
        // stem or drags the whole cell toward a "┼" it doesn't shape-match.
        // "├" (stem to the east, spine north-south)
        add_glyph!(
            "├",
            PortMask::N | PortMask::S | PortMask::E,
            |m: &mut CellMask| {
                draw_line_mask(hw, 0, hw, 31, m);
                draw_line_mask(hw, hh, 15, hh, m);
            }
        );
        // "┤" (stem to the west, spine north-south)
        add_glyph!(
            "┤",
            PortMask::N | PortMask::S | PortMask::W,
            |m: &mut CellMask| {
                draw_line_mask(hw, 0, hw, 31, m);
                draw_line_mask(0, hh, hw, hh, m);
            }
        );
        // "┬" (stem to the south, spine east-west)
        add_glyph!(
            "┬",
            PortMask::E | PortMask::W | PortMask::S,
            |m: &mut CellMask| {
                draw_line_mask(0, hh, 15, hh, m);
                draw_line_mask(hw, hh, hw, 31, m);
            }
        );
        // "┴" (stem to the north, spine east-west)
        add_glyph!(
            "┴",
            PortMask::E | PortMask::W | PortMask::N,
            |m: &mut CellMask| {
                draw_line_mask(0, hh, 15, hh, m);
                draw_line_mask(hw, 0, hw, hh, m);
            }
        );

        // Half-length strokes: a stroke that only reaches from a cell edge
        // to the center, rather than spanning the whole cell. Skeleton
        // segments frequently terminate mid-cell (a line ending, a short
        // branch stub) rather than running edge-to-edge; previously the
        // only glyphs available were full-length "─"/"│", so any
        // partial-length stroke's mask distance to those was large, and it
        // still only had a *single* port, no better connectivity-wise than
        // the correct half-stroke would have offered. These fill that gap
        // with single-port glyphs whose mask actually matches a stub.
        // "╵" (stub reaching up from the bottom half)
        add_glyph!("╵", PortMask::N, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, hh, m);
        });
        // "╷" (stub reaching down from the top half)
        add_glyph!("╷", PortMask::S, |m: &mut CellMask| {
            draw_line_mask(hw, hh, hw, 31, m);
        });
        // "╴" (stub reaching left from the right half)
        add_glyph!("╴", PortMask::W, |m: &mut CellMask| {
            draw_line_mask(0, hh, hw, hh, m);
        });
        // "╶" (stub reaching right from the left half)
        add_glyph!("╶", PortMask::E, |m: &mut CellMask| {
            draw_line_mask(hw, hh, 15, hh, m);
        });

        // "╳" (Diagonal cross)
        add_glyph!(
            "╳",
            PortMask::NE | PortMask::NW | PortMask::SE | PortMask::SW,
            |m: &mut CellMask| {
                draw_line_mask(0, 31, 15, 0, m);
                draw_line_mask(0, 0, 15, 31, m);
            }
        );

        // Sharp corners (geometrically identical to the rounded ones in this
        // 1px-wide Bresenham implementation, giving the engine more variety).
        // "┌"
        add_glyph!("┌", PortMask::S | PortMask::E, |m: &mut CellMask| {
            draw_line_mask(hw, 31, hw, hh, m);
            draw_line_mask(hw, hh, 15, hh, m);
        });
        // "┐"
        add_glyph!("┐", PortMask::S | PortMask::W, |m: &mut CellMask| {
            draw_line_mask(hw, 31, hw, hh, m);
            draw_line_mask(hw, hh, 0, hh, m);
        });
        // "└"
        add_glyph!("└", PortMask::N | PortMask::E, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, hh, m);
            draw_line_mask(hw, hh, 15, hh, m);
        });
        // "┘"
        add_glyph!("┘", PortMask::N | PortMask::W, |m: &mut CellMask| {
            draw_line_mask(hw, 0, hw, hh, m);
            draw_line_mask(hw, hh, 0, hh, m);
        });
        // Deliberately no isolated "point"/speck glyph: mask XOR distance
        // rewards fewer set bits almost unconditionally, so a near-empty
        // 1-bit mask wins the shape term against *any* sparse cell
        // regardless of the cell's actual orientation — verified by
        // rendering a real image with one added, which turned into a wall
        // of "·" everywhere a stroke only lightly grazed a cell. The
        // half-length stubs above already cover "a stroke that doesn't
        // reach a full edge" without that pathology, since their popcount
        // is comparable to the full-length lines instead of orders of
        // magnitude smaller.
        let index = GlyphIndex::build(&glyphs);

        Ok(Self {
            font_id: "builtin_lines".to_string(),
            glyphs,
            index,
        })
    }

    pub fn get_charset_string(charset: &str) -> Option<&'static str> {
        match charset {
            "ascii" => Some(" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"),
            "blocks" => Some(" ░▒▓█▄▀▌▐▖▗▘▙▚▛▜▝▞▟"),
            "braille" => Some(" ⠁⠂⠃⠄⠅⠆⠇⠈⠉⠊⠋⠌⠍⠎⠏⠐⠑⠒⠓⠔⠕⠖⠗⠘⠙⠚⠛⠜⠝⠞⠟⠠⠡⠢⠣⠤⠥⠦⠧⠨⠩⠪⠫⠬⠭⠮⠯⠰⠱⠲⠳⠴⠵⠶⠷⠸⠹⠺⠻⠼⠽⠾⠿"),
            _ => None
        }
    }

    /// Builds a custom-font glyph atlas from every grapheme in `text`.
    ///
    /// `text` may contain duplicate graphemes; each *distinct* grapheme
    /// produces one [`GlyphDescriptor`], and how often it repeats in `text`
    /// determines its `frequency` (normalized to `[0, 1]`, `1.0` for the
    /// most frequent grapheme). This is how the CLI's `--glyph-mode
    /// weighted` ("依据词频影响挑选", `topoglyph-docs/requirements.md`
    /// section 3.2) gets its weighting data: repeat a character in
    /// `--custom-chars` to bias matching toward it. Under `--glyph-mode
    /// set` (`MatchWeights::frequency_bias == 0.0`), `frequency` is computed
    /// the same way but simply never consulted, so passing duplicate
    /// characters is harmless either way.
    ///
    /// Grapheme cluster segmentation (via `unicode-segmentation`) means
    /// multi-codepoint sequences — combining marks, ZWJ emoji sequences
    /// like a family emoji — are each treated as one glyph rather than
    /// being split at the codepoint level.
    ///
    /// Each glyph's `cell_width` is computed from its real terminal display
    /// width (`unicode-width`'s East Asian Width data), so CJK ideographs
    /// and most emoji correctly report `2` instead of the previous
    /// hardcoded `1`. Note this is metadata only for now: the mask itself
    /// is still rasterized into the standard single-cell 16x32 [`CellMask`]
    /// (matching always compares one grid cell's mask against one glyph's
    /// mask), so a double-width glyph's shape is squeezed into the same
    /// mask as a single-width one rather than the matcher spanning two
    /// adjacent grid cells for it. Actually reflowing the output grid to
    /// give wide glyphs two columns (`topoglyph-docs/TODO.md` 0.5.0, "支持
    /// 多列字符") is a separate, not-yet-implemented change to
    /// `topoglyph_core::clipping`/`matching`; this only makes the metadata
    /// itself correct so a future pass can consume it.
    pub fn from_custom_font(
        text: &str,
        font_bytes: &[u8],
        options: &AtlasOptions,
    ) -> Result<Self, String> {
        let font = Font::try_from_bytes(font_bytes).ok_or("Failed to load font")?;
        let scale = Scale {
            x: options.mask_width as f32,
            y: options.mask_height as f32,
        };

        let (order, frequencies) = grapheme_frequencies(text);

        let mut glyphs = Vec::new();
        for grapheme in order {
            let v_metrics = font.v_metrics(scale);
            let offset = point(0.0, v_metrics.ascent);
            let rust_glyph = font.layout(grapheme, scale, offset).next();

            let mut mask = CellMask::new();
            if let Some(g) = rust_glyph {
                if let Some(bb) = g.pixel_bounding_box() {
                    g.draw(|x, y, v| {
                        let px = x as i32 + bb.min.x;
                        let py = y as i32 + bb.min.y;
                        if px >= 0
                            && px < options.mask_width as i32
                            && py >= 0
                            && py < options.mask_height as i32
                            && v > 0.1
                        {
                            // Threshold for density
                            let bit_idx = (py * options.mask_width as i32 + px) as usize;
                            mask.words[bit_idx / 64] |= 1 << (bit_idx % 64);
                        }
                    });
                }
            }

            let mask_w = options.mask_width as usize;
            let mask_h = options.mask_height as usize;
            let ports = ports_from_mask(&mask, mask_w, mask_h);
            let features = extract_features(&mask, mask_w, mask_h);

            glyphs.push(GlyphDescriptor {
                token: grapheme.to_string(),
                cell_width: grapheme_cell_width(grapheme),
                mask,
                ports,
                orientation: features.orientation,
                density: features.density,
                centroid: features.centroid,
                curvature: features.curvature,
                stroke_count: features.stroke_count,
                frequency: frequencies[grapheme],
            });
        }

        let index = GlyphIndex::build(&glyphs);

        Ok(Self {
            font_id: "custom_font".to_string(),
            glyphs,
            index,
        })
    }
}

/// Splits `text` into distinct graphemes (in first-seen order, for
/// deterministic atlas output regardless of hash map iteration order) and
/// computes each one's relative frequency (`count / max_count`, in `(0,
/// 1]`), for [`GlyphAtlas::from_custom_font`]'s `--glyph-mode weighted`
/// support (`topoglyph-docs/requirements.md` section 3.2).
fn grapheme_frequencies(text: &str) -> (Vec<&str>, HashMap<&str, f32>) {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for grapheme in text.graphemes(true) {
        let count = counts.entry(grapheme).or_insert(0);
        if *count == 0 {
            order.push(grapheme);
        }
        *count += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or(1).max(1) as f32;
    let frequencies = counts
        .into_iter()
        .map(|(g, c)| (g, c as f32 / max_count))
        .collect();
    (order, frequencies)
}

/// Real terminal display width of a grapheme (via `unicode-width`'s East
/// Asian Width data), clamped to `[1, 255]` to fit [`GlyphDescriptor`]'s
/// `u8` `cell_width` field. Zero-width graphemes (e.g. a bare combining
/// mark that didn't attach to a base character) are floored to `1` — a
/// glyph occupying no grid columns at all isn't representable in the
/// current single-cell-per-glyph matching model.
fn grapheme_cell_width(grapheme: &str) -> u8 {
    UnicodeWidthStr::width(grapheme).clamp(1, 255) as u8
}

/// Detects which of the mask's 4 edges have at least one set subcell,
/// approximating `crate::clipping`'s port detection (which comes for free
/// there from clipping against the cell boundary) for rasterized font
/// glyphs, where no such clipping pass exists. Diagonal ports (NE/NW/SE/SW)
/// are intentionally left unset: a single boundary pixel doesn't reliably
/// indicate a diagonal crossing the way it does for the hand-drawn line
/// glyphs, so we only claim the 4 cardinal ports we can detect with
/// confidence.
fn ports_from_mask(mask: &CellMask, width: usize, height: usize) -> PortMask {
    if width == 0 || height == 0 {
        return PortMask::empty();
    }
    let mut ports = PortMask::empty();
    for x in 0..width {
        if mask.get(x, 0, width) {
            ports.insert(PortMask::N);
        }
        if mask.get(x, height - 1, width) {
            ports.insert(PortMask::S);
        }
    }
    for y in 0..height {
        if mask.get(0, y, width) {
            ports.insert(PortMask::W);
        }
        if mask.get(width - 1, y, width) {
            ports.insert(PortMask::E);
        }
    }
    ports
}

// Simple Bresenham to build the built-in atlas masks
fn draw_line_mask(mut x0: isize, mut y0: isize, x1: isize, y1: isize, mask: &mut CellMask) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if (0..16).contains(&x0) && (0..32).contains(&y0) {
            let bit_idx = (y0 * 16 + x0) as usize;
            mask.words[bit_idx / 64] |= 1 << (bit_idx % 64);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(ports: PortMask, density: f32, cell_width: u8) -> GlyphDescriptor {
        GlyphDescriptor {
            token: "x".to_string(),
            cell_width,
            mask: CellMask::new(),
            ports,
            orientation: [0.0; 8],
            density,
            centroid: [0.5, 0.5],
            curvature: 0.0,
            stroke_count: 1,
            frequency: 1.0,
        }
    }

    #[test]
    fn grapheme_frequencies_ranks_repeated_grapheme_as_most_frequent() {
        let (order, freqs) = grapheme_frequencies("aaab");
        assert_eq!(order, vec!["a", "b"]);
        // "a" appears 3 times (the max), "b" appears once.
        assert_eq!(freqs["a"], 1.0);
        assert!((freqs["b"] - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn grapheme_frequencies_gives_every_char_full_weight_when_all_unique() {
        let (order, freqs) = grapheme_frequencies("xyz");
        assert_eq!(order, vec!["x", "y", "z"]);
        for g in order {
            assert_eq!(freqs[g], 1.0);
        }
    }

    #[test]
    fn grapheme_frequencies_handles_empty_input() {
        let (order, freqs) = grapheme_frequencies("");
        assert!(order.is_empty());
        assert!(freqs.is_empty());
    }

    #[test]
    fn grapheme_frequencies_treats_a_zwj_emoji_sequence_as_one_grapheme() {
        // Family emoji: man + ZWJ + woman + ZWJ + girl, a single extended
        // grapheme cluster despite being multiple codepoints.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let text = format!("{family}{family}x");
        let (order, freqs) = grapheme_frequencies(&text);
        assert_eq!(order, vec![family, "x"]);
        assert_eq!(freqs[family], 1.0);
        assert_eq!(freqs["x"], 0.5);
    }

    #[test]
    fn grapheme_cell_width_reports_1_for_ascii() {
        assert_eq!(grapheme_cell_width("x"), 1);
        assert_eq!(grapheme_cell_width("│"), 1);
    }

    #[test]
    fn grapheme_cell_width_reports_2_for_cjk_ideographs() {
        assert_eq!(grapheme_cell_width("字"), 2);
        assert_eq!(grapheme_cell_width("愛"), 2);
    }

    #[test]
    fn grapheme_cell_width_floors_zero_width_graphemes_to_1() {
        // A bare combining mark with no base character has display width 0
        // in unicode-width, but a glyph must occupy at least one grid cell.
        assert_eq!(grapheme_cell_width("\u{0301}"), 1);
    }

    #[test]
    fn from_text_builds_a_nonempty_index_matching_glyph_count() {
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let indexed_count: usize = atlas.index.by_ports.values().map(Vec::len).sum();
        assert_eq!(indexed_count, atlas.glyphs.len());
        let density_indexed: usize = atlas.index.by_density.iter().map(Vec::len).sum();
        assert_eq!(density_indexed, atlas.glyphs.len());
    }

    #[test]
    fn built_in_atlas_includes_t_junctions_and_half_strokes_but_no_bare_point() {
        // Regression test for the original 9-glyph atlas (only the 4
        // full-length lines, 4 rounded corners, and a "┼" cross) having no
        // 3-port or single-port options: any branching or partial-length
        // stroke had nothing better to match than a full 2-port line or the
        // one 4-port cross, which is what made "─" dominate real output
        // (see topoglyph-docs/TODO.md 0.4.0 notes). This locks in that the
        // expanded set is actually present and each new glyph carries the
        // ports its token implies.
        let atlas = GlyphAtlas::from_text("", &AtlasOptions::default()).unwrap();
        let find = |token: &str| atlas.glyphs.iter().find(|g| g.token == token);

        assert_eq!(atlas.glyphs.len(), 22, "expected 9 original + 8 new + 5 expanded glyphs");

        let tee_east = find("├").expect("├ should be in the built-in atlas");
        assert_eq!(
            tee_east.ports,
            PortMask::N | PortMask::S | PortMask::E,
            "├ should expose exactly N/S/E ports"
        );

        let stub_up = find("╵").expect("╵ should be in the built-in atlas");
        assert_eq!(
            stub_up.ports,
            PortMask::N,
            "╵ should expose only a single N port"
        );

        // No isolated "point" glyph: an empty/near-empty mask wins mask XOR
        // distance against almost any sparse cell regardless of the cell's
        // real orientation, which produced a wall of "·" when tried against
        // real image data. See the comment above the atlas construction.
        assert!(
            find("·").is_none(),
            "a bare point glyph should not be in the built-in atlas"
        );

        // Every new glyph's popcount should be in the same order of
        // magnitude as the original full-length lines (not a single lonely
        // bit), so it can only win shape score by actually resembling the
        // cell's stroke, not by being suspiciously sparse.
        let full_line_popcount = find("─").unwrap().mask.popcount();
        for token in ["├", "┤", "┬", "┴", "╵", "╷", "╴", "╶"] {
            let glyph = find(token).unwrap_or_else(|| panic!("{token} should be in the atlas"));
            let popcount = glyph.mask.popcount();
            assert!(
                popcount * 4 >= full_line_popcount,
                "{token}'s mask ({popcount} bits) is disproportionately sparser than \
                 a full-length line ({full_line_popcount} bits); this is the exact \
                 pathology that made a bare point glyph dominate real output"
            );
        }
    }

    #[test]
    fn by_ports_groups_glyphs_with_identical_port_mask() {
        let glyphs = vec![
            glyph(PortMask::N | PortMask::S, 0.1, 1),
            glyph(PortMask::N | PortMask::S, 0.2, 1),
            glyph(PortMask::W | PortMask::E, 0.1, 1),
        ];
        let index = GlyphIndex::build(&glyphs);
        let ns_group = index.by_ports.get(&(PortMask::N | PortMask::S)).unwrap();
        assert_eq!(ns_group, &vec![0, 1]);
    }

    #[test]
    fn glyphs_with_any_port_finds_partial_overlap() {
        let glyphs = vec![
            glyph(PortMask::N | PortMask::S, 0.1, 1),
            glyph(PortMask::W | PortMask::E, 0.1, 1),
            glyph(PortMask::N, 0.1, 1),
        ];
        let index = GlyphIndex::build(&glyphs);
        let mut hits = index.glyphs_with_any_port(PortMask::N);
        hits.sort_unstable();
        assert_eq!(hits, vec![0, 2]);
    }

    #[test]
    fn glyphs_with_any_port_empty_query_returns_every_glyph() {
        let glyphs = vec![glyph(PortMask::N, 0.1, 1), glyph(PortMask::S, 0.1, 1)];
        let index = GlyphIndex::build(&glyphs);
        assert_eq!(index.glyphs_with_any_port(PortMask::empty()).len(), 2);
    }

    #[test]
    fn glyphs_near_density_respects_bin_tolerance() {
        let glyphs = vec![
            glyph(PortMask::empty(), 0.0, 1),  // bin 0
            glyph(PortMask::empty(), 0.5, 1),  // bin 4
            glyph(PortMask::empty(), 0.99, 1), // bin 7
        ];
        let index = GlyphIndex::build(&glyphs);
        // Querying near density 0.0 with zero tolerance should only hit bin 0.
        assert_eq!(index.glyphs_near_density(0.0, 0).len(), 1);
        // Widening tolerance to the full range should hit every glyph.
        assert_eq!(index.glyphs_near_density(0.0, 7).len(), 3);
    }

    #[test]
    fn by_cell_width_groups_multi_column_glyphs_separately() {
        let glyphs = vec![
            glyph(PortMask::empty(), 0.1, 1),
            glyph(PortMask::empty(), 0.1, 2),
            glyph(PortMask::empty(), 0.1, 2),
        ];
        let index = GlyphIndex::build(&glyphs);
        assert_eq!(index.by_cell_width.get(&1).unwrap().len(), 1);
        assert_eq!(index.by_cell_width.get(&2).unwrap().len(), 2);
    }
}
