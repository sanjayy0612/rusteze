use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

const WAVE_FORMAT_PCM: u16 = 1;

pub(crate) struct WavWriter {
    file: File,
    data_bytes: u32,
    sample_rate: u32,
    channels: u16,
}

impl WavWriter {
    pub(crate) fn create(path: &Path, sample_rate: u32, channels: u16) -> io::Result<Self> {
        let mut file = File::create(path)?;
        file.write_all(&[0; 44])?;
        Ok(Self {
            file,
            data_bytes: 0,
            sample_rate,
            channels,
        })
    }

    pub(crate) fn write_samples(&mut self, samples: &[i16]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.file.write_all(&bytes)?;
        self.data_bytes = self
            .data_bytes
            .checked_add(bytes.len() as u32)
            .ok_or_else(|| io::Error::other("WAV file is too large"))?;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> io::Result<()> {
        let byte_rate = self.sample_rate * u32::from(self.channels) * 2;
        let block_align = self.channels * 2;
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
        self.file.flush()
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
    use super::f32le_to_pcm16;

    #[test]
    fn converts_float_samples_to_pcm16() {
        let samples = f32le_to_pcm16(&0.5f32.to_le_bytes()).unwrap();
        assert_eq!(samples, vec![16_383]);
    }

    #[test]
    fn rejects_incomplete_float_samples() {
        assert!(f32le_to_pcm16(&[0, 0, 0]).is_err());
    }
}
