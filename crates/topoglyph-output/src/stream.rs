//! Incremental `.tglyph` encoding for long animations.
//!
//! Unlike [`crate::animation::TglyphAnimation`], this encoder never retains
//! every full frame. It writes each frame immediately and keeps only the
//! previous canvas needed to compute the next delta.

use std::collections::{HashMap, HashSet};
use std::io::{self, Seek, SeekFrom, Write};

use topoglyph_core::canvas::{TextCanvas, TextCell};

use crate::animation::TglyphError;

const BINARY_MAGIC: &[u8; 8] = b"TGLYPHB2";
const FLAG_COLOR: u8 = 0b0000_0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFormat {
    Binary,
    Text,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Animation(#[from] TglyphError),
    #[error("binary dictionary does not contain token {0:?}")]
    UnknownToken(String),
    #[error("animation has more than {0} frames")]
    TooManyFrames(u32),
}

pub struct StreamingEncoder<W> {
    writer: W,
    fps: f32,
    include_color: bool,
    format: AnimationFormat,
    dictionary: Vec<String>,
    dictionary_index: HashMap<String, u64>,
    width: Option<usize>,
    height: Option<usize>,
    frame_count: u32,
    frame_count_offset: Option<u64>,
    previous: Option<TextCanvas>,
}

impl<W: Write + Seek> StreamingEncoder<W> {
    pub fn new<I>(
        writer: W,
        fps: f32,
        include_color: bool,
        format: AnimationFormat,
        tokens: I,
    ) -> Result<Self, StreamError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut seen = HashSet::new();
        let mut dictionary = Vec::new();
        for token in std::iter::once(" ".to_string()).chain(tokens) {
            if seen.insert(token.clone()) {
                dictionary.push(token);
            }
        }
        let dictionary_index = dictionary
            .iter()
            .enumerate()
            .map(|(index, token)| (token.clone(), index as u64))
            .collect();

        Ok(Self {
            writer,
            fps,
            include_color,
            format,
            dictionary,
            dictionary_index,
            width: None,
            height: None,
            frame_count: 0,
            frame_count_offset: None,
            previous: None,
        })
    }

    pub fn push_frame(&mut self, canvas: TextCanvas) -> Result<(), StreamError> {
        if self.frame_count == u32::MAX {
            return Err(StreamError::TooManyFrames(u32::MAX));
        }

        match (self.width, self.height) {
            (None, None) => {
                self.width = Some(canvas.width);
                self.height = Some(canvas.height);
                self.write_header()?;
            }
            (Some(width), Some(height)) if canvas.width != width || canvas.height != height => {
                return Err(TglyphError::FrameSizeMismatch(
                    self.frame_count as usize,
                    width,
                    height,
                    canvas.width,
                    canvas.height,
                )
                .into());
            }
            _ => {}
        }

        match self.format {
            AnimationFormat::Binary => self.write_binary_frame(&canvas)?,
            AnimationFormat::Text => self.write_text_frame(&canvas)?,
        }
        self.previous = Some(canvas);
        self.frame_count += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, StreamError> {
        let frame_count_offset = self.frame_count_offset.ok_or(TglyphError::NoFrames)?;
        let end = self.writer.stream_position()?;
        self.writer.seek(SeekFrom::Start(frame_count_offset))?;
        match self.format {
            AnimationFormat::Binary => {
                self.writer.write_all(&self.frame_count.to_le_bytes())?;
            }
            AnimationFormat::Text => {
                write!(self.writer, "{:010}", self.frame_count)?;
            }
        }
        self.writer.seek(SeekFrom::Start(end))?;
        self.writer.flush()?;
        Ok(self.writer)
    }

    fn write_header(&mut self) -> Result<(), StreamError> {
        let width = self.width.expect("width set before header");
        let height = self.height.expect("height set before header");
        match self.format {
            AnimationFormat::Binary => {
                self.writer.write_all(BINARY_MAGIC)?;
                self.writer.write_all(&(width as u32).to_le_bytes())?;
                self.writer.write_all(&(height as u32).to_le_bytes())?;
                self.writer.write_all(&self.fps.to_le_bytes())?;
                self.writer
                    .write_all(&[if self.include_color { FLAG_COLOR } else { 0 }])?;
                self.frame_count_offset = Some(self.writer.stream_position()?);
                self.writer.write_all(&0_u32.to_le_bytes())?;
                write_varint(&mut self.writer, self.dictionary.len() as u64)?;
                for token in &self.dictionary {
                    write_varint(&mut self.writer, token.len() as u64)?;
                    self.writer.write_all(token.as_bytes())?;
                }
            }
            AnimationFormat::Text => {
                writeln!(self.writer, "TOPOGLYPH-ANIM v1")?;
                writeln!(self.writer, "WIDTH {width}")?;
                writeln!(self.writer, "HEIGHT {height}")?;
                writeln!(self.writer, "FPS {}", self.fps)?;
                writeln!(
                    self.writer,
                    "COLOR {}",
                    if self.include_color { "on" } else { "off" }
                )?;
                write!(self.writer, "FRAMES ")?;
                self.frame_count_offset = Some(self.writer.stream_position()?);
                writeln!(self.writer, "0000000000")?;
            }
        }
        Ok(())
    }

    fn write_binary_frame(&mut self, canvas: &TextCanvas) -> Result<(), StreamError> {
        if let Some(previous) = self.previous.as_ref() {
            let change_count = canvas
                .cells
                .iter()
                .zip(&previous.cells)
                .filter(|(cell, previous_cell)| {
                    cell.token != previous_cell.token
                        || (self.include_color && cell.color != previous_cell.color)
                })
                .count();
            write_varint(&mut self.writer, change_count as u64)?;

            let mut previous_index = -1_i64;
            for (index, (cell, previous_cell)) in
                canvas.cells.iter().zip(&previous.cells).enumerate()
            {
                if cell.token == previous_cell.token
                    && (!self.include_color || cell.color == previous_cell.color)
                {
                    continue;
                }
                write_varint(
                    &mut self.writer,
                    zigzag_encode(index as i64 - previous_index),
                )?;
                previous_index = index as i64;
                Self::write_binary_cell(
                    &mut self.writer,
                    &self.dictionary_index,
                    self.include_color,
                    cell,
                )?;
            }
        } else {
            for cell in &canvas.cells {
                Self::write_binary_cell(
                    &mut self.writer,
                    &self.dictionary_index,
                    self.include_color,
                    cell,
                )?;
            }
        }
        Ok(())
    }

    fn write_binary_cell(
        writer: &mut W,
        dictionary_index: &HashMap<String, u64>,
        include_color: bool,
        cell: &TextCell,
    ) -> Result<(), StreamError> {
        let dictionary_index = dictionary_index
            .get(cell.token.as_str())
            .copied()
            .ok_or_else(|| StreamError::UnknownToken(cell.token.clone()))?;
        write_varint(writer, dictionary_index)?;
        if include_color {
            match cell.color.as_deref().and_then(parse_hex_color) {
                Some(rgb) => {
                    writer.write_all(&[1])?;
                    writer.write_all(&rgb)?;
                }
                None => writer.write_all(&[0])?,
            }
        }
        Ok(())
    }

    fn write_text_frame(&mut self, canvas: &TextCanvas) -> Result<(), StreamError> {
        let width = self.width.expect("width set before frame");
        let height = self.height.expect("height set before frame");
        if let Some(previous) = self.previous.as_ref() {
            writeln!(self.writer, "---D{}---", self.frame_count)?;
            for (index, (cell, previous_cell)) in
                canvas.cells.iter().zip(&previous.cells).enumerate()
            {
                if cell.token == previous_cell.token
                    && (!self.include_color || cell.color == previous_cell.color)
                {
                    continue;
                }
                let row = index / width;
                let column = index % width;
                let token = if cell.token == " " {
                    ""
                } else {
                    cell.token.as_str()
                };
                if self.include_color {
                    writeln!(
                        self.writer,
                        "{row},{column},{token},{}",
                        cell.color.as_deref().unwrap_or("")
                    )?;
                } else {
                    writeln!(self.writer, "{row},{column},{token}")?;
                }
            }
        } else {
            writeln!(self.writer, "---F0---")?;
            for row in 0..height {
                for column in 0..width {
                    self.writer
                        .write_all(canvas.cells[row * width + column].token.as_bytes())?;
                }
                self.writer.write_all(b"\n")?;
            }
            if self.include_color {
                writeln!(self.writer, "---C0---")?;
                for (index, cell) in canvas.cells.iter().enumerate() {
                    if let Some(color) = &cell.color {
                        let row = index / width;
                        let column = index % width;
                        writeln!(self.writer, "{row},{column},{color}")?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn write_varint(writer: &mut impl Write, mut value: u64) -> io::Result<()> {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            writer.write_all(&[byte])?;
            return Ok(());
        }
        writer.write_all(&[byte | 0x80])?;
    }
}

fn zigzag_encode(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
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
