//! Writes a video's audio track beside its `.tglyph` animation.
//!
//! The primary sidecar is M4A/AAC: existing AAC packets are remuxed without
//! decoding, while other input codecs are transcoded to 128 kbit/s AAC.
//! This keeps audio close to its source size instead of expanding it to
//! uncompressed PCM WAV.

use std::path::{Path, PathBuf};

use ffmpeg_next::{codec, encoder, filter, format, frame, media};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioExtractMode {
    NoAudio,
    Remuxed,
    Transcoded,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioExtractError {
    #[error("ffmpeg audio processing failed: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),
    #[error("ffmpeg failed while {stage}: {source}")]
    Stage {
        stage: &'static str,
        #[source]
        source: ffmpeg_next::Error,
    },
    #[error("AAC encoder is unavailable in this FFmpeg build")]
    EncoderUnavailable,
    #[error("required FFmpeg audio filter {0:?} is unavailable")]
    FilterUnavailable(&'static str),
}

/// Returns the primary compact audio sidecar path:
/// `foo.tglyph` -> `foo.tglyph.m4a`.
pub fn sidecar_audio_path(tglyph_path: &Path) -> PathBuf {
    let mut path = tglyph_path.as_os_str().to_owned();
    path.push(".m4a");
    PathBuf::from(path)
}

/// Returns the legacy PCM sidecar path used by releases before M4A support.
/// New conversions never write this path; playback checks it only so existing
/// `.tglyph.wav` pairs continue to work.
pub fn sidecar_wav_path(tglyph_path: &Path) -> PathBuf {
    let mut path = tglyph_path.as_os_str().to_owned();
    path.push(".wav");
    PathBuf::from(path)
}

/// Extracts the best audio stream to an M4A sidecar.
///
/// AAC input is packet-remuxed without quality loss or re-encoding. Other
/// codecs are converted to 128 kbit/s AAC. Inputs without an audio stream
/// return [`AudioExtractMode::NoAudio`] and do not create an output file.
pub fn extract_audio_to_m4a(
    input: &Path,
    output: &Path,
) -> Result<AudioExtractMode, AudioExtractError> {
    ffmpeg_next::init()?;
    let mut input_context = format::input(input)?;
    let audio_stream = match input_context.streams().best(media::Type::Audio) {
        Some(stream) => stream,
        None => return Ok(AudioExtractMode::NoAudio),
    };

    if audio_stream.parameters().id() == codec::Id::AAC {
        remux_aac(&mut input_context, output)?;
        Ok(AudioExtractMode::Remuxed)
    } else {
        transcode_to_aac(&mut input_context, output)?;
        Ok(AudioExtractMode::Transcoded)
    }
}

fn remux_aac(
    input_context: &mut format::context::Input,
    output: &Path,
) -> Result<(), AudioExtractError> {
    let input_stream = input_context
        .streams()
        .best(media::Type::Audio)
        .expect("audio stream checked by caller");
    let input_stream_index = input_stream.index();
    let input_time_base = input_stream.time_base();
    let input_parameters = input_stream.parameters();

    let mut output_context = format::output(output)?;
    {
        let mut output_stream = output_context.add_stream(encoder::find(codec::Id::None))?;
        output_stream.set_parameters(input_parameters);
        output_stream.set_time_base(input_time_base);
        // Container-specific codec tags from the source container may be
        // invalid in M4A. FFmpeg chooses the correct tag when this is zero.
        unsafe {
            (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
        }
    }
    output_context.set_metadata(input_context.metadata().to_owned());
    output_context.write_header()?;
    let output_time_base = output_context
        .stream(0)
        .expect("output stream just created")
        .time_base();

    for (stream, mut packet) in input_context.packets() {
        if stream.index() != input_stream_index {
            continue;
        }
        packet.rescale_ts(stream.time_base(), output_time_base);
        packet.set_position(-1);
        packet.set_stream(0);
        packet.write_interleaved(&mut output_context)?;
    }
    output_context.write_trailer()?;
    Ok(())
}

fn transcode_to_aac(
    input_context: &mut format::context::Input,
    output: &Path,
) -> Result<(), AudioExtractError> {
    let input_stream = input_context
        .streams()
        .best(media::Type::Audio)
        .expect("audio stream checked by caller");
    let input_stream_index = input_stream.index();
    let input_time_base = input_stream.time_base();
    let decoder_context = codec::context::Context::from_parameters(input_stream.parameters())
        .map_err(|source| AudioExtractError::Stage {
            stage: "creating the audio decoder context",
            source,
        })?;
    let decoder = decoder_context
        .decoder()
        .audio()
        .map_err(|source| AudioExtractError::Stage {
            stage: "opening the audio decoder",
            source,
        })?;

    let aac = encoder::find(codec::Id::AAC)
        .ok_or(AudioExtractError::EncoderUnavailable)?
        .audio()?;
    let mut output_context = format::output(output).map_err(|source| AudioExtractError::Stage {
        stage: "opening the M4A output",
        source,
    })?;
    let global_header = output_context
        .format()
        .flags()
        .contains(format::flag::Flags::GLOBAL_HEADER);
    let (audio_encoder, output_time_base) = {
        let mut output_stream =
            output_context
                .add_stream(aac)
                .map_err(|source| AudioExtractError::Stage {
                    stage: "adding the AAC output stream",
                    source,
                })?;
        let encoder_context = codec::context::Context::from_parameters(output_stream.parameters())
            .map_err(|source| AudioExtractError::Stage {
                stage: "creating the AAC encoder context",
                source,
            })?;
        let mut audio_encoder =
            encoder_context
                .encoder()
                .audio()
                .map_err(|source| AudioExtractError::Stage {
                    stage: "configuring the AAC encoder",
                    source,
                })?;

        let channel_layout = aac
            .channel_layouts()
            .map(|layouts| layouts.best(i32::from(decoder.channels())))
            .unwrap_or_else(|| {
                ffmpeg_next::channel_layout::ChannelLayout::default(i32::from(decoder.channels()))
            });
        let sample_rate = aac
            .rates()
            .and_then(|rates| {
                rates.min_by_key(|rate| (*rate as i64 - decoder.rate() as i64).unsigned_abs())
            })
            .unwrap_or(decoder.rate() as i32);
        let sample_format = aac
            .formats()
            .and_then(|mut formats| formats.next())
            .ok_or(AudioExtractError::EncoderUnavailable)?;

        if global_header {
            audio_encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        audio_encoder.set_rate(sample_rate);
        audio_encoder.set_channel_layout(channel_layout);
        audio_encoder.set_format(sample_format);
        audio_encoder.set_bit_rate(128_000);
        audio_encoder.set_time_base((1, sample_rate));
        output_stream.set_time_base((1, sample_rate));

        let audio_encoder =
            audio_encoder
                .open_as(aac)
                .map_err(|source| AudioExtractError::Stage {
                    stage: "opening the AAC encoder",
                    source,
                })?;
        output_stream.set_parameters(&audio_encoder);
        let output_time_base = output_stream.time_base();
        (audio_encoder, output_time_base)
    };

    let graph = build_audio_filter(&decoder, &audio_encoder)?;
    output_context.set_metadata(input_context.metadata().to_owned());
    output_context
        .write_header()
        .map_err(|source| AudioExtractError::Stage {
            stage: "writing the M4A header",
            source,
        })?;

    let mut transcoder = AacTranscoder {
        decoder,
        encoder: audio_encoder,
        filter: graph,
        input_time_base,
        output_time_base,
    };

    for (stream, mut packet) in input_context.packets() {
        if stream.index() != input_stream_index {
            continue;
        }
        packet.rescale_ts(stream.time_base(), transcoder.input_time_base);
        transcoder.decoder.send_packet(&packet)?;
        transcoder.drain_decoder(&mut output_context)?;
    }
    transcoder.decoder.send_eof()?;
    transcoder.drain_decoder(&mut output_context)?;
    transcoder
        .filter
        .get("in")
        .expect("filter source exists")
        .source()
        .flush()?;
    transcoder.drain_filter(&mut output_context)?;
    transcoder.encoder.send_eof()?;
    transcoder.drain_encoder(&mut output_context)?;
    output_context.write_trailer()?;
    Ok(())
}

fn build_audio_filter(
    decoder: &codec::decoder::Audio,
    encoder: &codec::encoder::Audio,
) -> Result<filter::Graph, AudioExtractError> {
    let decoder_layout = if decoder.channel_layout().bits() == 0 {
        ffmpeg_next::channel_layout::ChannelLayout::default(i32::from(decoder.channels()))
    } else {
        decoder.channel_layout()
    };
    let mut graph = filter::Graph::new();
    let arguments = format!(
        "time_base={}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
        decoder.time_base(),
        decoder.rate(),
        decoder.format().name(),
        decoder_layout.bits()
    );
    let source = filter::find("abuffer").ok_or(AudioExtractError::FilterUnavailable("abuffer"))?;
    let sink =
        filter::find("abuffersink").ok_or(AudioExtractError::FilterUnavailable("abuffersink"))?;
    graph.add(&source, "in", &arguments)?;
    graph.add(&sink, "out", "")?;
    let conversion = format!(
        "aformat=sample_fmts={}:sample_rates={}:channel_layouts=0x{:x}",
        encoder.format().name(),
        encoder.rate(),
        encoder.channel_layout().bits()
    );
    graph.output("in", 0)?.input("out", 0)?.parse(&conversion)?;
    graph.validate()?;
    if let Some(codec) = encoder.codec() {
        if !codec
            .capabilities()
            .contains(codec::capabilities::Capabilities::VARIABLE_FRAME_SIZE)
        {
            graph
                .get("out")
                .expect("filter sink exists")
                .sink()
                .set_frame_size(encoder.frame_size());
        }
    }
    Ok(graph)
}

struct AacTranscoder {
    decoder: codec::decoder::Audio,
    encoder: codec::encoder::Audio,
    filter: filter::Graph,
    input_time_base: ffmpeg_next::Rational,
    output_time_base: ffmpeg_next::Rational,
}

impl AacTranscoder {
    fn drain_decoder(
        &mut self,
        output: &mut format::context::Output,
    ) -> Result<(), AudioExtractError> {
        let mut decoded = frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            let timestamp = decoded.timestamp();
            decoded.set_pts(timestamp);
            if decoded.channel_layout().bits() == 0 {
                decoded.set_channel_layout(ffmpeg_next::channel_layout::ChannelLayout::default(
                    i32::from(decoded.channels()),
                ));
            }
            self.filter
                .get("in")
                .expect("filter source exists")
                .source()
                .add(&decoded)?;
            self.drain_filter(output)?;
        }
        Ok(())
    }

    fn drain_filter(
        &mut self,
        output: &mut format::context::Output,
    ) -> Result<(), AudioExtractError> {
        let mut filtered = frame::Audio::empty();
        while self
            .filter
            .get("out")
            .expect("filter sink exists")
            .sink()
            .frame(&mut filtered)
            .is_ok()
        {
            self.encoder.send_frame(&filtered)?;
            self.drain_encoder(output)?;
        }
        Ok(())
    }

    fn drain_encoder(
        &mut self,
        output: &mut format::context::Output,
    ) -> Result<(), AudioExtractError> {
        let mut encoded = ffmpeg_next::Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(0);
            encoded.rescale_ts(self.input_time_base, self.output_time_base);
            encoded.write_interleaved(output)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_path(extension: &str) -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "topoglyph-audio-{}-{id}.{extension}",
            std::process::id()
        ))
    }

    fn write_pcm_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for frame in 0..44_100 {
            let sample = ((frame as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin()
                * i16::MAX as f32
                * 0.25) as i16;
            writer.write_sample(sample).unwrap();
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn extraction_transcodes_pcm_to_compact_aac_m4a() {
        let input = temp_path("wav");
        let output = temp_path("m4a");
        write_pcm_wav(&input);

        let mode = extract_audio_to_m4a(&input, &output).unwrap();
        assert_eq!(mode, AudioExtractMode::Transcoded);
        assert!(
            std::fs::metadata(&output).unwrap().len() * 2
                < std::fs::metadata(&input).unwrap().len()
        );

        let context = ffmpeg_next::format::input(&output).unwrap();
        let stream = context
            .streams()
            .best(ffmpeg_next::media::Type::Audio)
            .unwrap();
        assert_eq!(stream.parameters().id(), ffmpeg_next::codec::Id::AAC);

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn extraction_remuxes_existing_aac_without_transcoding() {
        let input = temp_path("wav");
        let first_m4a = temp_path("m4a");
        let remuxed_m4a = temp_path("m4a");
        write_pcm_wav(&input);
        assert_eq!(
            extract_audio_to_m4a(&input, &first_m4a).unwrap(),
            AudioExtractMode::Transcoded
        );

        let mode = extract_audio_to_m4a(&first_m4a, &remuxed_m4a).unwrap();
        assert_eq!(mode, AudioExtractMode::Remuxed);
        let first_size = std::fs::metadata(&first_m4a).unwrap().len();
        let remuxed_size = std::fs::metadata(&remuxed_m4a).unwrap().len();
        assert!(first_size.abs_diff(remuxed_size) < first_size / 10);

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(first_m4a);
        let _ = std::fs::remove_file(remuxed_m4a);
    }

    #[test]
    fn sidecar_audio_path_appends_m4a_extension() {
        assert_eq!(
            sidecar_audio_path(Path::new("/tmp/animations/clip.tglyph")),
            PathBuf::from("/tmp/animations/clip.tglyph.m4a")
        );
    }

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
    fn extract_audio_reports_no_audio_for_missing_audio_stream() {
        // A 1x1 solid PNG has no audio stream at all (it's not even a
        // video container), exercising the same path a silent recording
        // or GIF would hit.
        let dir = std::env::temp_dir();
        let png_path = dir.join("topoglyph_audio_test_input.png");
        let output_path = dir.join("topoglyph_audio_test_output.m4a");
        // Minimal valid 1x1 PNG.
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x99, 0x9C, 0x28,
            0x18, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png_path, png_bytes).unwrap();

        let result = extract_audio_to_m4a(&png_path, &output_path);
        let _ = std::fs::remove_file(&png_path);
        let _ = std::fs::remove_file(&output_path);

        assert_eq!(result.unwrap(), AudioExtractMode::NoAudio);
    }
}
