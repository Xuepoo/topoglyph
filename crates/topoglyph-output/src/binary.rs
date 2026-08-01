//! Compact binary `.tglyph` v2 encoding.
//!
//! Real animation content analysis (a full `topoglyph video` run on
//! bad-apple.mp4, 6572 frames, 120x45 grid) found that the v1 text format's
//! `<row>,<col>,<token>` delta lines spend ~90% of their bytes on decimal
//! row/col digits and separator commas, with the actual token data making
//! up only ~10% — and across the whole animation, only 19 distinct tokens
//! (out of a 120x45=5400-cell grid) ever appear. v2 exploits both facts:
//!
//! - Every distinct token used anywhere in the animation gets a single
//!   dictionary entry, referenced by a small varint index instead of
//!   repeating the UTF-8 token bytes at every occurrence.
//! - Cell positions are stored as a *zigzag varint delta from the previous
//!   changed cell's flat index* (`row * width + col`) within the same
//!   frame, instead of two separate decimal numbers — changed cells tend
//!   to cluster spatially (a moving edge/stroke touches nearby cells), so
//!   consecutive deltas are usually small.
//!
//! Measured on the same bad-apple.mp4 animation, this take the file from
//! 21.0MB (v1 text) to ~4.9MB (v2 binary) -- about 23% of the original
//! size, entirely from denser *positional* encoding (the token/color
//! payload itself was already a small fraction of the v1 file).
//!
//! This is a genuinely different format from v1 (not human-readable, no
//! line-oriented text), so it's versioned via a distinct magic
//! (`TglyphAnimation::decode`/`to_text` auto-detect which of v1/v2 a given
//! byte sequence is and dispatch accordingly -- see that module).
//!
//! # Layout
//!
//! ```text
//! magic:        8 bytes  b"TGLYPHB2"
//! width:        u32 LE
//! height:       u32 LE
//! fps:          f32 LE
//! flags:        u8        (bit 0 = include_color)
//! frame_count:  u32 LE
//! dict_len:     varint    (number of distinct tokens used anywhere)
//! dict:         dict_len entries, each:
//!                 token_byte_len: varint
//!                 token_bytes:    [token_byte_len] UTF-8 bytes
//! frame 0 (full grid, width*height cells in row-major order):
//!   for each cell:
//!     dict_index: varint
//!     [only if include_color] has_color: 1 byte (0/1), then if 1: 3 bytes RGB
//! frame i>0 (delta against frame i-1):
//!   change_count: varint
//!   for each change (sorted by ascending flat cell index):
//!     cell_delta:  zigzag varint (flat_index - previous_flat_index_in_this_frame,
//!                  first entry is relative to -1 so it equals flat_index)
//!     dict_index:  varint
//!     [only if include_color] has_color: 1 byte (0/1), then if 1: 3 bytes RGB
//! ```
//!
//! Varints are unsigned LEB128 (7 payload bits per byte, MSB = continuation
//! flag). Signed deltas use the standard zigzag transform
//! (`(n << 1) ^ (n >> 63)`) before varint-encoding so small negative and
//! positive deltas both cost one byte.

use topoglyph_core::canvas::{TextCanvas, TextCell};

use crate::animation::{TglyphAnimation, TglyphError};

const MAGIC: &[u8; 8] = b"TGLYPHB2";
const FLAG_COLOR: u8 = 0b0000_0001;

/// Returns `true` if `bytes` starts with the v2 binary magic. Callers (see
/// `TglyphAnimation::decode`) use this to dispatch between the v1 text
/// parser and [`decode`] without needing the caller to track which format
/// a given file/byte buffer is.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, TglyphError> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = *bytes
            .get(*pos)
            .ok_or(TglyphError::UnexpectedEof("varint"))?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(TglyphError::MalformedBinary("varint too long"));
        }
    }
    Ok(result)
}

