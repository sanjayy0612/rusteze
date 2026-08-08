use crate::storage;
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

const WAVE_FORMAT_PCM: u16 = 1;
const MAX_WAV_DATA_BYTES: u32 = u32::MAX - 36;

pub(crate) struct WavWriter {
    file: File,
    data_bytes: u32,
    sample_rate: u32,
    channels: u16,
}

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

#[cfg(test)]
mod tests {
    use super::{f32le_to_pcm16, WavWriter};
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
}
