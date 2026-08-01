use serde::Serialize;
use topoglyph_core::canvas::TextCanvas;

pub trait TextEncoder {
    type Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error>;
}

#[derive(Default)]
pub struct PlainTextEncoder;

impl PlainTextEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl TextEncoder for PlainTextEncoder {
    type Error = std::io::Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error> {
        let mut result = String::new();
        for (i, cell) in canvas.cells.iter().enumerate() {
            result.push_str(&cell.token);
            if (i + 1) % canvas.width == 0 {
                result.push('\n');
            }
        }
        Ok(result.into_bytes())
    }
}

#[derive(Default)]
pub struct AnsiEncoder;

impl AnsiEncoder {
    pub fn new() -> Self {
        Self
    }

    // Parse #RRGGBB or #RRGGBBAA
    fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
        let hex = hex.trim_start_matches('#');
        if hex.len() >= 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        } else {
            None
        }
    }
}

impl TextEncoder for AnsiEncoder {
    type Error = std::io::Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error> {
        let mut result = String::new();
        let mut last_color = None;
        let total = canvas.cells.len();

        for (i, cell) in canvas.cells.iter().enumerate() {
            if cell.color != last_color {
                if let Some(color_str) = &cell.color {
                    if let Some((r, g, b)) = Self::parse_hex_color(color_str) {
                        result.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b));
                    } else {
                        result.push_str("\x1b[0m"); // reset if invalid
                    }
                } else {
                    result.push_str("\x1b[0m");
                }
                last_color = cell.color.clone();
            }

            result.push_str(&cell.token);

            // Reset before the row's newline (not after), and only when a
            // color is still active. Emitting it unconditionally after the
            // trailing '\n' turns the reset into its own phantom row once
            // split on lines (see `run_play`'s `.lines()` loop), which is
            // exactly what pushed terminal playback down by one row on
            // every colored frame.
            let is_last_cell = i + 1 == total;
            if is_last_cell && last_color.is_some() {
                result.push_str("\x1b[0m");
                last_color = None;
            }
            if (i + 1) % canvas.width == 0 {
                result.push('\n');
            }
        }
        if last_color.is_some() {
            result.push_str("\x1b[0m");
        }
        Ok(result.into_bytes())
    }
}

/// Serializable mirror of [`topoglyph_core::canvas::TextCell`]. Kept as a
/// separate DTO rather than adding `#[derive(Serialize)]` directly to the
/// core type so `topoglyph-core` doesn't need a `serde` dependency just to
/// support this one debug encoder.
#[derive(Debug, Serialize)]
struct DebugCell {
    token: String,
    score: f32,
    source_path: Option<usize>,
    color: Option<String>,
}

