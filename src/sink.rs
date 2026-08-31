//! Audio output for local playback.
//!
//! librespot ships a rodio sink, but it opens the output device with
//! `.unwrap()` on the player thread, and the release profile aborts on any
//! panic. A Windows PC with no default playback device (nothing in the jack,
//! a Bluetooth headset that is off, a remote desktop session) therefore took
//! the whole app down the moment playback was authorized, before the
//! credential was even stored. This sink opens the device only when playback
//! starts, reports a failure as a sink error (librespot answers by pausing),
//! and tells the interface why, so the app stays up as a Connect remote and
//! plays as soon as an output exists.

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

/// How many chunks may wait in rodio's queue before `write` blocks. Chunks
/// run from a few hundred to a few thousand samples; this is about a fifth
/// of a second, which is also how long a pause takes to be heard, since
/// librespot lets the queue play out first.
const QUEUE_LIMIT: usize = 12;

/// How long `stop` lets the queue play out before pausing regardless.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// How often playback looks at which output the system calls its default.
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// How many engine buffers fit in one second. Ten makes each buffer a
/// tenth of a second: long enough for a music player to ride out a missed
/// deadline without making its controls feel sluggish. Windows needs this
/// instead of its roughly ten-millisecond default (#88); macOS benefits for
/// the same reason when CoreAudio and a Bluetooth device are busy.
#[cfg(any(windows, target_os = "macos", test))]
const ENGINE_BUFFERS_PER_SECOND: u32 = 10;

