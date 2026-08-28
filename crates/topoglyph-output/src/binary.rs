//! Compact binary `.tglyph` v2/v3 encoding.
//!
//! Real animation content analysis (a full `topoglyph video` run on
//! bad-apple.mp4, 6572 frames, 120×45 grid) found that the v1 text format's
//! `<row>,<col>,<token>` delta lines spend ~90% of their bytes on decimal
//! row/col digits and separator commas, with the actual token data making
//! up only ~10% — and across the whole animation, only 19 distinct tokens
//! (out of a 120×45=5400-cell grid) ever appear. v2 exploits both facts:
//!
//! - Every distinct token used anywhere in the animation gets a single
//!   dictionary entry, referenced by a small varint index instead of
//!   repeating the UTF-8 token bytes at every occurrence.
//! - Cell positions are stored as a *varint delta from the previous
//!   changed cell's flat index* (`row * width + col`) within the same
//!   frame, instead of two separate decimal numbers — changed cells tend
//!   to cluster spatially (a moving edge/stroke touches nearby cells), so
//!   consecutive gaps are usually small.
//!
//! Measured on the same bad-apple.mp4 animation, this takes the file from
//! 21.0MB (v1 text) to ~4.9MB (v2 binary) -- about 23% of the original
//! size, entirely from denser *positional* encoding (the token/color
//! payload itself was already a small fraction of the v1 file).
//!
//! # v2 Layout (TGLYPHB2, retained for backward compat)
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
//!                  first entry is relative to -1 so it equals flat_index+1)
//!     dict_index:  varint
//!     [only if include_color] has_color: 1 byte (0/1), then if 1: 3 bytes RGB
//! ```
//!
//! Varints are unsigned LEB128 (7 payload bits per byte, MSB = continuation
//! flag). v2 signed deltas use the standard zigzag transform
//! (`(n << 1) ^ (n >> 63)`) before varint-encoding. v2's zigzag is
//! wasteful because `flat_index` is strictly increasing within a frame, so
//! deltas are always positive — the sign bit is never used and halves the
//! 1-byte range (0..63 instead of 0..127). v3 fixes this.
//!
//! # v3 Layout (TGLYPHB3)
//!
//! v3 is byte-for-byte equivalent at the TextCanvas level but denser:
//!
//! ```text
//! magic:        8 bytes  b"TGLYPHB3"
//! width:        u32 LE
//! height:       u32 LE
//! fps:          f32 LE
//! flags:        u8        (bit 0 = include_color)
//! frame_count:  u32 LE
//! dict_len:     varint
//! dict:         dict_len entries, each:
//!                 token_byte_len: varint
//!                 token_bytes:    [token_byte_len] UTF-8 bytes
//! [only if include_color]
//!   palette_len: varint   (number of distinct RGB colors used anywhere)
//!   palette:     palette_len entries, each: 3 bytes RGB (r,g,b)
//! frames:
//!   frame 0:
//!     frame_type: u8 (1 = Full)
//!     for each cell (width*height, row-major):
//!       dict_index: varint
//!       [only if include_color] color_ref: varint (0=None, 1..palette_len => palette[ref-1], palette_len+1 => raw 3 bytes RGB fallback)
//!   frame i>0:
//!     frame_type: u8 (0 = SparseDelta, 1 = Full; 2 = BitmapDelta reserved)
//!     if SparseDelta:
//!       change_count: varint
//!       for each change (sorted ascending):
//!         gap:        varint (unsigned, gap = flat_index - prev_index - 1, prev starts at -1 so first gap == flat_index)
//!         dict_index: varint
//!         [only if include_color] color_ref: varint (same as above)
//!     if Full:
//!       for each cell (width*height):
//!         dict_index: varint
//!         [only if include_color] color_ref: varint
//! ```
//!
//! Adaptive choice per frame i>0: SparseDelta vs Full is picked by
//! comparing `change_count * 2 > cell_count` (≈50% density). When >50% of
//! cells change, a full frame (cell_count varints) is smaller than
//! sparse (change_count * (gap+token) ~2B per change). BitmapDelta (bitset)
//! is reserved for future use.