impl From<&topoglyph_core::canvas::TextCell> for DebugCell {
    fn from(cell: &topoglyph_core::canvas::TextCell) -> Self {
        Self {
            token: cell.token.clone(),
            score: cell.score,
            source_path: cell.source_path,
            color: cell.color.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DebugCanvas {
    width: usize,
    height: usize,
    cells: Vec<DebugCell>,
}

impl From<&TextCanvas> for DebugCanvas {
    fn from(canvas: &TextCanvas) -> Self {
        Self {
            width: canvas.width,
            height: canvas.height,
            cells: canvas.cells.iter().map(DebugCell::from).collect(),
        }
    }
}

/// Dumps the full [`TextCanvas`] — including per-cell match `score` and
/// `source_path`, which the plain-text/ANSI encoders discard — as pretty
/// JSON. Intended for the `atlas inspect` / debug tooling described in
/// `topoglyph-docs/TODO.md` 0.4.0 ("实现 Score Heatmap" / "JSON Debug 输出"),
/// where a human or a separate visualization tool needs the raw per-cell
/// match data rather than a rendered glyph string.
#[derive(Default)]
pub struct JsonDebugEncoder;

impl JsonDebugEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl TextEncoder for JsonDebugEncoder {
    type Error = serde_json::Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error> {
        let debug_canvas = DebugCanvas::from(canvas);
        serde_json::to_vec_pretty(&debug_canvas)
    }
}

/// Renders the canvas as a standalone HTML document using a monospace
/// `<pre>` block, one `<span>` per color run (mirroring the ANSI encoder's
/// "only emit an escape on color change" behavior to keep markup compact),
/// per `topoglyph-docs/TODO.md` 0.5.0 ("实现 HTML 导出模式 (锁定字体和字符宽度)").
#[derive(Default)]
pub struct HtmlEncoder;

impl HtmlEncoder {
    pub fn new() -> Self {
        Self
    }
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

impl TextEncoder for HtmlEncoder {
    type Error = std::io::Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error> {
        let mut body = String::new();
        let mut last_color: Option<Option<String>> = None;
        let mut span_open = false;

        for (i, cell) in canvas.cells.iter().enumerate() {
            if last_color.as_ref() != Some(&cell.color) {
                if span_open {
                    body.push_str("</span>");
                }
                match &cell.color {
                    Some(color) => {
                        body.push_str(&format!("<span style=\"color:{}\">", escape_html(color)));
                        span_open = true;
                    }
                    None => {
                        span_open = false;
                    }
                }
                last_color = Some(cell.color.clone());
            }

            body.push_str(&escape_html(&cell.token));
            if (i + 1) % canvas.width == 0 {
                body.push('\n');
            }
        }
        if span_open {
            body.push_str("</span>");
        }

        let html = format!(
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\
<style>pre{{font-family:monospace;line-height:1;white-space:pre;}}</style>\
</head><body><pre>{body}</pre></body></html>\n"
        );
        Ok(html.into_bytes())
    }
}

/// Renders each non-space cell's token as SVG `<text>`, laid out on a fixed
/// monospace grid, per `topoglyph-docs/TODO.md` 0.4.0's `DebugSvgEncoder`.
/// Unlike the HTML/plain-text/ANSI encoders, this fixes glyph positioning
/// with explicit `x`/`y` attributes per cell rather than relying on
/// monospace line-wrapping, which makes it suitable as a debug overlay
/// (e.g. for visually diffing two match results cell-by-cell).
pub struct DebugSvgEncoder {
    /// Width in SVG user units of one grid cell.
    pub cell_width: f32,
    /// Height in SVG user units of one grid cell.
    pub cell_height: f32,
}

impl Default for DebugSvgEncoder {
    fn default() -> Self {
        Self {
            cell_width: 10.0,
            cell_height: 18.0,
        }
    }
}

impl DebugSvgEncoder {
    pub fn new(cell_width: f32, cell_height: f32) -> Self {
        Self {
            cell_width,
            cell_height,
        }
    }
}

fn escape_xml(input: &str) -> String {
    escape_html(input)
}

impl TextEncoder for DebugSvgEncoder {
    type Error = std::io::Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error> {
        let svg_width = canvas.width as f32 * self.cell_width;
        let svg_height = canvas.height as f32 * self.cell_height;

        let mut body = String::new();
        body.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{svg_width}\" height=\"{svg_height}\" viewBox=\"0 0 {svg_width} {svg_height}\">\n"
        ));
        body.push_str(&format!(
            "<rect width=\"{svg_width}\" height=\"{svg_height}\" fill=\"white\"/>\n"
        ));

        for (i, cell) in canvas.cells.iter().enumerate() {
            if cell.token == " " || cell.token.is_empty() {
                continue;
            }
            let col = i % canvas.width;
            let row = i / canvas.width;
            let x = col as f32 * self.cell_width;
            // Baseline near the bottom of the cell rather than its top, so
            // glyphs sit visually "on" their cell like normal text.
            let y = (row as f32 + 1.0) * self.cell_height - self.cell_height * 0.25;
            let fill = cell.color.clone().unwrap_or_else(|| "black".to_string());

            body.push_str(&format!(
                "<text x=\"{x}\" y=\"{y}\" font-family=\"monospace\" font-size=\"{}\" fill=\"{}\">{}</text>\n",
                self.cell_height,
                escape_xml(&fill),
                escape_xml(&cell.token)
            ));
        }

        body.push_str("</svg>\n");
        Ok(body.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use topoglyph_core::canvas::TextCell;

    fn canvas_2x2(tokens: [&str; 4], colors: [Option<&str>; 4]) -> TextCanvas {
        TextCanvas {
            width: 2,
            height: 2,
            cells: tokens
                .iter()
                .zip(colors.iter())
                .map(|(token, color)| TextCell {
                    token: token.to_string(),
                    score: 0.0,
                    source_path: None,
                    color: color.map(|c| c.to_string()),
                })
                .collect(),
        }
    }

    #[test]
    fn plain_text_encoder_wraps_at_canvas_width() {
        let canvas = canvas_2x2(["a", "b", "c", "d"], [None, None, None, None]);
        let out = PlainTextEncoder::new().encode(&canvas).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ab\ncd\n");
    }

    #[test]
    fn ansi_encoder_parses_valid_hex_color() {
        assert_eq!(
            AnsiEncoder::parse_hex_color("#ff8000"),
            Some((0xff, 0x80, 0x00))
        );
    }

    #[test]
    fn ansi_encoder_rejects_short_hex_color() {
        assert_eq!(AnsiEncoder::parse_hex_color("#fff"), None);
    }

    #[test]
    fn ansi_encoder_rejects_non_hex_characters() {
        assert_eq!(AnsiEncoder::parse_hex_color("#zzzzzz"), None);
    }

    #[test]
    fn ansi_encoder_emits_color_escape_for_colored_cell() {
        let canvas = canvas_2x2(["a", "b", "c", "d"], [Some("#ff0000"), None, None, None]);
        let out = AnsiEncoder::new().encode(&canvas).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[38;2;255;0;0m"));
        // The color turns off again at cell 1 (see
        // `ansi_encoder_resets_color_before_end_of_row_when_it_changes_mid_row`),
        // so the reset for *this* run lands right after 'a', not at the
        // very end of the string.
        assert!(text.contains("\x1b[38;2;255;0;0ma\x1b[0m"));
    }

    #[test]
    fn ansi_encoder_resets_active_color_before_final_newline_not_after() {
        // Regression test for the "output drifts down one row per frame"
        // bug: a trailing reset appended *after* the last row's '\n'
        // becomes its own phantom line once callers split on `.lines()`
        // (see `topoglyph-cli`'s `run_play`), silently growing every
        // colored frame by one extra printed row.
        let canvas = canvas_2x2(["a", "b", "c", "d"], [None, None, None, Some("#00ff00")]);
        let out = AnsiEncoder::new().encode(&canvas).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2, "reset must not add a phantom line");
        assert!(text.ends_with("\x1b[0m\n"));
    }

    #[test]
    fn ansi_encoder_emits_no_trailing_reset_when_color_already_off() {
        // When the active color already turned off before the final cell,
        // there is nothing left to reset — the encoder must not append an
        // unconditional reset that the caller would then see as an empty
        // trailing line.
        let canvas = canvas_2x2(["a", "b", "c", "d"], [Some("#ff0000"), None, None, None]);
        let out = AnsiEncoder::new().encode(&canvas).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.ends_with("\x1b[0m\n\x1b[0m"));
    }

    fn canvas_2x2_scored(tokens: [&str; 4], scores: [f32; 4]) -> TextCanvas {
        TextCanvas {
            width: 2,
            height: 2,
            cells: tokens
                .iter()
                .zip(scores.iter())
                .enumerate()
                .map(|(i, (token, &score))| TextCell {
                    token: token.to_string(),
                    score,
                    source_path: Some(i),
                    color: None,
                })
                .collect(),
        }
    }

    #[test]
    fn json_debug_encoder_round_trips_scores_and_source_path() {
        let canvas = canvas_2x2_scored(["a", "b", "c", "d"], [1.5, 2.5, 0.0, 3.25]);
        let out = JsonDebugEncoder::new().encode(&canvas).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["width"], 2);
        assert_eq!(parsed["height"], 2);
        assert_eq!(parsed["cells"][0]["token"], "a");
        assert_eq!(parsed["cells"][1]["score"], 2.5);
        assert_eq!(parsed["cells"][3]["source_path"], 3);
    }

    #[test]
    fn json_debug_encoder_preserves_null_source_path() {
        let canvas = canvas_2x2(["a", "b", "c", "d"], [None, None, None, None]);
        let out = JsonDebugEncoder::new().encode(&canvas).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed["cells"][0]["source_path"].is_null());
        assert!(parsed["cells"][0]["color"].is_null());
    }

    #[test]
    fn html_encoder_wraps_in_pre_and_escapes_reserved_characters() {
        let canvas = canvas_2x2(["<", ">", "&", "d"], [None, None, None, None]);
        let out = HtmlEncoder::new().encode(&canvas).unwrap();
        let html = String::from_utf8(out).unwrap();
        assert!(html.contains("<pre>"));
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
        assert!(html.contains("&amp;"));
        assert!(!html.contains("<pre><"));
    }

    #[test]
    fn html_encoder_wraps_colored_run_in_a_single_span() {
        let canvas = canvas_2x2(
            ["a", "b", "c", "d"],
            [Some("#00ff00"), Some("#00ff00"), None, None],
        );
        let out = HtmlEncoder::new().encode(&canvas).unwrap();
        let html = String::from_utf8(out).unwrap();
        assert_eq!(html.matches("<span").count(), 1);
        assert!(html.contains("color:#00ff00"));
    }

    #[test]
    fn debug_svg_encoder_emits_one_text_element_per_nonspace_cell() {
        let canvas = canvas_2x2(["a", " ", "c", "d"], [None, None, None, None]);
        let out = DebugSvgEncoder::default().encode(&canvas).unwrap();
        let svg = String::from_utf8(out).unwrap();
        assert!(svg.starts_with("<svg"));
        assert_eq!(svg.matches("<text").count(), 3);
    }

    #[test]
    fn debug_svg_encoder_sizes_canvas_from_cell_dimensions() {
        let canvas = canvas_2x2(["a", "b", "c", "d"], [None, None, None, None]);
        let encoder = DebugSvgEncoder::new(12.0, 20.0);
        let out = encoder.encode(&canvas).unwrap();
        let svg = String::from_utf8(out).unwrap();
        assert!(svg.contains("width=\"24\""));
        assert!(svg.contains("height=\"40\""));
    }
}
