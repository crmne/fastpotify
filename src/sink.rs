//! Audio output for local playback.
//!
//! librespot's rodio sink panics if no output device is available. Release
//! builds abort on that panic. This sink opens the device when playback starts
//! and reports failures through the UI. Fastpotify can then remain available
//! as a Connect remote until an output appears.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};
use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};

use crate::resample::Resampler;

/// The backend name Settings uses for this sink.
pub const NAME: &str = "rodio";

/// Told about output failures, with a message fit for the interface.
pub type ErrorHook = Arc<dyn Fn(String) + Send + Sync>;

/// Maximum queued rodio chunks before `write` blocks, about 200 ms of audio.
const QUEUE_LIMIT: usize = 12;

/// Maximum time `stop` waits for the queue to drain.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// How often playback looks at which output the system calls its default.
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Default Windows device buffer length in milliseconds.
///
/// Small platform defaults can click under load (#88). A 100 ms buffer avoids
/// these underruns while keeping controls responsive.
pub const DEFAULT_BUFFER_MS: u32 = 100;

/// Allowed Windows device buffer range. Lower values can click; higher values
/// delay playback controls.
pub const BUFFER_MS_RANGE: std::ops::RangeInclusive<u32> = 20..=500;

/// The buffer to ask the device for, in frames.
///
/// Clamp to the reported range because CoreAudio rejects unsupported sizes.
/// If a device reports no range, request the configured size; `open_stream`
/// can retry without a fixed size.
fn engine_buffer(
    sample_rate: u32,
    ms: u32,
    supported: cpal::SupportedBufferSize,
) -> cpal::BufferSize {
    let ms = ms.clamp(*BUFFER_MS_RANGE.start(), *BUFFER_MS_RANGE.end());
    let frames = (u64::from(sample_rate) * u64::from(ms) / 1000).max(1) as u32;
    match supported {
        cpal::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            cpal::BufferSize::Fixed(frames.clamp(min.max(1), max))
        }
        _ => cpal::BufferSize::Fixed(frames),
    }
}

pub struct RodioSink {
    /// The output device name from Settings; `None` means the default.
    device: Option<String>,
    output: Option<Output>,
    on_error: ErrorHook,
    /// Player volume, applied at output so changes affect queued audio.
    volume: Box<dyn VolumeGetter + Send>,
    applied_volume: f32,
    /// Watches for changes to the default output.
    watch: Option<DefaultWatch>,
    /// How much sound to ask the device to hold, in milliseconds. Taken
    /// when the stream opens, so a change lands with the next restart.
    buffer_ms: u32,
}

struct Output {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
    /// The name of the device the stream was opened on.
    device_name: Option<String>,
    /// Set from the audio thread when the stream dies (device unplugged).
    failed: Arc<AtomicBool>,
    /// The rate the stream runs at, and the converter to it when that is
    /// not Spotify's.
    sample_rate: u32,
    resampler: Option<Resampler>,
    /// Whether this track has supplied audio since its last stop.
    fed: bool,
    last_write: Option<Instant>,
}

impl Output {
    fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

impl RodioSink {
    pub fn new(
        device: Option<String>,
        on_error: ErrorHook,
        volume: Box<dyn VolumeGetter + Send>,
        buffer_ms: u32,
    ) -> Self {
        Self {
            device,
            output: None,
            on_error,
            volume,
            applied_volume: -1.0,
            watch: None,
            buffer_ms,
        }
    }

    /// Follows the system default output when no device is selected.
    ///
    /// Windows and macOS need explicit polling. PipeWire and PulseAudio move
    /// streams themselves, while ALSA's answer does not change. Polling runs
    /// off the player thread. `at_once` requests a fresh value at playback
    /// start.
    fn follow_default(&mut self, at_once: bool) {
        if cfg!(target_os = "linux") || self.device.is_some() {
            return;
        }
        let Some(output) = &self.output else {
            return;
        };
        let watch = self.watch.get_or_insert_with(DefaultWatch::start);
        let current = if at_once { watch.ask() } else { watch.name() };
        if current.is_some() && current != output.device_name {
            log::info!(
                "the default audio output is now {}; moving playback to it",
                current.as_deref().unwrap_or("[unknown device]")
            );
            self.output = None;
        }
    }

