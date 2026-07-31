/// Final output text canvas representation.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCanvas {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<TextCell>,
}

/// A finalized cell containing the chosen token.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCell {
    pub token: String,
    pub score: f32,
    pub source_path: Option<usize>,
    pub color: Option<String>,
}
