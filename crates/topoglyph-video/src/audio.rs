//! Extracts a video's audio track as a `.wav` sidecar file next to a
//! `.tglyph` animation, so `topoglyph play` can play the original audio
//! back in sync with the text-art frames instead of producing a silent
//! text-only render (0.2.2: 视频转换的时候应该带有音频，播放的时候也带有
//! 音频播放).
//!
//! Deliberately a *sidecar* file rather than embedding audio bytes inside
//! `.tglyph` itself: `.tglyph` stays a small, purpose-built text-cell
//! format, and the sidecar is a plain standard-compliant WAV any player
//! (not just `topoglyph play`) can open.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AudioExtractError {
    #[error("ffmpeg input error: {0}")]
    Input(#[source] ffmpeg_next::Error),
    #[error("no audio stream found")]
    NoAudioStream,
    #[error("failed to open audio decoder: {0}")]
    Decoder(#[source] ffmpeg_next::Error),
    #[error("failed to build resampler: {0}")]
    Resampler(#[source] ffmpeg_next::Error),
    #[error("resampling failed: {0}")]
    Resample(#[source] ffmpeg_next::Error),
    #[error("failed to write WAV file: {0}")]
    Wav(#[source] hound::Error),
}

/// Returns the sidecar `.wav` path a `.tglyph` output path should use:
/// `foo.tglyph` -> `foo.tglyph.wav`. Keeping the full original extension in
/// the sidecar name (rather than replacing it) means `foo.tglyph` and its
/// audio sort/glob together and neither file's name is ambiguous on its
/// own.
pub fn sidecar_wav_path(tglyph_path: &Path) -> PathBuf {
    let mut path = tglyph_path.as_os_str().to_owned();
    path.push(".wav");
    PathBuf::from(path)
}

/// Decodes the best audio stream in `input` to a 16-bit PCM WAV file at
/// `wav_path`, resampling to the stream's native sample rate/channel count
/// (no re-sampling needed since we're not mixing with anything else) but
/// converting to a fixed `S16` sample format, since that's what every WAV
/// player and `rodio::Decoder` handles universally.
///
/// Returns `Ok(false)` (not an error) if `input` has no audio stream at
/// all, e.g. a silent screen recording or a GIF — callers should treat
/// that as "nothing to extract", not a failure.
pub fn extract_audio_to_wav(
    input: &Path,
    wav_path: &Path,
) -> Result<bool, AudioExtractError> {
    let mut ictx = ffmpeg_next::format::input(input).map_err(AudioExtractError::Input)?;

    let audio_stream_index = match ictx
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
    {
        Some(stream) => stream.index(),
        None => return Ok(false),
    };

    let stream = ictx.stream(audio_stream_index).unwrap();
    let context = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
        .map_err(AudioExtractError::Decoder)?;
    let mut decoder = context.decoder().audio().map_err(AudioExtractError::Decoder)?;

    let in_rate = decoder.rate();
    let in_layout = decoder.channel_layout();
    let channels = decoder.channels().max(1);

    let mut resampler = ffmpeg_next::software::resampler(
        (decoder.format(), in_layout, in_rate),
        (
            ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed),
            in_layout,
            in_rate,
        ),
    )
    .map_err(AudioExtractError::Resampler)?;

    let spec = hound::WavSpec {
        channels,
        sample_rate: in_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(wav_path, spec).map_err(AudioExtractError::Wav)?;

    let mut decoded = ffmpeg_next::frame::Audio::empty();
    let mut resampled = ffmpeg_next::frame::Audio::empty();

    let write_resampled = |resampled: &ffmpeg_next::frame::Audio,
                            writer: &mut hound::WavWriter<_>|
     -> Result<(), AudioExtractError> {
        // `plane::<T>()` slices by `samples()` element count, i.e.
        // *per-channel* frame count — for a packed/interleaved format with
        // >1 channel, the real interleaved buffer holds `samples() *
        // channels()` i16s, so reading only `plane(0)` silently truncates
        // (and effectively corrupts, since it's still reading from the
        // start of the same interleaved buffer) to roughly `1/channels`
        // of the actual audio. Read the raw byte plane instead and slice
        // it to the exact valid byte range ourselves.
        let channels = resampled.channels() as usize;
        let sample_count = resampled.samples() * channels;
        let bytes = resampled.data(0);
        let valid_bytes = sample_count * std::mem::size_of::<i16>();
        let bytes = &bytes[..valid_bytes.min(bytes.len())];
        for chunk in bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            writer
                .write_sample(sample)
                .map_err(AudioExtractError::Wav)?;
        }
        Ok(())
    };

    for (stream, packet) in ictx.packets() {
        if stream.index() != audio_stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(AudioExtractError::Decoder)?;
        while decoder.receive_frame(&mut decoded).is_ok() {
            resampler
                .run(&decoded, &mut resampled)
                .map_err(AudioExtractError::Resample)?;
            write_resampled(&resampled, &mut writer)?;
        }
    }
    decoder.send_eof().map_err(AudioExtractError::Decoder)?;
    while decoder.receive_frame(&mut decoded).is_ok() {
        resampler
            .run(&decoded, &mut resampled)
            .map_err(AudioExtractError::Resample)?;
        write_resampled(&resampled, &mut writer)?;
    }
    // Drain any samples the resampler buffered internally (it can hold a
    // few samples back for correct filtering at the boundary).
    loop {
        match resampler.flush(&mut resampled) {
            Ok(Some(_)) => write_resampled(&resampled, &mut writer)?,
            Ok(None) => break,
            Err(_) => break,
        }
    }

    writer.finalize().map_err(AudioExtractError::Wav)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_wav_path_appends_wav_extension() {
        assert_eq!(
            sidecar_wav_path(Path::new("out.tglyph")),
            PathBuf::from("out.tglyph.wav")
        );
    }

    #[test]
    fn sidecar_wav_path_preserves_directory() {
        assert_eq!(
            sidecar_wav_path(Path::new("/tmp/animations/clip.tglyph")),
            PathBuf::from("/tmp/animations/clip.tglyph.wav")
        );
    }

    #[test]
    fn extract_audio_returns_false_for_missing_audio_stream() {
        // A 1x1 solid PNG has no audio stream at all (it's not even a
        // video container), exercising the same "no audio -> Ok(false)"
        // path a silent screen recording or GIF would hit.
        let dir = std::env::temp_dir();
        let png_path = dir.join("topoglyph_audio_test_input.png");
        let wav_path = dir.join("topoglyph_audio_test_output.wav");
        // Minimal valid 1x1 PNG.
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x99, 0x9C, 0x28,
            0x18, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png_path, png_bytes).unwrap();

        let result = extract_audio_to_wav(&png_path, &wav_path);
        let _ = std::fs::remove_file(&png_path);
        let _ = std::fs::remove_file(&wav_path);

        assert_eq!(result.unwrap(), false);
    }
}
