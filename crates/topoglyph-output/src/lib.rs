pub mod animation;
pub mod binary;
pub mod encoder;
pub mod stream;

#[cfg(test)]
mod streaming_contract_tests {
    use std::io::Cursor;

    use topoglyph_core::canvas::{TextCanvas, TextCell};

    use crate::animation::TglyphAnimation;
    use crate::stream::{AnimationFormat, StreamingEncoder};

    fn canvas(tokens: &[&str]) -> TextCanvas {
        TextCanvas {
            width: tokens.len(),
            height: 1,
            cells: tokens
                .iter()
                .map(|token| TextCell {
                    token: (*token).to_string(),
                    score: 0.0,
                    source_path: None,
                    color: None,
                })
                .collect(),
        }
    }

    #[test]
    fn binary_streaming_encoder_round_trips_ordered_frames() {
        let writer = Cursor::new(Vec::new());
        let mut encoder = StreamingEncoder::new(
            writer,
            24.0,
            false,
            AnimationFormat::Binary,
            [" ", "a", "b"].map(str::to_string),
        )
        .unwrap();

        encoder.push_frame(canvas(&["a", " "])).unwrap();
        encoder.push_frame(canvas(&["a", "b"])).unwrap();
        encoder.push_frame(canvas(&[" ", "b"])).unwrap();
        let bytes = encoder.finish().unwrap().into_inner();

        let decoded = TglyphAnimation::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.frames.len(), 3);
        assert_eq!(decoded.frames[0], canvas(&["a", " "]));
        assert_eq!(decoded.frames[1], canvas(&["a", "b"]));
        assert_eq!(decoded.frames[2], canvas(&[" ", "b"]));
    }

    #[test]
    fn text_streaming_encoder_round_trips_ordered_frames() {
        let writer = Cursor::new(Vec::new());
        let mut encoder = StreamingEncoder::new(
            writer,
            30.0,
            false,
            AnimationFormat::Text,
            std::iter::empty(),
        )
        .unwrap();

        encoder.push_frame(canvas(&["a", " "])).unwrap();
        encoder.push_frame(canvas(&[" ", "b"])).unwrap();
        let bytes = encoder.finish().unwrap().into_inner();

        let decoded = TglyphAnimation::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.frames[0], canvas(&["a", " "]));
        assert_eq!(decoded.frames[1], canvas(&[" ", "b"]));
    }
}
