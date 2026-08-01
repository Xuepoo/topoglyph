//! `.tglyph` text-animation format: a plain-text, frame-differential
//! sequence of [`TextCanvas`]es with timing metadata, for the "输出很多每一
//! 帧转换的文字" use case (`topoglyph-docs/TODO.md` 0.5.0 video section).
//!
//! Rather than encoding a *video* file, an animation is expressed as pure
//! text: the same character-art alphabet the still-image encoders already
//! produce, one frame per video frame, separated by explicit markers. The
//! first frame is written in full; every subsequent frame is written as a
//! *delta* against the previous frame — only the cells whose token/color
//! actually changed. Because a talking-head or mostly-static video changes
//! only a fraction of its cells between two consecutive frames, this delta
//! encoding is what actually gets compression mileage here (as opposed to,
//! say, re-encoding printable text into a denser binary/base-N alphabet,
//! which doesn't help when the payload is already printable text and the
//! real redundancy is *between frames*, not *within* one frame's byte
//! representation). Color is emitted per changed cell only when the
//! animation is built with `include_color: true` (default `false`, per
//! `topoglyph-docs/TODO.md`: "颜色默认关闭，由用户选择开启").
//!
//! # Format
//!
//! ```text
//! TOPOGLYPH-ANIM v1
//! WIDTH 120
//! HEIGHT 60
//! FPS 24
//! COLOR off
//! FRAMES 300
//! ---F0---
//! <height lines of width characters: the full first frame>
//! ---D1---
//! <row>,<col>,<char>[,<#rrggbb>]
//! ...
//! ---D2---
//! ...
//! ```
//!
//! `<char>` is never a literal comma, newline, or `-` at the start of the
//! frame marker line, so lines are split on the first two commas rather
//! than requiring escaping (glyph tokens from the built-in/box-drawing
//! atlases and font-rasterized custom charsets never contain a comma).
//! `<char>` may be empty to represent a space (see [`Self::decode`]).

use topoglyph_core::canvas::{TextCanvas, TextCell};

