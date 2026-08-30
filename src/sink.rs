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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};
use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};

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

pub struct RodioSink {
    /// The output device name from Settings; `None` means the default.
    device: Option<String>,
    output: Option<Output>,
    on_error: ErrorHook,
    /// The player's volume, applied here at the output so a change is heard
    /// at once instead of after the queue drains.
    volume: Box<dyn VolumeGetter + Send>,
    applied_volume: f32,
    default_checked_at: Option<Instant>,
}

struct Output {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
    /// The name of the device the stream was opened on.
    device_name: Option<String>,
    /// Set from the audio thread when the stream dies (device unplugged).
    failed: Arc<AtomicBool>,
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
            default_checked_at: None,
        }
    }

    /// Moves to the system's default output when that changes while the
    /// listener has not picked a device: headphones plugged in, a Bluetooth
    /// speaker connected, another device chosen in the sound settings. The
    /// stream was opened on one device and would keep playing through it
    /// otherwise. Asking is cheap on Windows and macOS. On Linux PipeWire
    /// and PulseAudio move the stream themselves, and ALSA's answer never
    /// changes, so nothing is asked there.
    fn follow_default(&mut self, at_once: bool) {
        if cfg!(target_os = "linux") || self.device.is_some() {
            return;
        }
        let Some(output) = &self.output else {
            return;
        };
        if !at_once
            && self
                .default_checked_at
                .is_some_and(|at| at.elapsed() < DEFAULT_CHECK_INTERVAL)
        {
            return;
        }
        self.default_checked_at = Some(Instant::now());
        let current = cpal::default_host()
            .default_output_device()
            .and_then(|device| device.name().ok());
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
                let message = error.user_message();
                log::error!("{message}");
                (self.on_error)(message.clone());
                Err(SinkError::ConnectionRefused(message))
            }
        }
    }
}

impl Sink for RodioSink {
    fn start(&mut self) -> SinkResult<()> {
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
        let Some(output) = &self.output else {
            return Err(SinkError::NotConnected(
                "the audio output is not open".into(),
            ));
        };
        output.sink.append(rodio::buffer::SamplesBuffer::new(
            NUM_CHANNELS as rodio::ChannelCount,
            SAMPLE_RATE as rodio::SampleRate,
            samples,
        ));
        // Let rodio drain a little; without this the whole track would be
        // decoded into memory at once.
        while output.sink.len() > QUEUE_LIMIT {
            if output.failed() {
                let message = crate::i18n::keyed("error.audio_stopped");
                (self.on_error)(message.clone());
                return Err(SinkError::OnWrite(message));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
enum OpenError {
    #[error("error.no_audio_device")]
    NoDevice,
    #[error("error.list_devices")]
    Devices(#[from] cpal::DevicesError),
    #[error("error.open_audio")]
    Stream(#[from] rodio::StreamError),
}

impl OpenError {
    fn user_message(&self) -> String {
        match self {
            Self::NoDevice => crate::i18n::keyed("error.no_audio_device"),
            Self::Devices(error) => {
                crate::i18n::keyed_args("error.list_devices", &[("error", &error.to_string())])
            }
            Self::Stream(error) => {
                crate::i18n::keyed_args("error.open_audio", &[("error", &error.to_string())])
            }
        }
    }
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
    // Spotify's native stereo 44.1 kHz first, so nothing is resampled; rodio
    // falls back to whatever the device does support.
    let mut stream = rodio::OutputStreamBuilder::from_device(device)?
        .with_channels(NUM_CHANNELS as rodio::ChannelCount)
        .with_sample_rate(SAMPLE_RATE as rodio::SampleRate)
        .with_error_callback(move |error: cpal::StreamError| {
            log::error!("audio stream error: {error}");
            flag.store(true, Ordering::Relaxed);
        })
        .open_stream_or_fallback()?;
    stream.log_on_drop(false);
    let sink = rodio::Sink::connect_new(stream.mixer());
    Ok(Output {
        sink,
        _stream: stream,
        device_name,
        failed,
    })
}

#[cfg(test)]
mod tests {
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
