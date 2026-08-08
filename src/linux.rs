use super::{CaptureMode, HelperError, PermissionStatus};
use crate::audio::{f32le_to_pcm16, WavWriter};
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::{
    param::format::{MediaSubtype, MediaType},
    param::format_utils,
    pod::Pod,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Copy, Debug)]
struct CaptureFormat {
    sample_rate: u32,
    channels: u16,
}

enum CaptureMessage {
    Format(CaptureFormat),
    Frames(Vec<u8>),
}

enum Control {
    Stop,
    Error(String),
}

struct StreamData {
    format: spa::param::audio::AudioInfoRaw,
    ready_sender: Option<SyncSender<Result<(), String>>>,
    frame_sender: Option<SyncSender<CaptureMessage>>,
    control_sender: pw::channel::Sender<Control>,
    announced: bool,
}

pub struct CaptureProcess {
    stop: Arc<AtomicBool>,
    workers: Vec<Worker>,
}

struct Worker {
    handle: JoinHandle<Result<(), String>>,
    control_sender: pw::channel::Sender<Control>,
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
                    "Linux PipeWire capture worker exited unexpectedly.".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub fn stop(self) -> Result<(), HelperError> {
        self.stop.store(true, Ordering::Release);
        for worker in &self.workers {
            let _ = worker.control_sender.send(Control::Stop);
        }

        let mut first_error = None;
        for worker in self.workers {
            match worker.handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
                Ok(Err(_)) => {}
                Err(_) if first_error.is_none() => {
                    first_error = Some("Linux PipeWire capture worker panicked.".to_string())
                }
                Err(_) => {}
            }
        }

        first_error
            .map(|error| Err(HelperError::Backend(error)))
            .unwrap_or(Ok(()))
    }

    fn start_source(&mut self, path: Option<PathBuf>, loopback: bool) -> Result<(), HelperError> {
        let stop = Arc::clone(&self.stop);
        let (control_ready_sender, control_ready_receiver) = mpsc::sync_channel(1);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (error_sender, error_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            run_capture(
                path,
                loopback,
                stop,
                control_ready_sender,
                ready_sender,
                error_sender,
            )
        });

        let control_sender = match control_ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(sender)) => sender,
            Ok(Err(error)) => {
                let _ = handle.join();
                return Err(HelperError::Backend(error));
            }
            Err(_) => {
                let _ = handle.join();
                return Err(HelperError::Backend(
                    "PipeWire did not become ready within five seconds. Is the PipeWire daemon running?"
                        .to_string(),
                ));
            }
        };

        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {
                self.workers.push(Worker {
                    handle,
                    control_sender,
                    errors: error_receiver,
                });
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = control_sender.send(Control::Stop);
                let _ = handle.join();
                Err(HelperError::Backend(error))
            }
            Err(_) => {
                let _ = control_sender.send(Control::Stop);
                let _ = handle.join();
                Err(HelperError::Backend(
                    "PipeWire did not negotiate an audio format within five seconds. No suitable default audio device was found."
                        .to_string(),
                ))
            }
        }
    }
}

pub fn check_permissions(mode: CaptureMode) -> Result<PermissionStatus, HelperError> {
    let mut process = CaptureProcess {
        stop: Arc::new(AtomicBool::new(false)),
        workers: Vec::new(),
    };

    let result = (|| {
        if mode.requires_screen_recording() {
            process.start_source(None, true)?;
        }
        if mode.requires_microphone() {
            process.start_source(None, false)?;
        }
        Ok::<(), HelperError>(())
    })();

    if let Err(error) = result {
        let _ = process.stop();
        return Err(error);
    }
    process.stop()?;

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
    // Linux has no Rusteze-owned permission prompt. PipeWire connection and
    // stream negotiation surface desktop-session or sandbox restrictions.
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
            process.start_source(Some(session_folder.join("system.wav")), true)?;
        }
        if mode.requires_microphone() {
            process.start_source(Some(session_folder.join("mic.wav")), false)?;
        }
        Ok::<(), HelperError>(())
    })();

    if let Err(error) = start_result {
        let _ = process.stop();
        return Err(error);
    }

    Ok(process)
}