/// A decoded `.tglyph` animation: header metadata plus every frame as a
/// fully-materialized [`TextCanvas`] (delta-decoding already applied), so
/// callers (a terminal player, a test) can index any frame directly instead
/// of re-deriving it from all preceding deltas each time.
#[derive(Debug, Clone, PartialEq)]
pub struct TglyphAnimation {
    pub width: usize,
    pub height: usize,
    pub fps: f32,
    pub include_color: bool,
    pub frames: Vec<TextCanvas>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TglyphError {
    #[error("empty input")]
    EmptyInput,
    #[error("missing or malformed header line {0}: expected {1}, got {2:?}")]
    MalformedHeader(&'static str, &'static str, String),
    #[error("frame {0}: expected {1} rows, found {2}")]
    FrameRowCountMismatch(usize, usize, usize),
    #[error("frame {0}: row {1} has {2} columns, expected {3}")]
    FrameColumnCountMismatch(usize, usize, usize, usize),
    #[error("frame {0}: expected marker '---F0---' or '---D<n>---', got {1:?}")]
    UnexpectedFrameMarker(usize, String),
    #[error("frame {0}: malformed delta line {1:?}")]
    MalformedDeltaLine(usize, String),
    #[error("frame {0}: delta references out-of-range cell ({1}, {2}) for a {3}x{4} canvas")]
    DeltaOutOfRange(usize, usize, usize, usize, usize),
    #[error("no frames were provided to encode")]
    NoFrames,
    #[error("frame {0}: expected {1}x{2}, got {3}x{4}")]
    FrameSizeMismatch(usize, usize, usize, usize, usize),
    #[error("unexpected end of binary data while reading {0}")]
    UnexpectedEof(&'static str),
    #[error("malformed binary .tglyph data: {0}")]
    MalformedBinary(&'static str),
}

impl TglyphAnimation {
    /// Builds an animation from a sequence of already-matched
    /// [`TextCanvas`]es (one per source video frame, in playback order),
    /// all of which must share the same `width`/`height`.
    pub fn encode(
        canvases: &[TextCanvas],
        fps: f32,
        include_color: bool,
    ) -> Result<Self, TglyphError> {
        let first = canvases.first().ok_or(TglyphError::NoFrames)?;
        let (width, height) = (first.width, first.height);

        for (i, canvas) in canvases.iter().enumerate() {
            if canvas.width != width || canvas.height != height {
                return Err(TglyphError::FrameSizeMismatch(
                    i,
                    width,
                    height,
                    canvas.width,
                    canvas.height,
                ));
            }
        }

        Ok(Self {
            width,
            height,
            fps,
            include_color,
            frames: canvases.to_vec(),
        })
    }

    /// Serializes the animation to the `.tglyph` text format described in
    /// the module docs.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("TOPOGLYPH-ANIM v1\n");
        out.push_str(&format!("WIDTH {}\n", self.width));
        out.push_str(&format!("HEIGHT {}\n", self.height));
        out.push_str(&format!("FPS {}\n", self.fps));
        out.push_str(&format!(
            "COLOR {}\n",
            if self.include_color { "on" } else { "off" }
        ));
        out.push_str(&format!("FRAMES {}\n", self.frames.len()));

        for (i, canvas) in self.frames.iter().enumerate() {
            if i == 0 {
                out.push_str("---F0---\n");
                for row in 0..self.height {
                    for col in 0..self.width {
                        out.push_str(&canvas.cells[row * self.width + col].token);
                    }
                    out.push('\n');
                }
                if self.include_color {
                    // The full-grid frame text above only carries tokens;
                    // any per-cell colors for frame 0 are recorded
                    // separately here in the same `row,col,color` shape as
                    // a delta line's color field, so decode() can reuse the
                    // exact same parsing path for both.
                    out.push_str("---C0---\n");
                    for (idx, cell) in canvas.cells.iter().enumerate() {
                        if let Some(color) = &cell.color {
                            let row = idx / self.width;
                            let col = idx % self.width;
                            out.push_str(&format!("{row},{col},{color}\n"));
                        }
                    }
                }
            } else {
                out.push_str(&format!("---D{i}---\n"));
                let prev = &self.frames[i - 1];
                for (idx, (cell, prev_cell)) in
                    canvas.cells.iter().zip(prev.cells.iter()).enumerate()
                {
                    if cell.token == prev_cell.token
                        && (!self.include_color || cell.color == prev_cell.color)
                    {
                        continue;
                    }
                    let row = idx / self.width;
                    let col = idx % self.width;
                    let token = if cell.token == " " {
                        String::new()
                    } else {
                        cell.token.clone()
                    };
                    if self.include_color {
                        let color = cell.color.as_deref().unwrap_or("");
                        out.push_str(&format!("{row},{col},{token},{color}\n"));
                    } else {
                        out.push_str(&format!("{row},{col},{token}\n"));
                    }
                }
            }
        }

        out
    }

    /// Parses a `.tglyph` text document back into an animation, fully
    /// materializing every frame (applying each delta on top of the
    /// previous frame's decoded canvas).
    pub fn decode(text: &str) -> Result<Self, TglyphError> {
        let mut lines = text.lines();

        let magic = lines.next().ok_or(TglyphError::EmptyInput)?;
        if magic != "TOPOGLYPH-ANIM v1" {
            return Err(TglyphError::MalformedHeader(
                "magic",
                "TOPOGLYPH-ANIM v1",
                magic.to_string(),
            ));
        }

        let width = parse_header_usize(lines.next(), "WIDTH")?;
        let height = parse_header_usize(lines.next(), "HEIGHT")?;
        let fps = parse_header_f32(lines.next(), "FPS")?;
        let include_color = parse_header_color(lines.next())?;
        let frame_count = parse_header_usize(lines.next(), "FRAMES")?;

        let mut frames: Vec<TextCanvas> = Vec::with_capacity(frame_count);

        for i in 0..frame_count {
            let marker = lines
                .next()
                .ok_or_else(|| TglyphError::UnexpectedFrameMarker(i, String::new()))?;
            let expected_marker = if i == 0 {
                "---F0---".to_string()
            } else {
                format!("---D{i}---")
            };
            if marker != expected_marker {
                return Err(TglyphError::UnexpectedFrameMarker(i, marker.to_string()));
            }

            if i == 0 {
                let mut cells = Vec::with_capacity(width * height);
                for row in 0..height {
                    let line = lines
                        .next()
                        .ok_or(TglyphError::FrameRowCountMismatch(0, height, row))?;
                    let chars: Vec<char> = line.chars().collect();
                    if chars.len() != width {
                        return Err(TglyphError::FrameColumnCountMismatch(
                            0,
                            row,
                            chars.len(),
                            width,
                        ));
                    }
                    for ch in chars {
                        cells.push(TextCell {
                            token: ch.to_string(),
                            score: 0.0,
                            source_path: None,
                            color: None,
                        });
                    }
                }

                if include_color {
                    let color_marker = lines
                        .next()
                        .ok_or_else(|| TglyphError::UnexpectedFrameMarker(0, String::new()))?;
                    if color_marker != "---C0---" {
                        return Err(TglyphError::UnexpectedFrameMarker(
                            0,
                            color_marker.to_string(),
                        ));
                    }
                    loop {
                        let mut peek = lines.clone();
                        match peek.next() {
                            None => break,
                            Some(next_line) if next_line.starts_with("---") => break,
                            _ => {}
                        }
                        let line = lines.next().unwrap();
                        apply_color_line(&mut cells, width, height, 0, line)?;
                    }
                }

                frames.push(TextCanvas {
                    width,
                    height,
                    cells,
                });
            } else {
                let mut canvas = frames[i - 1].clone();
                // Delta lines run until the next frame marker or EOF; since
                // we don't know the delta's line count up front, peek ahead
                // by cloning the iterator rather than consuming lines that
                // belong to the next marker.
                loop {
                    let mut peek = lines.clone();
                    match peek.next() {
                        None => break,
                        Some(next_line) if next_line.starts_with("---") => break,
                        _ => {}
                    }
                    let line = lines.next().unwrap();
                    apply_delta_line(&mut canvas, i, line, include_color)?;
                }
                frames.push(canvas);
            }
        }

        Ok(Self {
            width,
            height,
            fps,
            include_color,
            frames,
        })
    }
    /// Serializes the animation to the compact binary v2 `.tglyph` layout
    /// (see `crate::binary`'s module docs). This is what `topoglyph video`
    /// writes by default as of 0.2.2 — measured ~23% the size of the
    /// equivalent [`to_text`] output on real animation content, at the
    /// cost of no longer being line-oriented human-readable text.
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::binary::encode(self)
    }

    /// Parses either format `.tglyph` document back into an animation:
    /// the compact binary v2 layout (detected via its magic bytes, see
    /// `crate::binary::is_binary`) or, for backward compatibility, the
    /// original human-readable text v1 format via [`decode`]. Callers
    /// that already know they have UTF-8 text should prefer calling
    /// [`decode`] directly; this is for the common "I have some bytes
    /// read from a `.tglyph` file and don't know which format" case (the
    /// CLI's `play` subcommand, for example).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TglyphError> {
        if crate::binary::is_binary(bytes) {
            crate::binary::decode(bytes)
        } else {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                TglyphError::MalformedHeader(
                    "magic",
                    "TGLYPHB2 or TOPOGLYPH-ANIM v1",
                    "<invalid UTF-8, and not v2 binary magic>".to_string(),
                )
            })?;
            Self::decode(text)
        }
    }
}