pub struct RodioSink {
    /// The output device name from Settings; `None` means the default.
    device: Option<String>,
    output: Option<Output>,
    on_error: ErrorHook,
    /// The player's volume, applied here at the output so a change is heard
    /// at once instead of after the queue drains.
    volume: Box<dyn VolumeGetter + Send>,
    applied_volume: f32,
    /// Keeps asking which output the system calls its default.
    watch: Option<DefaultWatch>,
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
    ) -> Self {
        Self {
            device,
            output: None,
            on_error,
            volume,
            applied_volume: -1.0,
            watch: None,
        }
    }

    /// Moves to the system's default output when that changes while the
    /// listener has not picked a device: headphones plugged in, a Bluetooth
    /// speaker connected, another device chosen in the sound settings. The
    /// stream was opened on one device and would keep playing through it
    /// otherwise. Windows and macOS are asked; on Linux PipeWire and
    /// PulseAudio move the stream themselves, and ALSA's answer never
    /// changes, so nothing is asked there. The asking happens on a thread
    /// of its own, since the player's thread has music to deliver on time;
    /// `at_once` asks right now, for the start of playback.
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
        match open_output(self.device.as_deref()) {
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
        if let Some(output) = &self.output {
            output.sink.play();
        }
        Ok(())
    }

    /// Never fails: librespot exits the process when a sink cannot stop.
    fn stop(&mut self) -> SinkResult<()> {
        if let Some(output) = &self.output {
            let deadline = Instant::now() + DRAIN_TIMEOUT;
            while !output.sink.empty() && !output.failed() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            output.sink.pause();
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
        output.sink.append(rodio::buffer::SamplesBuffer::new(
            NUM_CHANNELS as rodio::ChannelCount,
            output.sample_rate as rodio::SampleRate,
            samples,
        ));
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
/// device, else at whatever rodio can find. The first two carry the
/// engine buffer Windows and macOS need; rodio's own fallback would not.
fn open_stream(
    device: &cpal::Device,
    on_error: impl FnMut(cpal::StreamError) + Send + Clone + 'static,
) -> Result<rodio::OutputStream, rodio::StreamError> {
    #[cfg(target_os = "macos")]
    let supported_buffer = device
        .default_output_config()
        .map(|config| *config.buffer_size())
        .unwrap_or(cpal::SupportedBufferSize::Unknown);
    let builder = |sample_rate: u32| -> Result<_, rodio::StreamError> {
        let builder = rodio::OutputStreamBuilder::from_device(device.clone())?
            .with_channels(NUM_CHANNELS as rodio::ChannelCount)
            .with_sample_rate(sample_rate as rodio::SampleRate)
            .with_error_callback(on_error.clone());
        #[cfg(windows)]
        let builder = builder.with_buffer_size(cpal::BufferSize::Fixed(
            sample_rate / ENGINE_BUFFERS_PER_SECOND,
        ));
        #[cfg(target_os = "macos")]
        let builder = match macos_engine_buffer(sample_rate, supported_buffer) {
            Some(buffer) => builder.with_buffer_size(buffer),
            None => builder,
        };
        Ok(builder)
    };
    if let Ok(stream) = builder(SAMPLE_RATE)?.open_stream() {
        return Ok(stream);
    }
    if let Ok(config) = device.default_output_config()
        && let Ok(stream) = builder(config.sample_rate().0)?.open_stream()
    {
        return Ok(stream);
    }
    builder(SAMPLE_RATE)?.open_stream_or_fallback()
}

/// CoreAudio rejects a fixed buffer outside the device's advertised range.
/// Aim for a tenth of a second, then stay within that range; if the driver
/// cannot describe one, leave its default alone instead of risking a stream
/// that no longer opens.
#[cfg(any(target_os = "macos", test))]
fn macos_engine_buffer(
    sample_rate: u32,
    supported: cpal::SupportedBufferSize,
) -> Option<cpal::BufferSize> {
    let target = (sample_rate / ENGINE_BUFFERS_PER_SECOND).max(1);
    match supported {
        cpal::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            Some(cpal::BufferSize::Fixed(target.clamp(min.max(1), max)))
        }
        cpal::SupportedBufferSize::Range { .. } | cpal::SupportedBufferSize::Unknown => None,
    }
}

/// The player's thread decodes the music and hands it here with about a
/// fifth of a second in hand. Under load a PC gives the foreground app
/// the cores first, and a fifth of a second is soon gone; Windows lets a
/// thread ask for precedence, so this one asks for a step above normal:
/// ahead of an app's ordinary threads, behind the audio engine's own,
/// and never so high that a stuck loop here could hold the machine. (#88)
///
/// Windows and macOS both have a knob an unprivileged thread can turn. On
/// Linux a thread cannot raise itself without rtkit.
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

#[cfg(target_os = "macos")]
fn take_precedence() {
    // SAFETY: this changes only the calling thread's scheduling class. Zero
    // is the documented relative priority for this QoS class.
    let result = unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INITIATED, 0)
    };
    if result != 0 {
        log::warn!(
            "cannot prioritize the audio producer thread: {}",
            std::io::Error::from_raw_os_error(result)
        );
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn take_precedence() {}

/// The name of the system's default output, as last asked. Asking
/// Windows means making a device enumerator and reading a property store,
/// which some driver stacks take their time over, so a thread of its own
/// asks every couple of seconds and the player's thread only reads the
/// answer. The thread ends when the sink that started it is gone.
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

    /// The answer as last asked; `None` before the first answer.
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

fn open_output(preferred: Option<&str>) -> Result<Output, OpenError> {
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
    let mut stream = open_stream(&device, on_error)?;
    stream.log_on_drop(false);
    let sample_rate = stream.config().sample_rate();
    if let &cpal::BufferSize::Fixed(frames) = stream.config().buffer_size() {
        let milliseconds = u64::from(frames) * 1000 / u64::from(sample_rate);
        log::info!("audio engine buffer: {frames} frames ({milliseconds} ms)");
    }
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn macos_engine_buffer_holds_a_tenth_second_within_device_limits() {
        use cpal::{BufferSize, SupportedBufferSize};

        assert_eq!(
            macos_engine_buffer(44_100, SupportedBufferSize::Range { min: 32, max: 8192 }),
            Some(BufferSize::Fixed(4410))
        );
        assert_eq!(
            macos_engine_buffer(48_000, SupportedBufferSize::Range { min: 32, max: 4096 }),
            Some(BufferSize::Fixed(4096))
        );
        assert_eq!(
            macos_engine_buffer(
                44_100,
                SupportedBufferSize::Range {
                    min: 8192,
                    max: 16384
                }
            ),
            Some(BufferSize::Fixed(8192))
        );
        assert_eq!(
            macos_engine_buffer(44_100, SupportedBufferSize::Unknown),
            None
        );
        assert_eq!(
            macos_engine_buffer(44_100, SupportedBufferSize::Range { min: 512, max: 256 }),
            None
        );
    }

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
