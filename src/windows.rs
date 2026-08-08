use super::{CaptureMode, HelperError, PermissionStatus};
use crate::audio::WavWriter;
use ::windows::Win32::{
    Media::Audio::{
        eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
    },
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    },
};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const PCM_SUBFORMAT: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];
const FLOAT_SUBFORMAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

#[derive(Clone, Copy, Debug)]
enum SampleKind {
    Pcm,
    Float,
}

#[derive(Clone, Copy, Debug)]
struct CaptureFormat {
    sample_rate: u32,
    channels: u16,
    block_align: usize,
    bytes_per_sample: usize,
    bits_per_sample: u16,
    kind: SampleKind,
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.is_err() {
            return Err(format!("COM initialization failed: {result:?}"));
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct AllocatedFormat(*mut WAVEFORMATEX);

impl Drop for AllocatedFormat {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
}

pub struct CaptureProcess {
    stop: Arc<AtomicBool>,
    workers: Vec<Worker>,
}

struct Worker {
    handle: JoinHandle<Result<(), String>>,
    errors: Receiver<String>,
}

impl CaptureProcess {
    pub fn check_health(&mut self) -> Result<(), HelperError> {
        for worker in &mut self.workers {
            if let Ok(error) = worker.errors.try_recv() {
                return Err(HelperError::Backend(error));
            }
            if worker.handle.is_finished() && !self.stop.load(Ordering::Acquire) {
                return Err(HelperError::Backend(
                    "Windows audio capture worker exited unexpectedly.".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub fn stop(self) -> Result<(), HelperError> {
        self.stop.store(true, Ordering::Release);
        let mut first_error = None;
        for worker in self.workers {
            match worker.handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
                Ok(Err(_)) => {}
                Err(_) if first_error.is_none() => {
                    first_error = Some("Windows audio capture worker panicked.".to_string())
                }
                Err(_) => {}
            }
        }

        first_error
            .map(|error| Err(HelperError::Backend(error)))
            .unwrap_or(Ok(()))
    }

    fn start_source(
        &mut self,
        path: &Path,
        flow: ::windows::Win32::Media::Audio::EDataFlow,
        loopback: bool,
    ) -> Result<(), HelperError> {
        let path = path.to_owned();
        let stop = Arc::clone(&self.stop);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (error_sender, error_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = run_capture(&path, flow, loopback, stop, &ready_sender);
            if let Err(error) = &result {
                let _ = error_sender.send(error.clone());
            }
            result
        });

        match ready_receiver.recv() {
            Ok(Ok(())) => {
                self.workers.push(Worker {
                    handle,
                    errors: error_receiver,
                });
                Ok(())
            }
            Ok(Err(error)) => {
                self.stop.store(true, Ordering::Release);
                let _ = handle.join();
                Err(HelperError::Backend(error))
            }
            Err(_) => {
                self.stop.store(true, Ordering::Release);
                let _ = handle.join();
                Err(HelperError::Backend(
                    "Windows audio capture worker failed during startup.".to_string(),
                ))
            }
        }
    }
}

pub fn check_permissions(mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    if mode.requires_screen_recording() {
        probe_endpoint(eRender, true).map_err(HelperError::Backend)?;
    }
    if mode.requires_microphone() {
        probe_endpoint(eCapture, false).map_err(HelperError::Backend)?;
    }

    Ok(PermissionStatus {
        microphone: if mode.requires_microphone() {
            "granted".to_string()
        } else {
            "not_required".to_string()
        },
        screen_recording: if mode.requires_screen_recording() {
            "granted".to_string()
        } else {
            "not_required".to_string()
        },
    })
}

pub fn request_permissions(mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    // Windows exposes device/privacy failures when the WASAPI client is initialized;
    // there is no equivalent prompt API for this backend.
    check_permissions(mode)
}

pub fn start_capture(
    session_folder: &Path,
    mode: CaptureMode,
) -> Result<CaptureProcess, HelperError> {
    let mut process = CaptureProcess {
        stop: Arc::new(AtomicBool::new(false)),
        workers: Vec::new(),
    };

    let start_result = (|| {
        if mode.requires_screen_recording() {
            process.start_source(&session_folder.join("system.wav"), eRender, true)?;
        }
        if mode.requires_microphone() {
            process.start_source(&session_folder.join("mic.wav"), eCapture, false)?;
        }
        Ok::<(), HelperError>(())
    })();

    if let Err(error) = start_result {
        let _ = process.stop();
        return Err(error);
    }

    Ok(process)
}

fn probe_endpoint(
    flow: ::windows::Win32::Media::Audio::EDataFlow,
    loopback: bool,
) -> Result<(), String> {
    let _com = ComApartment::initialize()?;
    let (device, client, format) = initialize_client(flow, loopback)?;
    drop(format);
    drop(client);
    drop(device);
    Ok(())
}

fn initialize_client(
    flow: ::windows::Win32::Media::Audio::EDataFlow,
    loopback: bool,
) -> Result<
    (
        ::windows::Win32::Media::Audio::IMMDevice,
        IAudioClient,
        AllocatedFormat,
    ),
    String,
> {
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }
    .map_err(|error| format!("Could not create the Windows audio device enumerator: {error}"))?;
    let device =
        unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }.map_err(|error| {
            if loopback {
                format!("No default Windows audio output device found: {error}")
            } else {
                format!("No default Windows microphone found: {error}")
            }
        })?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| format!("Could not activate the Windows audio endpoint: {error}"))?;
    let format = unsafe { client.GetMixFormat() }
        .map_err(|error| format!("Could not read the Windows audio device format: {error}"))?;
    let format = AllocatedFormat(format);
    let flags = if loopback {
        AUDCLNT_STREAMFLAGS_LOOPBACK
    } else {
        0
    };
    unsafe {
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, format.0, None)
            .map_err(|error| {
                if loopback {
                    format!("Failed to initialize WASAPI loopback capture: {error}")
                } else {
                    format!("Microphone access is unavailable: {error}")
                }
            })?;
    }
    Ok((device, client, format))
}

fn run_capture(
    path: &Path,
    flow: ::windows::Win32::Media::Audio::EDataFlow,
    loopback: bool,
    stop: Arc<AtomicBool>,
    ready_sender: &SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let _com = ComApartment::initialize()?;
    let (_device, client, allocated_format) = initialize_client(flow, loopback)?;
    let format = unsafe { read_capture_format(allocated_format.0)? };
    let capture: IAudioCaptureClient = unsafe { client.GetService() }
        .map_err(|error| format!("Failed to initialize WASAPI capture: {error}"))?;
    unsafe { client.Start() }
        .map_err(|error| format!("Failed to start Windows audio capture: {error}"))?;
    let mut writer = match WavWriter::create(path, format.sample_rate, format.channels) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = unsafe { client.Stop() };
            return Err(format!("Could not create {}: {error}", path.display()));
        }
    };
    if ready_sender.send(Ok(())).is_err() {
        let _ = unsafe { client.Stop() };
        let _ = writer.finish();
        return Err("Rusteze stopped waiting for Windows audio capture startup.".to_string());
    }

    let capture_result = capture_packets(&capture, &mut writer, format, &stop);
    let stop_result = unsafe { client.Stop() }
        .map_err(|error| format!("Failed to stop Windows audio capture: {error}"));
    let finish_result = writer
        .finish()
        .map_err(|error| format!("Could not finalize {}: {error}", path.display()));

    capture_result.and(stop_result).and(finish_result)
}