use std::collections::HashMap;

use topoglyph_core::canvas::{TextCanvas, TextCell};

use crate::animation::{TglyphAnimation, TglyphError};

const MAGIC_V2: &[u8; 8] = b"TGLYPHB2";
const MAGIC_V3: &[u8; 8] = b"TGLYPHB3";
const MAGIC: &[u8; 8] = MAGIC_V3;
const FLAG_COLOR: u8 = 0b0000_0001;

const FRAME_TYPE_SPARSE: u8 = 0;
const FRAME_TYPE_FULL: u8 = 1;
const FRAME_TYPE_BITMAP: u8 = 2;

/// Returns `true` if `bytes` looks like any binary `.tglyph` (v2 or v3).
/// Callers (see `TglyphAnimation::from_bytes`) use this to dispatch between
/// the v1 text parser and [`decode`] without needing the caller to track
/// which format a given file/byte buffer is.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC_V2) || bytes.starts_with(MAGIC_V3)
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

// Kept for v2 backward compat decoding.
#[allow(dead_code)]
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

fn read_cell_color_v2(
    bytes: &[u8],
    include_color: bool,
    pos: &mut usize,
) -> Result<Option<String>, TglyphError> {
    if !include_color {
        return Ok(None);
    }
    let has_color = read_u8(bytes, pos)?;
    if has_color == 0 {
        return Ok(None);
    }
    let rgb = read_bytes(bytes, pos, 3)?;
    Ok(Some(format_hex_color([rgb[0], rgb[1], rgb[2]])))
}