    fn apply_volume(&mut self) {
        let factor = self.volume.attenuation_factor() as f32;
        if let Some(output) = &self.output
            && factor != self.applied_volume
        {
            output.sink.set_volume(factor);
            self.applied_volume = factor;
        }
    }

    /// Opens the output if it is not open, or if it died since.
    fn ensure_open(&mut self) -> SinkResult<()> {
        if self.output.as_ref().is_some_and(Output::failed) {
            log::warn!("the audio output stopped working; reopening it");
            self.output = None;
        }
        if self.output.is_some() {
            return Ok(());
        }
        match open_output(self.device.as_deref(), self.buffer_ms) {
            Ok(output) => {
                self.output = Some(output);
                self.applied_volume = -1.0;
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                log::error!("{message}");
                (self.on_error)(message.clone());
                Err(SinkError::ConnectionRefused(message))
            }
        }
    }
}

impl Sink for RodioSink {
    fn start(&mut self) -> SinkResult<()> {
        take_precedence();
        self.follow_default(true);
        self.ensure_open()?;
        self.apply_volume();
        if let Some(output) = &mut self.output {
            output.sink.play();
        }
        Ok(())
    }

    /// Never fails: librespot exits the process when a sink cannot stop.
    fn stop(&mut self) -> SinkResult<()> {
        if let Some(output) = &mut self.output {
            let deadline = Instant::now() + DRAIN_TIMEOUT;
            while !output.sink.empty() && !output.failed() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            output.sink.pause();
            output.fed = false;
            output.last_write = None;
        }
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|error| SinkError::OnWrite(error.to_string()))?;
        let samples = converter.f64_to_f32(samples);
        self.follow_default(false);
        self.ensure_open()?;
        self.apply_volume();
        let Some(output) = &mut self.output else {
            return Err(SinkError::NotConnected(
                "the audio output is not open".into(),
            ));
        };
        let samples = match &mut output.resampler {
            Some(resampler) => resampler.process(&samples),
            None => samples,
        };
        let now = Instant::now();
        if output.fed && output.sink.empty() && !output.sink.is_paused() {
            let late_ms = output
                .last_write
                .map(|last| now.duration_since(last).as_millis())
                .unwrap_or(0);
            log::warn!("audio queue ran dry; next packet arrived after {late_ms} ms");
        }
        output.sink.append(rodio::buffer::SamplesBuffer::new(
            NUM_CHANNELS as rodio::ChannelCount,
            output.sample_rate as rodio::SampleRate,
            samples,
        ));
        output.fed = true;
        output.last_write = Some(now);
        // Let rodio drain a little; without this the whole track would be
        // decoded into memory at once.
        while output.sink.len() > QUEUE_LIMIT {
            if output.failed() {
                let message = "The audio output stopped working".to_string();
                (self.on_error)(message.clone());
                return Err(SinkError::OnWrite(message));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

/// Opens the stream at Spotify's stereo 44.1 kHz, so nothing is converted,
/// else at the device's own rate, which Windows insists on for a shared
/// device, else at whatever rodio can find.
///
/// The first two attempts request the configured buffer. The fallback lets
/// the driver choose its buffer size.
fn open_stream(
    device: &cpal::Device,
    on_error: impl FnMut(cpal::StreamError) + Send + Clone + 'static,
    buffer_ms: u32,
) -> Result<rodio::OutputStream, rodio::StreamError> {
    let supported = device
        .default_output_config()
        .map(|config| *config.buffer_size())
        .unwrap_or(cpal::SupportedBufferSize::Unknown);
    let builder = |sample_rate: u32, buffer: bool| -> Result<_, rodio::StreamError> {
        let builder = rodio::OutputStreamBuilder::from_device(device.clone())?
            .with_channels(NUM_CHANNELS as rodio::ChannelCount)
            .with_sample_rate(sample_rate as rodio::SampleRate)
            .with_error_callback(on_error.clone());
        Ok(if buffer {
            builder.with_buffer_size(engine_buffer(sample_rate, buffer_ms, supported))
        } else {
            builder
        })
    };
    // The fixed engine buffer addresses Windows shared-mode underruns (#88).
    // CoreAudio, ALSA, PulseAudio, and PipeWire keep their proven
    // driver-selected callback periods.
    let fixed_buffer = cfg!(windows);
    if let Ok(stream) = builder(SAMPLE_RATE, fixed_buffer)?.open_stream() {
        return Ok(stream);
    }
    if let Ok(config) = device.default_output_config()
        && let Ok(stream) = builder(config.sample_rate().0, fixed_buffer)?.open_stream()
    {
        return Ok(stream);
    }
    builder(SAMPLE_RATE, false)?.open_stream_or_fallback()
}

/// Raises the Windows decoder thread one step above normal to prevent queued
/// audio from running out under load (#88).
///
/// Linux requires rtkit; CoreAudio owns its real-time callback on macOS.
#[cfg(windows)]
fn take_precedence() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    // SAFETY: the current thread's pseudo-handle needs no closing, and the
    // call takes nothing else.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}

#[cfg(not(windows))]
fn take_precedence() {}

/// Last default-output name, polled on a worker thread because Windows device
/// enumeration can block. The thread ends when the sink is dropped.
struct DefaultWatch(Arc<Mutex<Option<String>>>);

impl DefaultWatch {
    fn start() -> Self {
        let shared = Arc::new(Mutex::new(None));
        let weak = Arc::downgrade(&shared);
        let watching = thread::Builder::new()
            .name("audio-default-watch".into())
            .spawn(move || {
                while let Some(shared) = weak.upgrade() {
                    let name = default_output_name();
                    *shared.lock().unwrap_or_else(PoisonError::into_inner) = name;
                    drop(shared);
                    thread::sleep(DEFAULT_CHECK_INTERVAL);
                }
            });
        if let Err(error) = watching {
            log::warn!("cannot watch the default audio output: {error}");
        }
        Self(shared)
    }

    /// Last polled name, or `None` before the first poll.
    fn name(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Asks right now, on this thread.
    fn ask(&self) -> Option<String> {
        let name = default_output_name();
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = name.clone();
        name
    }
}

fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|device| device.name().ok())
}

#[derive(Debug, thiserror::Error)]
enum OpenError {
    #[error("No audio output device was found. Connect or enable one, then press play again.")]
    NoDevice,
    #[error("Cannot list the audio devices: {0}")]
    Devices(#[from] cpal::DevicesError),
    #[error("Cannot open the audio output: {0}")]
    Stream(#[from] rodio::StreamError),
}

fn open_output(preferred: Option<&str>, buffer_ms: u32) -> Result<Output, OpenError> {
    let host = cpal::default_host();
    let device = match preferred.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => {
            let chosen = host
                .output_devices()?
                .find(|device| device.name().is_ok_and(|found| found == name));
            match chosen {
                Some(device) => device,
                None => {
                    log::warn!("audio device {name:?} is not available; using the default");
                    host.default_output_device().ok_or(OpenError::NoDevice)?
                }
            }
        }
        None => host.default_output_device().ok_or(OpenError::NoDevice)?,
    };
    let device_name = device.name().ok();
    log::info!(
        "audio output: {}",
        device_name.as_deref().unwrap_or("[unknown device]")
    );

    let failed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&failed);
    let on_error = move |error: cpal::StreamError| {
        log::error!("audio stream error: {error}");
        flag.store(true, Ordering::Relaxed);
    };
    let mut stream = open_stream(&device, on_error, buffer_ms)?;
    stream.log_on_drop(false);
    let sample_rate = stream.config().sample_rate();
    let resampler = Resampler::new(SAMPLE_RATE, sample_rate, NUM_CHANNELS as usize);
    if resampler.is_some() {
        log::info!(
            "the output runs at {sample_rate} Hz; the music is converted from {SAMPLE_RATE} Hz"
        );
    }
    let sink = rodio::Sink::connect_new(stream.mixer());
    Ok(Output {
        sink,
        _stream: stream,
        device_name,
        failed,
        sample_rate,
        resampler,
        fed: false,
        last_write: None,
    })
}

#[cfg(test)]
mod tests {

    /// Converts the configured buffer duration to device frames.
    #[test]
    fn the_buffer_follows_the_setting_and_the_rate() {
        let unknown = cpal::SupportedBufferSize::Unknown;
        assert_eq!(
            engine_buffer(44_100, 100, unknown),
            cpal::BufferSize::Fixed(4410),
            "a tenth of a second at 44.1 kHz"
        );
        assert_eq!(
            engine_buffer(48_000, 100, unknown),
            cpal::BufferSize::Fixed(4800),
            "the same tenth of a second at 48 kHz"
        );
        assert_eq!(
            engine_buffer(44_100, 20, unknown),
            cpal::BufferSize::Fixed(882)
        );
    }

    /// Clamps the buffer to the device range required by CoreAudio.
    #[test]
    fn a_device_that_states_its_range_is_kept_inside_it() {
        let range = cpal::SupportedBufferSize::Range { min: 64, max: 2048 };
        assert_eq!(
            engine_buffer(44_100, 100, range),
            cpal::BufferSize::Fixed(2048),
            "held down to what the device can take"
        );
        assert_eq!(
            engine_buffer(44_100, 20, range),
            cpal::BufferSize::Fixed(882),
            "and left alone when it fits"
        );
        let tiny = cpal::SupportedBufferSize::Range {
            min: 4096,
            max: 8192,
        };
        assert_eq!(
            engine_buffer(44_100, 20, tiny),
            cpal::BufferSize::Fixed(4096),
            "and brought up to a device that will not go smaller"
        );
    }

    /// Rule: a settings file with a wild number in it still opens a
    /// stream. The range is the range whoever wrote the file thought of.
    #[test]
    fn a_number_from_outside_the_range_is_brought_back_in() {
        let unknown = cpal::SupportedBufferSize::Unknown;
        assert_eq!(
            engine_buffer(44_100, 0, unknown),
            engine_buffer(44_100, *BUFFER_MS_RANGE.start(), unknown)
        );
        assert_eq!(
            engine_buffer(44_100, 100_000, unknown),
            engine_buffer(44_100, *BUFFER_MS_RANGE.end(), unknown)
        );
    }
    use super::*;
    use std::sync::Mutex;

    /// A machine without audio (CI, a PC with nothing plugged in) must get
    /// an error and a message for the interface, never a panic. A machine
    /// with audio opens its default device.
    #[test]
    fn starting_without_a_device_is_an_error_not_a_panic() {
        let reported: Arc<Mutex<Option<String>>> = Arc::default();
        let store = Arc::clone(&reported);
        let mut sink = RodioSink::new(
            Some("no such device".into()),
            Arc::new(move |message| *store.lock().unwrap() = Some(message)),
            Box::new(librespot_playback::mixer::NoOpVolume),
            DEFAULT_BUFFER_MS,
        );
        match sink.start() {
            Ok(()) => assert!(reported.lock().unwrap().is_none()),
            Err(SinkError::ConnectionRefused(message)) => {
                assert_eq!(reported.lock().unwrap().as_deref(), Some(message.as_str()));
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
        assert!(sink.stop().is_ok());
    }
}