/// Applies one `---C0---` color-initialization line (`row,col,color`) onto
/// frame 0's cells, in-place. Separate from [`apply_delta_line`] because
/// this format has no token field to update — frame 0's tokens already came
/// from the full-grid frame text.
fn apply_color_line(
    cells: &mut [TextCell],
    width: usize,
    height: usize,
    frame_idx: usize,
    line: &str,
) -> Result<(), TglyphError> {
    let mut parts = line.splitn(3, ',');
    let row: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;
    let col: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;
    let color = parts
        .next()
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;

    if row >= height || col >= width {
        return Err(TglyphError::DeltaOutOfRange(
            frame_idx, row, col, width, height,
        ));
    }

    cells[row * width + col].color = Some(color.to_string());
    Ok(())
}

fn apply_delta_line(
    canvas: &mut TextCanvas,
    frame_idx: usize,
    line: &str,
    include_color: bool,
) -> Result<(), TglyphError> {
    let mut parts = line.splitn(if include_color { 4 } else { 3 }, ',');
    let row: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;
    let col: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;
    let token = parts
        .next()
        .ok_or_else(|| TglyphError::MalformedDeltaLine(frame_idx, line.to_string()))?;
    let token = if token.is_empty() {
        " ".to_string()
    } else {
        token.to_string()
    };
    let color = if include_color {
        parts.next().and_then(|c| {
            if c.is_empty() {
                None
            } else {
                Some(c.to_string())
            }
        })
    } else {
        None
    };

    if row >= canvas.height || col >= canvas.width {
        return Err(TglyphError::DeltaOutOfRange(
            frame_idx,
            row,
            col,
            canvas.width,
            canvas.height,
        ));
    }

    let idx = row * canvas.width + col;
    canvas.cells[idx].token = token;
    if include_color {
        canvas.cells[idx].color = color;
    }
    Ok(())
}