fn run_capture(
    path: Option<PathBuf>,
    loopback: bool,
    stop: Arc<AtomicBool>,
    control_ready_sender: SyncSender<Result<pw::channel::Sender<Control>, String>>,
    ready_sender: SyncSender<Result<(), String>>,
    error_sender: mpsc::Sender<String>,
) -> Result<(), String> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| format!("Could not create the PipeWire main loop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("Could not create the PipeWire context: {error}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| format!("PipeWire is not available on this system: {error}"))?;

    let (control_sender, control_receiver) = pw::channel::channel();
    let control_loop = mainloop.clone();
    let _control_receiver =
        control_receiver.attach(mainloop.loop_(), move |control| match control {
            Control::Stop => control_loop.quit(),
            Control::Error(error) => {
                let _ = error_sender.send(error);
                control_loop.quit();
            }
        });

    let mut stream_properties = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
    };
    if loopback {
        stream_properties.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
    }

    let stream = match pw::stream::StreamBox::new(&core, "rusteze-capture", stream_properties) {
        Ok(stream) => stream,
        Err(error) => {
            let message = format!("Failed to create PipeWire capture stream: {error}");
            let _ = control_ready_sender.send(Err(message.clone()));
            return Err(message);
        }
    };

    let (frame_sender, frame_receiver) = mpsc::sync_channel(32);
    let writer_handle = path.as_ref().map(|path| {
        let path = path.clone();
        let control_sender = control_sender.clone();
        thread::spawn(move || writer_loop(&path, frame_receiver, control_sender))
    });

    let data = StreamData {
        format: spa::param::audio::AudioInfoRaw::new(),
        ready_sender: Some(ready_sender),
        frame_sender: path.as_ref().map(|_| frame_sender),
        control_sender: control_sender.clone(),
        announced: false,
    };

    let listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_, data, _, new| {
            if let pw::stream::StreamState::Error(error) = new {
                report_stream_error(data, format!("PipeWire capture stream failed: {error}"));
            }
        })
        .param_changed(|_, data, id, param| {
            if id != spa::param::ParamType::Format.as_raw() || data.announced {
                return;
            }
            let Some(param) = param else {
                return;
            };

            let (media_type, media_subtype) = match format_utils::parse_format(param) {
                Ok(format) => format,
                Err(error) => {
                    report_stream_error(
                        data,
                        format!("Could not parse PipeWire audio format: {error}"),
                    );
                    return;
                }
            };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                report_stream_error(data, "PipeWire returned a non-raw audio format".to_string());
                return;
            }
            if let Err(error) = data.format.parse(param) {
                report_stream_error(
                    data,
                    format!("Could not read PipeWire audio format: {error}"),
                );
                return;
            }
            if data.format.format() != spa::param::audio::AudioFormat::F32LE {
                report_stream_error(
                    data,
                    format!(
                        "PipeWire negotiated unsupported audio format: {:?}",
                        data.format.format()
                    ),
                );
                return;
            }
            if data.format.rate() == 0 || data.format.channels() == 0 {
                report_stream_error(
                    data,
                    "PipeWire negotiated an invalid audio format".to_string(),
                );
                return;
            }

            let capture_format = CaptureFormat {
                sample_rate: data.format.rate(),
                channels: data.format.channels() as u16,
            };
            if let Some(sender) = data.frame_sender.as_ref().cloned() {
                if sender.send(CaptureMessage::Format(capture_format)).is_err() {
                    report_stream_error(
                        data,
                        "PipeWire audio writer stopped unexpectedly".to_string(),
                    );
                    return;
                }
            }
            if let Some(sender) = data.ready_sender.take() {
                let _ = sender.send(Ok(()));
            }
            data.announced = true;
        })
        .process(|stream, data| {
            let Some(sender) = data.frame_sender.as_ref().cloned() else {
                return;
            };
            let Some(mut buffer) = stream.dequeue_buffer() else {
                report_stream_error(
                    data,
                    "PipeWire capture stream returned no buffer".to_string(),
                );
                return;
            };
            let datas = buffer.datas_mut();
            let Some(audio_data) = datas.first_mut() else {
                report_stream_error(
                    data,
                    "PipeWire capture stream returned no audio data".to_string(),
                );
                return;
            };
            let offset = audio_data.chunk().offset() as usize;
            let size = audio_data.chunk().size() as usize;
            let Some(bytes) = audio_data.data() else {
                report_stream_error(
                    data,
                    "PipeWire capture stream returned no mapped buffer".to_string(),
                );
                return;
            };
            let Some(end) = offset.checked_add(size) else {
                report_stream_error(
                    data,
                    "PipeWire returned an invalid audio buffer range".to_string(),
                );
                return;
            };
            if end > bytes.len() {
                report_stream_error(
                    data,
                    "PipeWire returned an audio buffer outside its mapped memory".to_string(),
                );
                return;
            }
            if sender
                .try_send(CaptureMessage::Frames(bytes[offset..end].to_vec()))
                .is_err()
            {
                report_stream_error(
                    data,
                    "PipeWire audio writer could not keep up; stopping to avoid corrupt output"
                        .to_string(),
                );
            }
        })
        .register()
        .map_err(|error| format!("Failed to register PipeWire capture callbacks: {error}"))?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(object),
    )
    .map_err(|error| format!("Could not build PipeWire audio format: {error}"))?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| {
        "Could not encode PipeWire audio format: serialized POD was invalid".to_string()
    })?];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|error| format!("Failed to connect PipeWire capture stream: {error}"))?;

    if control_ready_sender.send(Ok(control_sender)).is_err() {
        return Err("Rusteze stopped waiting for PipeWire capture startup".to_string());
    }

    mainloop.run();
    drop(listener);
    drop(stream);

    let writer_result = writer_handle.map(|handle| match handle.join() {
        Ok(result) => result,
        Err(_) => Err("Linux PipeWire audio writer panicked".to_string()),
    });
    if let Some(result) = writer_result {
        result?;
    }

    if stop.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err("Linux PipeWire capture stopped unexpectedly".to_string())
    }
}

