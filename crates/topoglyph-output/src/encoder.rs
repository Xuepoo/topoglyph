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