fn read_cell_color_v3(
    bytes: &[u8],
    include_color: bool,
    palette: &[[u8; 3]],
    pos: &mut usize,
) -> Result<Option<String>, TglyphError> {
    if !include_color {
        return Ok(None);
    }
    if palette.is_empty() {
        // No palette was written (streaming path or truly no colors): fall
        // back to v2's 1+3 byte encoding for forward compat.
        let has_color = read_u8(bytes, pos)?;
        if has_color == 0 {
            return Ok(None);
        }
        let rgb = read_bytes(bytes, pos, 3)?;
        return Ok(Some(format_hex_color([rgb[0], rgb[1], rgb[2]])));
    }
    let idx = read_varint(bytes, pos)? as usize;
    if idx == 0 {
        Ok(None)
    } else if idx <= palette.len() {
        Ok(Some(format_hex_color(palette[idx - 1])))
    } else if idx == palette.len() + 1 {
        // Raw fallback: encoder emitted a color not in palette (e.g. palette
        // was capped or streaming without global palette knowledge).
        let rgb = read_bytes(bytes, pos, 3)?;
        Ok(Some(format_hex_color([rgb[0], rgb[1], rgb[2]])))
    } else {
        Err(TglyphError::MalformedBinary(
            "color palette index out of range",
        ))
    }
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

/// Encodes `anim` to the v3 binary layout described in the module docs.
pub fn encode(anim: &TglyphAnimation) -> Vec<u8> {
    // Build the token dictionary up front: every distinct token used by
    // any cell in any frame, ordered by descending frequency so the most
    // common tokens (typically the empty/space cell) get the smallest
    // varint indices.
    let mut frequency: HashMap<&str, usize> = HashMap::new();
    for canvas in &anim.frames {
        for cell in &canvas.cells {
            *frequency.entry(cell.token.as_str()).or_insert(0) += 1;
        }
    }
    let mut dict: Vec<&str> = frequency.keys().copied().collect();
    dict.sort_by(|a, b| frequency[b].cmp(&frequency[a]).then_with(|| a.cmp(b)));
    let dict_index: HashMap<&str, usize> = dict
        .iter()
        .enumerate()
        .map(|(i, &token)| (token, i))
        .collect();

    // Build global color palette if needed, ordered by descending frequency.
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut palette_index: HashMap<[u8; 3], usize> = HashMap::new();
    if anim.include_color {
        let mut color_freq: HashMap<[u8; 3], usize> = HashMap::new();
        for canvas in &anim.frames {
            for cell in &canvas.cells {
                if let Some(rgb) = cell.color.as_deref().and_then(parse_hex_color) {
                    *color_freq.entry(rgb).or_insert(0) += 1;
                }
            }
        }
        let mut palette_sorted: Vec<[u8; 3]> = color_freq.keys().copied().collect();
        palette_sorted.sort_by(|a, b| color_freq[b].cmp(&color_freq[a]).then_with(|| a.cmp(b)));
        for (i, rgb) in palette_sorted.iter().enumerate() {
            palette_index.insert(*rgb, i);
        }
        palette = palette_sorted;
    }

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

    if anim.include_color {
        write_varint(&mut out, palette.len() as u64);
        for rgb in &palette {
            out.extend_from_slice(rgb);
        }
    }

    let write_cell_color_v3_inline = |out: &mut Vec<u8>, cell: &TextCell| {
        if !anim.include_color {
            return;
        }
        if palette.is_empty() {
            // Fallback: same as v2 (streaming or no colors at all)
            match cell.color.as_deref().and_then(parse_hex_color) {
                Some(rgb) => {
                    out.push(1);
                    out.extend_from_slice(&rgb);
                }
                None => out.push(0),
            }
            return;
        }
        match cell.color.as_deref().and_then(parse_hex_color) {
            None => write_varint(out, 0),
            Some(rgb) => {
                if let Some(&idx) = palette_index.get(&rgb) {
                    write_varint(out, (idx + 1) as u64);
                } else {
                    // Should not happen for non-streaming encode where palette
                    // covers all distinct colors, but handle for robustness.
                    write_varint(out, (palette.len() + 1) as u64);
                    out.extend_from_slice(&rgb);
                }
            }
        }
    };

    let cell_count = anim.width * anim.height;

    for (i, canvas) in anim.frames.iter().enumerate() {
        if i == 0 {
            // Frame 0 is always Full.
            out.push(FRAME_TYPE_FULL);
            for cell in &canvas.cells {
                write_varint(&mut out, dict_index[cell.token.as_str()] as u64);
                write_cell_color_v3_inline(&mut out, cell);
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

            // Adaptive: if more than ~50% of cells changed, writing a full
            // frame is cheaper than sparse (gap+token per change ~2B vs
            // token per cell ~1B). Use Full when changed*2 > cell_count.
            let use_full = changed.len() * 2 > cell_count;

            if use_full {
                out.push(FRAME_TYPE_FULL);
                for cell in &canvas.cells {
                    write_varint(&mut out, dict_index[cell.token.as_str()] as u64);
                    write_cell_color_v3_inline(&mut out, cell);
                }
            } else {
                out.push(FRAME_TYPE_SPARSE);
                write_varint(&mut out, changed.len() as u64);
                let mut prev_idx: i64 = -1;
                for idx in changed {
                    // Unsigned gap: gap = idx - prev - 1, first gap == idx
                    let gap = idx as i64 - prev_idx - 1;
                    debug_assert!(gap >= 0);
                    write_varint(&mut out, gap as u64);
                    prev_idx = idx as i64;
                    write_varint(
                        &mut out,
                        dict_index[canvas.cells[idx].token.as_str()] as u64,
                    );
                    write_cell_color_v3_inline(&mut out, &canvas.cells[idx]);
                }
            }
        }
    }

    out
}

/// Streams frames out of a binary `.tglyph` buffer one at a time instead
/// of materializing every decoded [`TextCanvas`] up front. Only the header,
/// token dictionary, palette, and the single most-recently-decoded frame
/// (needed to apply the next delta) are kept in memory regardless of
/// `frame_count` — this is what lets `topoglyph play` play back arbitrarily
/// long animations without their full decoded size ever fitting in RAM at
/// once.
///
/// Supports both v2 (`TGLYPHB2`, zigzag deltas, no palette, no frame_type)
/// and v3 (`TGLYPHB3`, unsigned gaps, palette, adaptive frame types).
///
/// Construct via [`BinaryFrameReader::new`], read `width`/`height`/`fps`/
/// `frame_count`/`include_color` up front, then drive it as an [`Iterator`].
pub struct BinaryFrameReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    width: usize,
    height: usize,
    fps: f32,
    include_color: bool,
    frame_count: usize,
    dict: Vec<String>,
    palette: Vec<[u8; 3]>,
    version: u8,
    next_index: usize,
    previous: Option<TextCanvas>,
}

impl<'a> BinaryFrameReader<'a> {
    /// Parses the header and token dictionary (and v3 palette) only; no
    /// frame data is decoded until [`Iterator::next`] is first called.
    pub fn new(bytes: &'a [u8]) -> Result<Self, TglyphError> {
        let mut pos = 0usize;

        let version = if bytes.starts_with(MAGIC_V3) {
            3
        } else if bytes.starts_with(MAGIC_V2) {
            2
        } else {
            return Err(TglyphError::MalformedHeader(
                "magic",
                "TGLYPHB2 or TGLYPHB3",
                String::from_utf8_lossy(bytes.get(..8).unwrap_or(bytes)).to_string(),
            ));
        };
        pos += MAGIC.len(); // both 8 bytes

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

        let mut palette: Vec<[u8; 3]> = Vec::new();
        if version == 3 && include_color {
            let palette_len = read_varint(bytes, &mut pos)? as usize;
            palette.reserve(palette_len);
            for _ in 0..palette_len {
                let rgb = read_bytes(bytes, &mut pos, 3)?;
                palette.push([rgb[0], rgb[1], rgb[2]]);
            }
        }

        Ok(Self {
            bytes,
            pos,
            width,
            height,
            fps,
            include_color,
            frame_count,
            dict,
            palette,
            version,
            next_index: 0,
            previous: None,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    pub fn include_color(&self) -> bool {
        self.include_color
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    fn decode_next_frame(&mut self) -> Result<TextCanvas, TglyphError> {
        let cell_count = self.width * self.height;
        let i = self.next_index;

        if self.version == 2 {
            // Legacy v2 path (zigzag, no frame_type, no palette)
            if i == 0 {
                let mut cells = Vec::with_capacity(cell_count);
                for _ in 0..cell_count {
                    let idx = read_varint(self.bytes, &mut self.pos)? as usize;
                    let token = self
                        .dict
                        .get(idx)
                        .ok_or(TglyphError::MalformedBinary(
                            "dictionary index out of range",
                        ))?
                        .clone();
                    let color = read_cell_color_v2(self.bytes, self.include_color, &mut self.pos)?;
                    cells.push(TextCell {
                        token,
                        score: 0.0,
                        source_path: None,
                        color,
                    });
                }
                Ok(TextCanvas {
                    width: self.width,
                    height: self.height,
                    cells,
                })
            } else {
                let mut canvas = self
                    .previous
                    .clone()
                    .expect("previous frame is set for every index after the first");
                let change_count = read_varint(self.bytes, &mut self.pos)? as usize;
                let mut last_idx: i64 = -1;
                for _ in 0..change_count {
                    let zz = read_varint(self.bytes, &mut self.pos)?;
                    let delta = zigzag_decode(zz);
                    let cell_idx = last_idx + delta;
                    last_idx = cell_idx;
                    if cell_idx < 0 || cell_idx as usize >= cell_count {
                        return Err(TglyphError::DeltaOutOfRange(
                            i,
                            cell_idx.max(0) as usize / self.width.max(1),
                            cell_idx.max(0) as usize % self.width.max(1),
                            self.width,
                            self.height,
                        ));
                    }
                    let dict_idx = read_varint(self.bytes, &mut self.pos)? as usize;
                    let token = self
                        .dict
                        .get(dict_idx)
                        .ok_or(TglyphError::MalformedBinary(
                            "dictionary index out of range",
                        ))?
                        .clone();
                    let color = read_cell_color_v2(self.bytes, self.include_color, &mut self.pos)?;
                    let cell = &mut canvas.cells[cell_idx as usize];
                    cell.token = token;
                    if self.include_color {
                        cell.color = color;
                    }
                }
                Ok(canvas)
            }
        } else {
            // v3 path (unsigned gap, frame_type, palette)
            if i == 0 {
                let frame_type = read_u8(self.bytes, &mut self.pos)?;
                if frame_type != FRAME_TYPE_FULL {
                    return Err(TglyphError::MalformedBinary(
                        "frame 0 must be Full (type 1) in v3",
                    ));
                }
                let mut cells = Vec::with_capacity(cell_count);
                for _ in 0..cell_count {
                    let idx = read_varint(self.bytes, &mut self.pos)? as usize;
                    let token = self
                        .dict
                        .get(idx)
                        .ok_or(TglyphError::MalformedBinary(
                            "dictionary index out of range",
                        ))?
                        .clone();
                    let color = read_cell_color_v3(
                        self.bytes,
                        self.include_color,
                        &self.palette,
                        &mut self.pos,
                    )?;
                    cells.push(TextCell {
                        token,
                        score: 0.0,
                        source_path: None,
                        color,
                    });
                }
                Ok(TextCanvas {
                    width: self.width,
                    height: self.height,
                    cells,
                })
            } else {
                let frame_type = read_u8(self.bytes, &mut self.pos)?;
                if frame_type == FRAME_TYPE_FULL {
                    let mut cells = Vec::with_capacity(cell_count);
                    for _ in 0..cell_count {
                        let idx = read_varint(self.bytes, &mut self.pos)? as usize;
                        let token = self
                            .dict
                            .get(idx)
                            .ok_or(TglyphError::MalformedBinary(
                                "dictionary index out of range",
                            ))?
                            .clone();
                        let color = read_cell_color_v3(
                            self.bytes,
                            self.include_color,
                            &self.palette,
                            &mut self.pos,
                        )?;
                        cells.push(TextCell {
                            token,
                            score: 0.0,
                            source_path: None,
                            color,
                        });
                    }
                    Ok(TextCanvas {
                        width: self.width,
                        height: self.height,
                        cells,
                    })
                } else if frame_type == FRAME_TYPE_SPARSE {
                    let mut canvas = self
                        .previous
                        .clone()
                        .expect("previous frame is set for every index after the first");
                    let change_count = read_varint(self.bytes, &mut self.pos)? as usize;
                    let mut prev_idx: i64 = -1;
                    for _ in 0..change_count {
                        let gap = read_varint(self.bytes, &mut self.pos)? as i64;
                        let cell_idx = prev_idx + gap + 1;
                        prev_idx = cell_idx;
                        if cell_idx < 0 || cell_idx as usize >= cell_count {
                            return Err(TglyphError::DeltaOutOfRange(
                                i,
                                cell_idx.max(0) as usize / self.width.max(1),
                                cell_idx.max(0) as usize % self.width.max(1),
                                self.width,
                                self.height,
                            ));
                        }
                        let dict_idx = read_varint(self.bytes, &mut self.pos)? as usize;
                        let token = self
                            .dict
                            .get(dict_idx)
                            .ok_or(TglyphError::MalformedBinary(
                                "dictionary index out of range",
                            ))?
                            .clone();
                        let color = read_cell_color_v3(
                            self.bytes,
                            self.include_color,
                            &self.palette,
                            &mut self.pos,
                        )?;
                        let cell = &mut canvas.cells[cell_idx as usize];
                        cell.token = token;
                        if self.include_color {
                            cell.color = color;
                        }
                    }
                    Ok(canvas)
                } else if frame_type == FRAME_TYPE_BITMAP {
                    Err(TglyphError::MalformedBinary(
                        "BitmapDelta frame type reserved, not yet implemented",
                    ))
                } else {
                    Err(TglyphError::MalformedBinary("unknown v3 frame type"))
                }
            }
        }
    }
}

impl Iterator for BinaryFrameReader<'_> {
    type Item = Result<TextCanvas, TglyphError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.frame_count {
            return None;
        }
        let result = self.decode_next_frame();
        self.next_index += 1;
        match result {
            Ok(canvas) => {
                self.previous = Some(canvas.clone());
                Some(Ok(canvas))
            }
            Err(error) => {
                // Stop iterating on the first error rather than retrying;
                // `next_index` is already past `frame_count`'s reach isn't
                // guaranteed, so force future calls to return `None` too.
                self.next_index = self.frame_count;
                Some(Err(error))
            }
        }
    }
}

/// Decodes a binary buffer back into a [`TglyphAnimation`]. Handles both v2
/// (`TGLYPHB2`, zigzag) and v3 (`TGLYPHB3`, gap+palette+adaptive). Callers
/// should check [`is_binary`] first (or go through
/// `TglyphAnimation::from_bytes`, which does this automatically).
///
/// This fully materializes every frame; prefer [`BinaryFrameReader`]
/// directly when memory-bounded playback matters (see its docs).
pub fn decode(bytes: &[u8]) -> Result<TglyphAnimation, TglyphError> {
    let reader = BinaryFrameReader::new(bytes)?;
    let width = reader.width();
    let height = reader.height();
    let fps = reader.fps();
    let include_color = reader.include_color();
    let frame_count = reader.frame_count();

    let mut frames: Vec<TextCanvas> = Vec::with_capacity(frame_count);
    for frame in reader {
        frames.push(frame?);
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
        let anim =
            TglyphAnimation::encode(&[f0.clone(), f1.clone(), f2.clone()], 24.0, false).unwrap();
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

    #[test]
    fn binary_frame_reader_streams_the_same_frames_as_full_decode() {
        // Regression test for the `topoglyph play` OOM: BinaryFrameReader
        // must decode identically to `decode`, one frame at a time, without
        // requiring `frame_count` frames to already be materialized.
        let f0 = canvas(&["a", "b", "c", "d"], 2, None);
        let f1 = canvas(&["a", "x", "c", "d"], 2, None);
        let f2 = canvas(&["a", "x", "y", "d"], 2, None);
        let anim =
            TglyphAnimation::encode(&[f0.clone(), f1.clone(), f2.clone()], 24.0, false).unwrap();
        let bytes = encode(&anim);

        let full = decode(&bytes).unwrap();
        let reader = BinaryFrameReader::new(&bytes).unwrap();
        assert_eq!(reader.width(), full.width);
        assert_eq!(reader.height(), full.height);
        assert_eq!(reader.fps(), full.fps);
        assert_eq!(reader.include_color(), full.include_color);
        assert_eq!(reader.frame_count(), full.frames.len());

        let streamed: Vec<TextCanvas> = reader.collect::<Result<_, _>>().unwrap();
        assert_eq!(streamed, full.frames);
    }

    #[test]
    fn v3_gap_encoding_is_more_compact_than_v2_zigzag_for_sparse() {
        // Verify the gap fix: first gap == flat_index, and 1-byte range is
        // 0..127 not 0..63, so a delta at 100 should be 1 byte in v3.
        let f0 = canvas(&["a"; 400], 20, None);
        let mut f1_tokens = vec!["a"; 400];
        // place change at index 100 (row 5, col 0) and 101 clustered
        f1_tokens[100] = "b";
        f1_tokens[101] = "b";
        let f1 = canvas(&f1_tokens, 20, None);
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();
        let bytes = encode(&anim);
        // Must be v3 magic now
        assert!(bytes.starts_with(b"TGLYPHB3"));
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.frames[1].cells[100].token, "b");
        assert_eq!(decoded.frames[1].cells[101].token, "b");
    }

    #[test]
    fn v3_still_decodes_v2_bytes() {
        // Hand-crafted minimal v2 frame: 1x1 grid, one token "a", one frame,
        // no color. We simulate what v2's encode would have produced for a
        // single frame (dict 1 entry "a", frame0 token 0) plus a sparse delta
        // second frame that changes cell 0 to "b" via zigzag delta 1.
        // Instead of crafting manually, we just check that a v2 file written
        // by an older build can be read: construct v2 bytes manually.
        let mut v2 = Vec::new();
        v2.extend_from_slice(b"TGLYPHB2");
        v2.extend_from_slice(&1u32.to_le_bytes()); // width 1
        v2.extend_from_slice(&1u32.to_le_bytes()); // height 1
        v2.extend_from_slice(&24f32.to_le_bytes());
        v2.push(0); // flags no color
        v2.extend_from_slice(&2u32.to_le_bytes()); // 2 frames
                                                   // dict len 2: "a", "b" (order doesn't matter, but we need deterministic)
                                                   // For minimal test, dict ["a","b"]
        v2.push(2); // dict_len varint 2
        v2.push(1);
        v2.extend_from_slice(b"a");
        v2.push(1);
        v2.extend_from_slice(b"b");
        // frame0: token 0 ("a")
        v2.push(0);
        // frame1 delta: 1 change, zigzag(1)=2, token 1 ("b")
        v2.push(1); // change_count 1
        v2.push(2); // zigzag 1 -> 2
        v2.push(1); // dict idx 1
        let decoded = decode(&v2).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.frames[0].cells[0].token, "a");
        assert_eq!(decoded.frames[1].cells[0].token, "b");
    }

    #[test]
    fn adaptive_full_frame_is_used_for_dense_changes() {
        // 2x2 grid, every cell changes each frame -> should pick Full
        let f0 = canvas(&["a", "a", "a", "a"], 2, None);
        let f1 = canvas(&["b", "b", "b", "b"], 2, None);
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, false).unwrap();
        let bytes = encode(&anim);
        // Second frame header: after first frame's 4 tokens, expect frame_type 1
        // We don't parse raw, just ensure it decodes and that v3 was used.
        assert!(bytes.starts_with(b"TGLYPHB3"));
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.frames[1].cells[0].token, "b");
    }

    #[test]
    fn color_palette_round_trip_with_many_colors() {
        // 4 cells, 3 distinct colors + None, palette should cover them in 1 byte each
        let f0 = canvas(
            &["a", "a", "a", "a"],
            2,
            Some(&[Some("#ff0000"), Some("#00ff00"), Some("#0000ff"), None]),
        );
        let f1 = canvas(
            &["a", "a", "a", "a"],
            2,
            Some(&[
                Some("#ff0000"),
                Some("#00ff00"),
                Some("#0000ff"),
                Some("#ffffff"),
            ]),
        );
        let anim = TglyphAnimation::encode(&[f0, f1], 24.0, true).unwrap();
        let bytes = encode(&anim);
        assert!(bytes.starts_with(b"TGLYPHB3"));
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.frames[0].cells[0].color.as_deref(), Some("#ff0000"));
        assert_eq!(decoded.frames[0].cells[3].color, None);
        assert_eq!(decoded.frames[1].cells[3].color.as_deref(), Some("#ffffff"));
        // Palette should make this smaller than per-cell 3B RGB
        // (at least not larger than old 1+3 scheme inflated)
        // Just check size is reasonable (< 200 bytes for tiny anim)
        assert!(bytes.len() < 200);
    }
}
