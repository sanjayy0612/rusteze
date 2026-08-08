#[cfg(any(not(target_os = "macos"), test))]
use crate::storage;
use std::path::{Path, PathBuf};
#[cfg(any(not(target_os = "macos"), test))]
use std::{
    fs::{self, File},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    time::Instant,
};

#[cfg(any(not(target_os = "macos"), test))]
const WAVE_FORMAT_PCM: u16 = 1;
#[cfg(any(not(target_os = "macos"), test))]
const MAX_WAV_DATA_BYTES: u32 = u32::MAX - 36;

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) struct WavWriter {
    file: File,
    data_bytes: u32,
    sample_rate: u32,
    channels: u16,
}

#[cfg(any(not(target_os = "macos"), test))]
impl WavWriter {
    pub(crate) fn create(path: &Path, sample_rate: u32, channels: u16) -> io::Result<Self> {
        if sample_rate == 0 || channels == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WAV sample rate and channel count must be nonzero.",
            ));
        }
        let mut file = storage::create_private_file_new(path)?;
        file.write_all(&[0; 44])?;
        Ok(Self {
            file,
            data_bytes: 0,
            sample_rate,
            channels,
        })
    }

    pub(crate) fn write_samples(&mut self, samples: &[i16]) -> io::Result<()> {
        let byte_count = samples.len().checked_mul(2).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "WAV sample buffer is too large",
            )
        })?;
        let new_data_bytes = u64::from(self.data_bytes)
            .checked_add(byte_count as u64)
            .filter(|total| *total <= u64::from(MAX_WAV_DATA_BYTES))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "WAV reached the 4 GiB RIFF limit; start a new recording segment",
                )
            })? as u32;

        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.file.write_all(&bytes)?;
        self.data_bytes = new_data_bytes;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> io::Result<()> {
        let byte_rate = self
            .sample_rate
            .checked_mul(u32::from(self.channels))
            .and_then(|rate| rate.checked_mul(2))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid WAV byte rate"))?;
        let block_align = self.channels.checked_mul(2).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid WAV block alignment")
        })?;
        let riff_size = 36u32
            .checked_add(self.data_bytes)
            .ok_or_else(|| io::Error::other("WAV file is too large"))?;

        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&riff_size.to_le_bytes())?;
        self.file.write_all(b"WAVEfmt ")?;
        self.file.write_all(&16u32.to_le_bytes())?;
        self.file.write_all(&WAVE_FORMAT_PCM.to_le_bytes())?;
        self.file.write_all(&self.channels.to_le_bytes())?;
        self.file.write_all(&self.sample_rate.to_le_bytes())?;
        self.file.write_all(&byte_rate.to_le_bytes())?;
        self.file.write_all(&block_align.to_le_bytes())?;
        self.file.write_all(&16u16.to_le_bytes())?;
        self.file.write_all(b"data")?;
        self.file.write_all(&self.data_bytes.to_le_bytes())?;
        self.file.flush()?;
        self.file.sync_all()
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn f32le_to_pcm16(bytes: &[u8]) -> Result<Vec<i16>, &'static str> {
    let chunks = bytes.chunks_exact(4);
    if !chunks.remainder().is_empty() {
        return Err("PipeWire returned an incomplete F32LE sample");
    }

    Ok(chunks
        .map(|chunk| {
            let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
        })
        .collect())
}

#[cfg(target_os = "macos")]
pub(crate) fn mix_tracks(session_folder: &Path) -> Result<PathBuf, String> {
    crate::native_helper::mix_audio(session_folder).map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn mix_tracks(session_folder: &Path) -> Result<PathBuf, String> {
    mix_wav_tracks(session_folder).map_err(|error| error.to_string())
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy)]
struct WavSpec {
    sample_rate: u32,
    channels: u16,
    frames: u64,
}

#[cfg(any(not(target_os = "macos"), test))]
struct WavSource {
    reader: BufReader<File>,
    spec: WavSpec,
    frames_read: u64,
    current_index: u64,
    current: Option<Vec<f32>>,
    next: Option<Vec<f32>>,
}

