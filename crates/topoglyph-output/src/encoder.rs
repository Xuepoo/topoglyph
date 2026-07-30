use topoglyph_core::canvas::TextCanvas;

pub trait TextEncoder {
    type Error;

    fn encode(&self, canvas: &TextCanvas) -> Result<Vec<u8>, Self::Error>;
}

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

pub struct AnsiEncoder;

impl AnsiEncoder {
    pub fn new() -> Self { Self }
    
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
            if (i + 1) % canvas.width == 0 {
                result.push('\n');
            }
        }
        result.push_str("\x1b[0m"); // reset at end
        Ok(result.into_bytes())
    }
}