unsafe fn read_capture_format(pointer: *const WAVEFORMATEX) -> Result<CaptureFormat, String> {
    if pointer.is_null() {
        return Err("Windows audio endpoint returned no format.".to_string());
    }
    let base = *pointer;
    let channels = base.nChannels;
    let bits_per_sample = base.wBitsPerSample;
    let bytes_per_sample = usize::from(bits_per_sample / 8);
    if channels == 0 || base.nSamplesPerSec == 0 || bytes_per_sample == 0 {
        return Err("Windows audio endpoint returned an unsupported format.".to_string());
    }
    let kind = match base.wFormatTag {
        WAVE_FORMAT_PCM => SampleKind::Pcm,
        WAVE_FORMAT_IEEE_FLOAT => SampleKind::Float,
        WAVE_FORMAT_EXTENSIBLE if base.cbSize >= 22 => {
            let subformat = std::slice::from_raw_parts((pointer as *const u8).add(24), 16);
            if subformat == PCM_SUBFORMAT {
                SampleKind::Pcm
            } else if subformat == FLOAT_SUBFORMAT {
                SampleKind::Float
            } else {
                return Err("Unsupported Windows extensible audio subformat.".to_string());
            }
        }
        _ => {
            let format_tag = base.wFormatTag;
            return Err(format!(
                "Unsupported Windows audio format tag {}.",
                format_tag
            ));
        }
    };
    Ok(CaptureFormat {
        sample_rate: base.nSamplesPerSec,
        channels,
        block_align: usize::from(base.nBlockAlign),
        bytes_per_sample,
        bits_per_sample,
        kind,
    })
}