#[cfg(any(not(target_os = "macos"), test))]
impl WavSource {
    fn open(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || storage::is_link_or_reparse_point(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Refusing to mix non-regular audio file {}", path.display()),
            ));
        }

        let (mut reader, spec) = read_wav_header(path)?;
        let mut frames_read = 0;
        let current = read_pcm_frame(&mut reader, spec.channels, spec.frames, &mut frames_read)?;
        let mut source = Self {
            reader,
            spec,
            frames_read,
            current_index: 0,
            current,
            next: None,
        };
        source.next = source.read_next()?;
        Ok(source)
    }

    fn read_next(&mut self) -> io::Result<Option<Vec<f32>>> {
        read_pcm_frame(
            &mut self.reader,
            self.spec.channels,
            self.spec.frames,
            &mut self.frames_read,
        )
    }

    fn frame_at(
        &mut self,
        output_index: u64,
        output_rate: u32,
        output_channels: u16,
    ) -> io::Result<Option<Vec<f32>>> {
        let scaled = u128::from(output_index) * u128::from(self.spec.sample_rate);
        let wanted_index = (scaled / u128::from(output_rate)) as u64;
        if wanted_index >= self.spec.frames || self.current.is_none() {
            return Ok(None);
        }

        while self.current_index < wanted_index {
            self.current = self.next.take();
            self.current_index += 1;
            self.next = self.read_next()?;
        }

        let fraction = (scaled % u128::from(output_rate)) as f32 / output_rate as f32;
        let current = self.current.as_ref().expect("current frame checked above");
        let next = self.next.as_ref().unwrap_or(current);
        let mut frame = Vec::with_capacity(usize::from(output_channels));
        for channel in 0..output_channels {
            let a = channel_sample(current, self.spec.channels, channel, output_channels);
            let b = channel_sample(next, self.spec.channels, channel, output_channels);
            frame.push(a + (b - a) * fraction);
        }
        Ok(Some(frame))
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn channel_sample(frame: &[f32], source_channels: u16, channel: u16, output_channels: u16) -> f32 {
    if output_channels == 1 && source_channels > 1 {
        return frame.iter().sum::<f32>() / f32::from(source_channels);
    }
    if source_channels == 1 {
        frame[0]
    } else {
        frame[usize::from(channel.min(source_channels - 1))]
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn read_pcm_frame(
    reader: &mut BufReader<File>,
    channels: u16,
    total_frames: u64,
    frames_read: &mut u64,
) -> io::Result<Option<Vec<f32>>> {
    if *frames_read >= total_frames {
        return Ok(None);
    }
    let mut frame = Vec::with_capacity(usize::from(channels));
    for _ in 0..channels {
        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes)?;
        frame.push(f32::from(i16::from_le_bytes(bytes)) / 32768.0);
    }
    *frames_read += 1;
    Ok(Some(frame))
}

#[cfg(any(not(target_os = "macos"), test))]
fn read_wav_header(path: &Path) -> io::Result<(BufReader<File>, WavSpec)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut header = [0u8; 12];
    reader.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Input is not a RIFF/WAVE file",
        ));
    }

    let mut format = None;
    loop {
        let mut chunk_header = [0u8; 8];
        reader.read_exact(&mut chunk_header)?;
        let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into().unwrap());
        match &chunk_header[0..4] {
            b"fmt " => {
                if chunk_size < 16 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WAV fmt chunk is incomplete",
                    ));
                }
                let mut bytes = [0u8; 16];
                reader.read_exact(&mut bytes)?;
                let encoding = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
                let sample_rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                let block_align = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
                let bits_per_sample = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
                let expected_block_align = channels.checked_mul(2);
                if encoding != WAVE_FORMAT_PCM
                    || channels == 0
                    || sample_rate == 0
                    || bits_per_sample != 16
                    || expected_block_align != Some(block_align)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Only interleaved PCM16 WAV tracks are supported",
                    ));
                }
                format = Some((sample_rate, channels, block_align));
                skip_chunk_tail(&mut reader, chunk_size - 16)?;
            }
            b"data" => {
                let Some((sample_rate, channels, block_align)) = format else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WAV data appeared before its format",
                    ));
                };
                if chunk_size % u32::from(block_align) != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WAV data does not contain complete frames",
                    ));
                }
                return Ok((
                    reader,
                    WavSpec {
                        sample_rate,
                        channels,
                        frames: u64::from(chunk_size / u32::from(block_align)),
                    },
                ));
            }
            _ => skip_chunk_tail(&mut reader, chunk_size)?,
        }
        if chunk_size % 2 == 1 {
            reader.seek(SeekFrom::Current(1))?;
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn skip_chunk_tail(reader: &mut BufReader<File>, byte_count: u32) -> io::Result<()> {
    reader.seek(SeekFrom::Current(i64::from(byte_count)))?;
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn mix_wav_tracks(session_folder: &Path) -> io::Result<PathBuf> {
    crate::meeting::ensure_path_has_recording_space(session_folder)?;
    let mut system = WavSource::open(&session_folder.join("system.wav"))?;
    let mut microphone = WavSource::open(&session_folder.join("mic.wav"))?;
    let output_rate = system.spec.sample_rate.max(microphone.spec.sample_rate);
    let output_channels = system.spec.channels.max(microphone.spec.channels);
    let output_frames = [system.spec, microphone.spec]
        .into_iter()
        .map(|spec| {
            (u128::from(spec.frames) * u128::from(output_rate))
                .div_ceil(u128::from(spec.sample_rate)) as u64
        })
        .max()
        .unwrap_or(0);
    let destination = session_folder.join("mixed.wav");
    let temporary = (0..128u32)
        .map(|counter| {
            session_folder.join(format!(".mixed.{}.{}.tmp.wav", std::process::id(), counter))
        })
        .find(|path| !path.exists())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Could not allocate a mixed-audio temporary file",
            )
        })?;

    let result = (|| {
        let mut writer = WavWriter::create(&temporary, output_rate, output_channels)?;
        let mut samples = Vec::with_capacity(usize::from(output_channels) * 4096);
        let mut last_space_check = Instant::now();
        for output_index in 0..output_frames {
            let system_frame = system.frame_at(output_index, output_rate, output_channels)?;
            let microphone_frame =
                microphone.frame_at(output_index, output_rate, output_channels)?;
            for channel in 0..usize::from(output_channels) {
                let mixed = (system_frame.as_ref().map_or(0.0, |frame| frame[channel])
                    + microphone_frame
                        .as_ref()
                        .map_or(0.0, |frame| frame[channel]))
                    * 0.5;
                samples.push((mixed.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16);
            }
            if samples.len() >= 4096 * usize::from(output_channels) {
                if last_space_check.elapsed() >= crate::meeting::RECORDING_SPACE_CHECK_INTERVAL {
                    crate::meeting::ensure_path_has_recording_space(session_folder)?;
                    last_space_check = Instant::now();
                }
                writer.write_samples(&samples)?;
                samples.clear();
            }
        }
        writer.write_samples(&samples)?;
        writer.finish()?;
        storage::replace_file_atomically(&temporary, &destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| destination)
}

#[cfg(test)]
mod tests {
    use super::{f32le_to_pcm16, mix_wav_tracks, read_pcm_frame, read_wav_header, WavWriter};
    use std::{fs, time::SystemTime};

    fn temporary_wav_path() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rusteze-wav-test-{}-{nonce}.wav",
            std::process::id()
        ))
    }

    #[test]
    fn converts_float_samples_to_pcm16() {
        let samples = f32le_to_pcm16(&0.5f32.to_le_bytes()).unwrap();
        assert_eq!(samples, vec![16_383]);
    }

    #[test]
    fn rejects_incomplete_float_samples() {
        assert!(f32le_to_pcm16(&[0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_a_riff_overflow_before_writing_more_audio() {
        let path = temporary_wav_path();
        let mut writer = WavWriter::create(&path, 48_000, 2).unwrap();
        writer.data_bytes = u32::MAX - 36;
        let length_before_write = fs::metadata(&path).unwrap().len();

        let result = writer.write_samples(&[0]);

        assert!(result.is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), length_before_write);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn finalizes_a_valid_pcm_wav_header() {
        let path = temporary_wav_path();
        let mut writer = WavWriter::create(&path, 48_000, 2).unwrap();
        writer.write_samples(&[0, 1, -1, 0]).unwrap();
        writer.finish().unwrap();

        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn mixes_tracks_without_changing_the_sources() {
        let directory = temporary_wav_path().with_extension("session");
        fs::create_dir(&directory).unwrap();
        let system_path = directory.join("system.wav");
        let microphone_path = directory.join("mic.wav");

        let mut system = WavWriter::create(&system_path, 24_000, 1).unwrap();
        system.write_samples(&[10_000, 10_000]).unwrap();
        system.finish().unwrap();
        let mut microphone = WavWriter::create(&microphone_path, 48_000, 2).unwrap();
        microphone
            .write_samples(&[2_000, 2_000, 2_000, 2_000, 2_000, 2_000, 2_000, 2_000])
            .unwrap();
        microphone.finish().unwrap();
        let original_system = fs::read(&system_path).unwrap();
        let original_microphone = fs::read(&microphone_path).unwrap();

        let output = mix_wav_tracks(&directory).unwrap();

        assert_eq!(fs::read(&system_path).unwrap(), original_system);
        assert_eq!(fs::read(&microphone_path).unwrap(), original_microphone);
        let (mut reader, spec) = read_wav_header(&output).unwrap();
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.frames, 4);
        let mut frames_read = 0;
        let first = read_pcm_frame(&mut reader, spec.channels, spec.frames, &mut frames_read)
            .unwrap()
            .unwrap();
        assert!((first[0] * 32768.0 - 6_000.0).abs() <= 1.0);
        assert!((first[1] * 32768.0 - 6_000.0).abs() <= 1.0);
        fs::remove_dir_all(directory).unwrap();
    }
}