fn zigzag_encode(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, TglyphError> {
    let slice = bytes
        .get(*pos..*pos + 4)
        .ok_or(TglyphError::UnexpectedEof("u32"))?;
    *pos += 4;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, TglyphError> {
    let slice = bytes
        .get(*pos..*pos + 4)
        .ok_or(TglyphError::UnexpectedEof("f32"))?;
    *pos += 4;
    Ok(f32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, TglyphError> {
    let byte = *bytes.get(*pos).ok_or(TglyphError::UnexpectedEof("u8"))?;
    *pos += 1;
    Ok(byte)
}

fn read_bytes<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], TglyphError> {
    let slice = bytes
        .get(*pos..*pos + len)
        .ok_or(TglyphError::UnexpectedEof("bytes"))?;
    *pos += len;
    Ok(slice)
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

fn format_hex_color(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Encodes `anim` to the v2 binary layout described in the module docs.
pub fn encode(anim: &TglyphAnimation) -> Vec<u8> {
    // Build the token dictionary up front: every distinct token used by
    // any cell in any frame, ordered by descending frequency so the most
    // common tokens (typically the empty/space cell) get the smallest
    // varint indices.
    let mut frequency: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for canvas in &anim.frames {
        for cell in &canvas.cells {
            *frequency.entry(cell.token.as_str()).or_insert(0) += 1;
        }
    }
    let mut dict: Vec<&str> = frequency.keys().copied().collect();
    dict.sort_by(|a, b| frequency[b].cmp(&frequency[a]).then_with(|| a.cmp(b)));
    let dict_index: std::collections::HashMap<&str, usize> = dict
        .iter()
        .enumerate()
        .map(|(i, &token)| (token, i))
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(anim.width as u32).to_le_bytes());
    out.extend_from_slice(&(anim.height as u32).to_le_bytes());
    out.extend_from_slice(&anim.fps.to_le_bytes());
    let flags = if anim.include_color { FLAG_COLOR } else { 0 };
    out.push(flags);
    out.extend_from_slice(&(anim.frames.len() as u32).to_le_bytes());

    write_varint(&mut out, dict.len() as u64);
    for token in &dict {
        write_varint(&mut out, token.len() as u64);
        out.extend_from_slice(token.as_bytes());
    }

    let write_cell_color = |out: &mut Vec<u8>, cell: &TextCell| {
        if !anim.include_color {
            return;
        }
        match cell.color.as_deref().and_then(parse_hex_color) {
            Some(rgb) => {
                out.push(1);
                out.extend_from_slice(&rgb);
            }
            None => out.push(0),
        }
    };

    for (i, canvas) in anim.frames.iter().enumerate() {
        if i == 0 {
            for cell in &canvas.cells {
                write_varint(&mut out, dict_index[cell.token.as_str()] as u64);
                write_cell_color(&mut out, cell);
            }
        } else {
            let prev = &anim.frames[i - 1];
            let changed: Vec<usize> = canvas
                .cells
                .iter()
                .zip(prev.cells.iter())
                .enumerate()
                .filter(|(_, (cell, prev_cell))| {
                    cell.token != prev_cell.token
                        || (anim.include_color && cell.color != prev_cell.color)
                })
                .map(|(idx, _)| idx)
                .collect();

            write_varint(&mut out, changed.len() as u64);
            let mut last_idx: i64 = -1;
            for idx in changed {
                let delta = idx as i64 - last_idx;
                last_idx = idx as i64;
                write_varint(&mut out, zigzag_encode(delta));
                write_varint(&mut out, dict_index[canvas.cells[idx].token.as_str()] as u64);
                write_cell_color(&mut out, &canvas.cells[idx]);
            }
        }
    }

    out
}

/// Decodes a v2 binary buffer back into a [`TglyphAnimation`]. Callers
/// should check [`is_binary`] first (or go through
/// `TglyphAnimation::decode`, which does this automatically).
pub fn decode(bytes: &[u8]) -> Result<TglyphAnimation, TglyphError> {
    let mut pos = 0usize;

    if !bytes.starts_with(MAGIC) {
        return Err(TglyphError::MalformedHeader(
            "magic",
            "TGLYPHB2",
            String::from_utf8_lossy(bytes.get(..8).unwrap_or(bytes)).to_string(),
        ));
    }
    pos += MAGIC.len();

    let width = read_u32(bytes, &mut pos)? as usize;
    let height = read_u32(bytes, &mut pos)? as usize;
    let fps = read_f32(bytes, &mut pos)?;
    let flags = read_u8(bytes, &mut pos)?;
    let include_color = flags & FLAG_COLOR != 0;
    let frame_count = read_u32(bytes, &mut pos)? as usize;

    let dict_len = read_varint(bytes, &mut pos)? as usize;
    let mut dict: Vec<String> = Vec::with_capacity(dict_len);
    for _ in 0..dict_len {
        let token_len = read_varint(bytes, &mut pos)? as usize;
        let token_bytes = read_bytes(bytes, &mut pos, token_len)?;
        let token = std::str::from_utf8(token_bytes)
            .map_err(|_| TglyphError::MalformedBinary("dictionary entry is not valid UTF-8"))?
            .to_string();
        dict.push(token);
    }

    let cell_count = width * height;
    let mut frames: Vec<TextCanvas> = Vec::with_capacity(frame_count);

    let read_cell_color = |bytes: &[u8], pos: &mut usize| -> Result<Option<String>, TglyphError> {
        if !include_color {
            return Ok(None);
        }
        let has_color = read_u8(bytes, pos)?;
        if has_color == 0 {
            return Ok(None);
        }
        let rgb = read_bytes(bytes, pos, 3)?;
        Ok(Some(format_hex_color([rgb[0], rgb[1], rgb[2]])))
    };

    for i in 0..frame_count {
        if i == 0 {
            let mut cells = Vec::with_capacity(cell_count);
            for _ in 0..cell_count {
                let idx = read_varint(bytes, &mut pos)? as usize;
                let token = dict
                    .get(idx)
                    .ok_or(TglyphError::MalformedBinary("dictionary index out of range"))?
                    .clone();
                let color = read_cell_color(bytes, &mut pos)?;
                cells.push(TextCell {
                    token,
                    score: 0.0,
                    source_path: None,
                    color,
                });
            }
            frames.push(TextCanvas {
                width,
                height,
                cells,
            });
        } else {
            let mut canvas = frames[i - 1].clone();
            let change_count = read_varint(bytes, &mut pos)? as usize;
            let mut last_idx: i64 = -1;
            for _ in 0..change_count {
                let zz = read_varint(bytes, &mut pos)?;
                let delta = zigzag_decode(zz);
                let cell_idx = last_idx + delta;
                last_idx = cell_idx;
                if cell_idx < 0 || cell_idx as usize >= cell_count {
                    return Err(TglyphError::DeltaOutOfRange(
                        i,
                        cell_idx.max(0) as usize / width.max(1),
                        cell_idx.max(0) as usize % width.max(1),
                        width,
                        height,
                    ));
                }
                let dict_idx = read_varint(bytes, &mut pos)? as usize;
                let token = dict
                    .get(dict_idx)
                    .ok_or(TglyphError::MalformedBinary("dictionary index out of range"))?
                    .clone();
                let color = read_cell_color(bytes, &mut pos)?;
                let cell = &mut canvas.cells[cell_idx as usize];
                cell.token = token;
                if include_color {
                    cell.color = color;
                }
            }
            frames.push(canvas);
        }
    }

    Ok(TglyphAnimation {
        width,
        height,
        fps,
        include_color,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use topoglyph_core::canvas::{TextCanvas, TextCell};

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
    fn round_trip_single_frame() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let anim = TglyphAnimation::encode(std::slice::from_ref(&f0), 24.0, false).unwrap();
        let bytes = encode(&anim);
        assert!(is_binary(&bytes));

        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.fps, 24.0);
        assert!(!decoded.include_color);
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
    fn round_trip_multi_frame_with_deltas() {
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let f1 = canvas(&["a", "x", "c", "d"], 2, None);
        let f2 = canvas(&["a", "x", "y", "d"], 2, None);
        let anim = TglyphAnimation::encode(&[f0.clone(), f1.clone(), f2.clone()], 24.0, false)
            .unwrap();
        let bytes = encode(&anim);
        let decoded = decode(&bytes).unwrap();

        assert_eq!(decoded.frames.len(), 3);
        let tokens = |c: &TextCanvas| c.cells.iter().map(|c| c.token.clone()).collect::<Vec<_>>();
        assert_eq!(tokens(&decoded.frames[0]), tokens(&f0));
        assert_eq!(tokens(&decoded.frames[1]), tokens(&f1));
        assert_eq!(tokens(&decoded.frames[2]), tokens(&f2));
    }

    #[test]
    fn round_trip_with_color() {
        let f0 = canvas(&["a", "b"], 2, Some(&[Some("#ff0000"), None]));
        let f1 = canvas(&["a", "b"], 2, Some(&[Some("#00ff00"), None]));
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, true).unwrap();
        let bytes = encode(&anim);
        let decoded = decode(&bytes).unwrap();

        assert!(decoded.include_color);
        assert_eq!(decoded.frames[0].cells[0].color.as_deref(), Some("#ff0000"));
        assert_eq!(decoded.frames[1].cells[0].color.as_deref(), Some("#00ff00"));
    }

    #[test]
    fn space_token_round_trips() {
        let f0 = canvas(&["a", " "], 2, None);
        let anim = TglyphAnimation::encode(std::slice::from_ref(&f0), 24.0, false).unwrap();
        let bytes = encode(&anim);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.frames[0].cells[1].token, " ");
    }

    #[test]
    fn is_binary_rejects_text_format() {
        assert!(!is_binary(b"TOPOGLYPH-ANIM v1\nWIDTH 2\n"));
    }

    #[test]
    fn decode_rejects_wrong_magic() {
        let err = decode(b"NOTBINARY").unwrap_err();
        assert!(matches!(err, TglyphError::MalformedHeader("magic", _, _)));
    }

    #[test]
    fn binary_encoding_is_smaller_than_text_for_sparse_deltas() {
        // A grid where only one cell changes between frames should show
        // the binary format's positional-encoding win: the delta line
        // itself is now a handful of varint bytes instead of decimal
        // "row,col,token".
        let f0 = canvas(&["a"; 400], 20, None);
        let mut f1_tokens = vec!["a"; 400];
        f1_tokens[199] = "b";
        let f1 = canvas(&f1_tokens, 20, None);
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();

        let text_len = anim.to_text().len();
        let binary_len = encode(&anim).len();
        assert!(
            binary_len < text_len,
            "binary ({binary_len}) should be smaller than text ({text_len})"
        );
    }

    #[test]
    fn round_trip_preserves_exact_byte_length_determinism() {
        // Encoding the same animation twice must produce byte-identical
        // output (dictionary ordering must be deterministic, not
        // HashMap-iteration-order-dependent), so `.tglyph` files are
        // reproducible across runs.
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let anim = TglyphAnimation::encode(std::slice::from_ref(&f0), 24.0, false).unwrap();
        assert_eq!(encode(&anim), encode(&anim));
    }
}