fn parse_header_usize(line: Option<&str>, key: &'static str) -> Result<usize, TglyphError> {
    let line = line.ok_or_else(|| TglyphError::MalformedHeader(key, key, String::new()))?;
    let value = line
        .strip_prefix(key)
        .and_then(|rest| rest.trim().parse().ok())
        .ok_or_else(|| TglyphError::MalformedHeader(key, key, line.to_string()))?;
    Ok(value)
}

fn parse_header_f32(line: Option<&str>, key: &'static str) -> Result<f32, TglyphError> {
    let line = line.ok_or_else(|| TglyphError::MalformedHeader(key, key, String::new()))?;
    let value = line
        .strip_prefix(key)
        .and_then(|rest| rest.trim().parse().ok())
        .ok_or_else(|| TglyphError::MalformedHeader(key, key, line.to_string()))?;
    Ok(value)
}

fn parse_header_color(line: Option<&str>) -> Result<bool, TglyphError> {
    let line = line.ok_or_else(|| TglyphError::MalformedHeader("COLOR", "COLOR", String::new()))?;
    match line.strip_prefix("COLOR").map(str::trim) {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(TglyphError::MalformedHeader(
            "COLOR",
            "COLOR on|off",
            line.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(tokens: &[&str], width: usize, colors: Option<&[Option<&str>]>) -> TextCanvas {
        let height = tokens.len() / width;
        TextCanvas {
            width,
            height,
            cells: tokens
                .iter()
                .enumerate()
                .map(|(i, token)| TextCell {
                    token: token.to_string(),
                    score: 0.0,
                    source_path: None,
                    color: colors.and_then(|c| c[i]).map(|s| s.to_string()),
                })
                .collect(),
        }
    }

    #[test]
    fn encode_rejects_empty_frame_list() {
        assert_eq!(
            TglyphAnimation::encode(&[], 24.0, false),
            Err(TglyphError::NoFrames)
        );
    }

    #[test]
    fn encode_rejects_mismatched_frame_dimensions() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let f1 = canvas(&["a", "b", "c"], 3, None);
        assert_eq!(
            TglyphAnimation::encode(&[f0, f1], 24.0, false),
            Err(TglyphError::FrameSizeMismatch(1, 2, 2, 3, 1))
        );
    }

    #[test]
    fn round_trip_single_frame() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let anim = TglyphAnimation::encode(std::slice::from_ref(&f0), 24.0, false).unwrap();
        let text = anim.to_text();
        assert!(text.starts_with("TOPOGLYPH-ANIM v1\n"));
        assert!(text.contains("FRAMES 1\n"));
        assert!(text.contains("---F0---\n"));

        let decoded = TglyphAnimation::decode(&text).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(
            decoded.frames[0]
                .cells
                .iter()
                .map(|c| c.token.clone())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn delta_frame_only_lists_changed_cells() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        // Only the top-right cell ("b" -> "x") changes.
        let f1 = canvas(&["a", "x", "c", "d"], 2, None);
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();
        let text = anim.to_text();

        let delta_section = text.split("---D1---\n").nth(1).unwrap();
        let delta_lines: Vec<&str> = delta_section.lines().collect();
        assert_eq!(delta_lines, vec!["0,1,x"]);
    }

    #[test]
    fn round_trip_multi_frame_with_deltas() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let f1 = canvas(&["a", "x", "c", "d"], 2, None);
        let f2 = canvas(&["a", "x", "y", "d"], 2, None);
        let anim =
            TglyphAnimation::encode(&[f0.clone(), f1.clone(), f2.clone()], 24.0, false).unwrap();
        let text = anim.to_text();
        let decoded = TglyphAnimation::decode(&text).unwrap();

        assert_eq!(decoded.frames.len(), 3);
        let tokens = |c: &TextCanvas| c.cells.iter().map(|c| c.token.clone()).collect::<Vec<_>>();
        assert_eq!(tokens(&decoded.frames[0]), tokens(&f0));
        assert_eq!(tokens(&decoded.frames[1]), tokens(&f1));
        assert_eq!(tokens(&decoded.frames[2]), tokens(&f2));
    }

    #[test]
    fn space_token_round_trips_through_empty_delta_field() {
        let f0 = canvas(&["a", "b"], 2, None);
        let f1 = canvas(&["a", " "], 2, None);
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();
        let text = anim.to_text();
        assert!(
            text.contains("0,1,\n"),
            "space token should serialize as an empty field, got: {text:?}"
        );

        let decoded = TglyphAnimation::decode(&text).unwrap();
        assert_eq!(decoded.frames[1].cells[1].token, " ");
    }

    #[test]
    fn color_is_omitted_by_default() {
        let f0 = canvas(&["a", "b"], 2, Some(&[Some("#ff0000"), None]));
        let anim = TglyphAnimation::encode(&[f0], 24.0, false).unwrap();
        let text = anim.to_text();
        assert!(
            !text.contains("#ff0000"),
            "color must not appear when include_color is false"
        );
        assert!(text.contains("COLOR off\n"));
    }

    #[test]
    fn color_round_trips_when_enabled() {
        let f0 = canvas(&["a", "b"], 2, Some(&[Some("#ff0000"), None]));
        let f1 = canvas(&["a", "b"], 2, Some(&[Some("#00ff00"), None]));
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, true).unwrap();
        let text = anim.to_text();
        assert!(text.contains("COLOR on\n"));

        let decoded = TglyphAnimation::decode(&text).unwrap();
        assert_eq!(decoded.frames[0].cells[0].color.as_deref(), Some("#ff0000"));
        assert_eq!(decoded.frames[1].cells[0].color.as_deref(), Some("#00ff00"));
    }

    #[test]
    fn a_color_only_change_produces_a_delta_when_color_enabled() {
        // Token is identical between frames but color changes; with
        // include_color=true this must still show up as a delta line.
        let f0 = canvas(&["a"], 1, Some(&[Some("#ff0000")]));
        let f1 = canvas(&["a"], 1, Some(&[Some("#00ff00")]));
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, true).unwrap();
        let text = anim.to_text();
        let delta_section = text.split("---D1---\n").nth(1).unwrap();
        assert_eq!(
            delta_section.lines().collect::<Vec<_>>(),
            vec!["0,0,a,#00ff00"]
        );
    }

    #[test]
    fn a_color_only_change_is_not_a_delta_when_color_disabled() {
        let f0 = canvas(&["a"], 1, Some(&[Some("#ff0000")]));
        let f1 = canvas(&["a"], 1, Some(&[Some("#00ff00")]));
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();
        let text = anim.to_text();
        let delta_section = text.split("---D1---\n").nth(1).unwrap();
        assert_eq!(
            delta_section.lines().collect::<Vec<&str>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn decode_rejects_wrong_magic() {
        let err = TglyphAnimation::decode("NOT-TOPOGLYPH\n").unwrap_err();
        assert!(matches!(err, TglyphError::MalformedHeader("magic", _, _)));
    }

    #[test]
    fn decode_rejects_row_count_mismatch() {
        let bad =
            "TOPOGLYPH-ANIM v1\nWIDTH 2\nHEIGHT 2\nFPS 24\nCOLOR off\nFRAMES 1\n---F0---\nab\n";
        let err = TglyphAnimation::decode(bad).unwrap_err();
        assert!(matches!(err, TglyphError::FrameRowCountMismatch(0, 2, 1)));
    }

    #[test]
    fn decode_rejects_column_count_mismatch() {
        let bad =
            "TOPOGLYPH-ANIM v1\nWIDTH 3\nHEIGHT 1\nFPS 24\nCOLOR off\nFRAMES 1\n---F0---\nab\n";
        let err = TglyphAnimation::decode(bad).unwrap_err();
        assert!(matches!(
            err,
            TglyphError::FrameColumnCountMismatch(0, 0, 2, 3)
        ));
    }

    #[test]
    fn decode_rejects_out_of_range_delta() {
        let bad = "TOPOGLYPH-ANIM v1\nWIDTH 2\nHEIGHT 1\nFPS 24\nCOLOR off\nFRAMES 2\n---F0---\nab\n---D1---\n5,5,x\n";
        let err = TglyphAnimation::decode(bad).unwrap_err();
        assert!(matches!(err, TglyphError::DeltaOutOfRange(1, 5, 5, 2, 1)));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(TglyphAnimation::decode(""), Err(TglyphError::EmptyInput));
    }

    #[test]
    fn single_static_frame_repeated_produces_empty_deltas() {
        // A perfectly static "video" (every frame identical) should produce
        // zero delta lines per frame -- this is the whole point of delta
        // encoding for mostly-unchanging footage.
        let f = canvas(&["a", "b", "c", "d"], 2, None);
        let anim =
            TglyphAnimation::encode(&[f.clone(), f.clone(), f.clone()], 24.0, false).unwrap();
        let text = anim.to_text();
        for marker in ["---D1---\n", "---D2---\n"] {
            let section = text.split(marker).nth(1).unwrap();
            let next_marker_pos = section.find("---").unwrap_or(section.len());
            assert_eq!(&section[..next_marker_pos], "");
        }
    }
}