fn report_stream_error(data: &mut StreamData, message: String) {
    if let Some(sender) = data.ready_sender.take() {
        let _ = sender.send(Err(message.clone()));
    }
    let _ = data.control_sender.send(Control::Error(message));
}

fn writer_loop(
    path: &Path,
    receiver: Receiver<CaptureMessage>,
    control_sender: pw::channel::Sender<Control>,
) -> Result<(), String> {
    let format = match receiver.recv() {
        Ok(CaptureMessage::Format(format)) => format,
        Ok(CaptureMessage::Frames(_)) => {
            return Err("PipeWire delivered audio before format negotiation".to_string())
        }
        Err(_) => return Err("PipeWire stream ended before format negotiation".to_string()),
    };
    let mut writer = match WavWriter::create(path, format.sample_rate, format.channels) {
        Ok(writer) => writer,
        Err(error) => {
            let message = format!("Could not create {}: {error}", path.display());
            let _ = control_sender.send(Control::Error(message.clone()));
            return Err(message);
        }
    };

    while let Ok(message) = receiver.recv() {
        match message {
            CaptureMessage::Format(_) => {}
            CaptureMessage::Frames(bytes) => {
                let samples = f32le_to_pcm16(&bytes).map_err(|error| error.to_string());
                let samples = match samples {
                    Ok(samples) => samples,
                    Err(error) => {
                        let _ = control_sender.send(Control::Error(error.clone()));
                        return Err(error);
                    }
                };
                if let Err(error) = writer.write_samples(&samples) {
                    let message = format!("Could not write {}: {error}", path.display());
                    let _ = control_sender.send(Control::Error(message.clone()));
                    let _ = writer.finish();
                    return Err(message);
                }
            }
        }
    }

    writer
        .finish()
        .map_err(|error| format!("Could not finalize {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::CaptureFormat;

    #[test]
    fn preserves_negotiated_rate_and_channels() {
        let format = CaptureFormat {
            sample_rate: 44_100,
            channels: 1,
        };
        assert_eq!(format.sample_rate, 44_100);
        assert_eq!(format.channels, 1);
    }
}