fn capture_packets(
    capture: &IAudioCaptureClient,
    writer: &mut WavWriter,
    format: CaptureFormat,
    stop: &AtomicBool,
) -> Result<(), String> {
    while !stop.load(Ordering::Acquire) {
        let packet_frames = unsafe { capture.GetNextPacketSize() }
            .map_err(|error| format!("Failed to read the Windows audio packet size: {error}"))?;
        if packet_frames == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        let mut data = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        unsafe {
            capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .map_err(|error| format!("Failed to read the Windows audio buffer: {error}"))?;
        }

        let result = if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
            writer.write_samples(&vec![0; frames as usize * usize::from(format.channels)])
        } else {
            let byte_count = frames as usize * format.block_align;
            if data.is_null() {
                Err(io::Error::other("Windows audio returned a null buffer"))
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(data, byte_count) };
                let samples = decode_samples(bytes, frames as usize, format);
                writer.write_samples(&samples)
            }
        };

        unsafe { capture.ReleaseBuffer(frames) }
            .map_err(|error| format!("Failed to release the Windows audio buffer: {error}"))?;
        result.map_err(|error| format!("Could not write Windows audio: {error}"))?;
    }
    Ok(())
}

fn decode_samples(bytes: &[u8], frames: usize, format: CaptureFormat) -> Vec<i16> {
    let channels = usize::from(format.channels);
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        for channel in 0..channels {
            let offset = frame * format.block_align + channel * format.bytes_per_sample;
            let sample = match format.kind {
                SampleKind::Pcm => pcm_sample(
                    &bytes[offset..offset + format.bytes_per_sample],
                    format.bits_per_sample,
                ),
                SampleKind::Float => float_sample(&bytes[offset..offset + format.bytes_per_sample]),
            };
            samples.push(sample);
        }
    }
    samples
}

fn pcm_sample(bytes: &[u8], bits: u16) -> i16 {
    match bits {
        8 => ((i16::from(bytes[0]) - 128) << 8).clamp(i16::MIN, i16::MAX),
        16 => i16::from_le_bytes([bytes[0], bytes[1]]),
        24 => {
            let value = i32::from_le_bytes([
                bytes[0],
                bytes[1],
                bytes[2],
                if bytes[2] & 0x80 != 0 { 0xff } else { 0 },
            ]);
            (value >> 8) as i16
        }
        32 => (i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 16) as i16,
        _ => 0,
    }
}

fn float_sample(bytes: &[u8]) -> i16 {
    let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::{decode_samples, CaptureFormat, SampleKind};

    #[test]
    fn converts_float_samples_to_pcm16() {
        let bytes = 0.5f32.to_le_bytes();
        let format = CaptureFormat {
            sample_rate: 48_000,
            channels: 1,
            block_align: 4,
            bytes_per_sample: 4,
            bits_per_sample: 32,
            kind: SampleKind::Float,
        };
        let samples = decode_samples(&bytes, 1, format);
        assert_eq!(samples[0], 16_383);
    }
}
