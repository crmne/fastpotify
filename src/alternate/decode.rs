//! Dedicated decoder thread and a non-blocking PCM source for the audio sink.

use std::io::{Read, Seek};
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio::{Source, source::SeekError};

use super::buffer::{BufferWait, SharedAudio};

const CHUNK_SAMPLES: usize = 2048;
const CHUNK_QUEUE: usize = 64;
const PHASE_START: u8 = 0;
const PHASE_RUN: u8 = 1;
const PHASE_ENDED: u8 = 2;
const PHASE_FAILED: u8 = 3;
const PHASE_CANCEL: u8 = 4;

#[cfg(test)]
pub(crate) const TONE_MP3: &[u8] = include_bytes!("fixtures/tone.mp3");
#[cfg(test)]
pub(crate) const TONE_M4A: &[u8] = include_bytes!("fixtures/tone.m4a");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatHint {
    pub extension: Option<String>,
    pub mime: Option<String>,
}

impl FormatHint {
    pub fn from_labels(format: Option<&str>, mime: Option<&str>, url: Option<&str>) -> Self {
        let extension = format
            .and_then(normalize_ext)
            .or_else(|| mime.and_then(ext_from_mime))
            .or_else(|| url.and_then(ext_from_url));
        Self {
            extension,
            mime: mime.map(str::to_string),
        }
    }
}

fn normalize_ext(raw: &str) -> Option<String> {
    let ext = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "mp3" | "mpeg" | "mpga" => Some("mp3".into()),
        "m4a" | "mp4" | "aac" => Some("m4a".into()),
        "wav" | "wave" => Some("wav".into()),
        other if other.len() <= 8 && other.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
            Some(other.to_string())
        }
        _ => None,
    }
}

fn ext_from_mime(mime: &str) -> Option<String> {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("mpeg") || mime.contains("mp3") {
        Some("mp3".into())
    } else if mime.contains("mp4") || mime.contains("aac") || mime.contains("m4a") {
        Some("m4a".into())
    } else if mime.contains("wav") {
        Some("wav".into())
    } else {
        None
    }
}

fn ext_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('.').next().and_then(normalize_ext)
}

struct DecodeState {
    phase: AtomicU8,
    channels: AtomicU16,
    sample_rate: AtomicU32,
    epoch: AtomicU64,
    target_ms: AtomicU32,
    error: Mutex<Option<String>>,
    last_pcm: Mutex<Instant>,
}

impl DecodeState {
    fn new(start_ms: u32) -> Self {
        Self {
            phase: AtomicU8::new(PHASE_START),
            channels: AtomicU16::new(0),
            sample_rate: AtomicU32::new(0),
            epoch: AtomicU64::new(0),
            target_ms: AtomicU32::new(start_ms),
            error: Mutex::new(None),
            last_pcm: Mutex::new(Instant::now()),
        }
    }

    fn note_pcm(&self) {
        *self.last_pcm.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
    }

    fn starved(&self, timeout: Duration) -> bool {
        matches!(self.phase.load(Ordering::SeqCst), PHASE_START | PHASE_RUN)
            && self
                .last_pcm
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .elapsed()
                > timeout
    }

    fn set_format(&self, channels: u16, sample_rate: u32) {
        self.channels.store(channels.max(1), Ordering::SeqCst);
        self.sample_rate.store(sample_rate.max(1), Ordering::SeqCst);
        self.note_pcm();
        let phase = self.phase.load(Ordering::SeqCst);
        if phase == PHASE_START || phase == PHASE_ENDED {
            self.phase.store(PHASE_RUN, Ordering::SeqCst);
        }
    }

    fn resume_run(&self) {
        let phase = self.phase.load(Ordering::SeqCst);
        if phase == PHASE_START || phase == PHASE_ENDED {
            self.phase.store(PHASE_RUN, Ordering::SeqCst);
        }
    }

    fn end(&self) {
        let _ =
            self.phase
                .compare_exchange(PHASE_RUN, PHASE_ENDED, Ordering::SeqCst, Ordering::SeqCst);
        let _ = self.phase.compare_exchange(
            PHASE_START,
            PHASE_ENDED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    fn fail(&self, message: impl Into<String>) {
        let current = self.phase.load(Ordering::SeqCst);
        if current == PHASE_CANCEL || current == PHASE_ENDED {
            return;
        }
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message.into());
        self.phase.store(PHASE_FAILED, Ordering::SeqCst);
    }

    fn cancel(&self) {
        self.phase.store(PHASE_CANCEL, Ordering::SeqCst);
    }

    fn stopped(&self) -> bool {
        matches!(
            self.phase.load(Ordering::SeqCst),
            PHASE_CANCEL | PHASE_FAILED | PHASE_ENDED
        )
    }

    fn error(&self) -> Option<String> {
        if self.phase.load(Ordering::SeqCst) == PHASE_FAILED {
            self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeStatus {
    Starting,
    Running,
    Ended,
    Failed,
    Cancelled,
}

pub struct DecodeHandle {
    state: Arc<DecodeState>,
    cmd: Sender<DecodeCmd>,
    audio: SharedAudio,
}

impl DecodeHandle {
    pub fn seek(&self, ms: u32) {
        self.state.target_ms.store(ms, Ordering::SeqCst);
        self.state.epoch.fetch_add(1, Ordering::SeqCst);
        let _ = self.cmd.send(DecodeCmd::Seek);
        self.audio.abandon_reads();
    }

    pub fn stop(&self) {
        self.state.cancel();
        let _ = self.cmd.send(DecodeCmd::Stop);
        self.audio.abandon_reads();
    }

    pub fn format(&self) -> Option<(u16, u32)> {
        let channels = self.state.channels.load(Ordering::SeqCst);
        let rate = self.state.sample_rate.load(Ordering::SeqCst);
        if channels == 0 || rate == 0 {
            None
        } else {
            Some((channels, rate))
        }
    }

    pub fn status(&self) -> DecodeStatus {
        match self.state.phase.load(Ordering::SeqCst) {
            PHASE_RUN => DecodeStatus::Running,
            PHASE_ENDED => DecodeStatus::Ended,
            PHASE_FAILED => DecodeStatus::Failed,
            PHASE_CANCEL => DecodeStatus::Cancelled,
            _ => DecodeStatus::Starting,
        }
    }

    pub fn error(&self) -> Option<String> {
        self.state.error()
    }

    pub fn starved(&self, timeout: Duration) -> bool {
        self.state.starved(timeout)
    }

    #[cfg(test)]
    pub fn epoch(&self) -> u64 {
        self.state.epoch.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn start_target_ms(&self) -> u32 {
        self.state.target_ms.load(Ordering::SeqCst)
    }
}

impl Drop for DecodeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy, Debug)]
enum DecodeCmd {
    Seek,
    Stop,
}

struct PcmChunk {
    epoch: u64,
    samples: Vec<f32>,
}

pub struct PcmSource {
    rx: Receiver<PcmChunk>,
    cmd: Sender<DecodeCmd>,
    state: Arc<DecodeState>,
    audio: SharedAudio,
    buf: Vec<f32>,
    buf_epoch: u64,
    idx: usize,
}

impl PcmSource {
    fn pull(&mut self) -> Option<f32> {
        loop {
            let epoch = self.state.epoch.load(Ordering::SeqCst);
            if self.idx < self.buf.len() {
                if self.buf_epoch != epoch {
                    self.buf.clear();
                    self.idx = 0;
                    continue;
                }
                let sample = self.buf[self.idx];
                self.idx += 1;
                return Some(sample);
            }
            match self.rx.try_recv() {
                Ok(chunk) => {
                    if chunk.epoch != epoch {
                        continue;
                    }
                    self.buf = chunk.samples;
                    self.buf_epoch = chunk.epoch;
                    self.idx = 0;
                }
                Err(TryRecvError::Empty) => {
                    return match self.state.phase.load(Ordering::SeqCst) {
                        PHASE_ENDED | PHASE_CANCEL => None,
                        PHASE_FAILED => Some(0.0),
                        _ => Some(0.0),
                    };
                }
                Err(TryRecvError::Disconnected) => {
                    return match self.state.phase.load(Ordering::SeqCst) {
                        PHASE_FAILED => Some(0.0),
                        _ => None,
                    };
                }
            }
        }
    }
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.pull()
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.state.channels.load(Ordering::SeqCst).max(1)
    }

    fn sample_rate(&self) -> u32 {
        self.state.sample_rate.load(Ordering::SeqCst).max(1)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, pos: Duration) -> std::result::Result<(), SeekError> {
        let ms = pos.as_millis().min(u128::from(u32::MAX)) as u32;
        self.state.target_ms.store(ms, Ordering::SeqCst);
        self.state.epoch.fetch_add(1, Ordering::SeqCst);
        self.buf.clear();
        self.idx = 0;
        let _ = self.cmd.send(DecodeCmd::Seek);
        self.audio.abandon_reads();
        Ok(())
    }
}

pub fn spawn_decoder(
    buffer: SharedAudio,
    hint: FormatHint,
    start_ms: u32,
) -> Result<(PcmSource, DecodeHandle), String> {
    let state = Arc::new(DecodeState::new(start_ms));
    let (pcm_tx, pcm_rx) = mpsc::sync_channel(CHUNK_QUEUE);
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let thread_state = Arc::clone(&state);
    let thread_buffer = buffer.clone();
    thread::Builder::new()
        .name("fastpotify-decode".into())
        .spawn(move || decode_loop(thread_buffer, hint, start_ms, pcm_tx, cmd_rx, thread_state))
        .map_err(|error| error.to_string())?;
    Ok((
        PcmSource {
            rx: pcm_rx,
            cmd: cmd_tx.clone(),
            state: Arc::clone(&state),
            audio: buffer.clone(),
            buf: Vec::new(),
            buf_epoch: 0,
            idx: 0,
        },
        DecodeHandle {
            state,
            cmd: cmd_tx,
            audio: buffer,
        },
    ))
}

fn decode_loop(
    buffer: SharedAudio,
    hint: FormatHint,
    _start_ms: u32,
    pcm_tx: SyncSender<PcmChunk>,
    cmd_rx: Receiver<DecodeCmd>,
    state: Arc<DecodeState>,
) {
    'rebuild: loop {
        if state.stopped() && state.phase.load(Ordering::SeqCst) != PHASE_RUN {
            return;
        }
        match take_cmd(&cmd_rx) {
            Some(DecodeCmd::Stop) => {
                state.cancel();
                return;
            }
            Some(DecodeCmd::Seek) | None => {}
        }
        let start_ms = state.target_ms.load(Ordering::SeqCst);
        if buffer.is_cancelled() {
            state.cancel();
            return;
        }
        if let Some(message) = buffer.error() {
            state.fail(message);
            return;
        }
        let reader = buffer.reader();
        let reader_epoch = reader.epoch();
        let closed = buffer.is_closed();
        let seekable = buffer.is_random_access() || closed;
        let byte_len = buffer.content_length().unwrap_or(buffer.len() as u64);
        let mut decoder = match open_decoder(reader, &hint, seekable, byte_len) {
            Ok(decoder) => decoder,
            Err(error) => {
                if buffer.is_cancelled() {
                    state.cancel();
                    return;
                }
                if let Some(message) = buffer.error() {
                    state.fail(message);
                    return;
                }
                match take_cmd(&cmd_rx) {
                    Some(DecodeCmd::Stop) => {
                        state.cancel();
                        return;
                    }
                    Some(DecodeCmd::Seek) => continue,
                    None => {}
                }
                if buffer.read_epoch() != reader_epoch {
                    continue;
                }
                if closed {
                    state.fail(error);
                    return;
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        let channels = decoder.channels();
        let rate = decoder.sample_rate();
        state.set_format(channels, rate);
        if start_ms > 0 {
            let seeked = seekable
                && native_seek_allowed(&hint)
                && decoder
                    .try_seek(Duration::from_millis(u64::from(start_ms)))
                    .is_ok();
            if !seeked && seekable && native_seek_allowed(&hint) {
                if buffer.read_epoch() != reader_epoch {
                    continue;
                }
                let end = buffer.content_length().unwrap_or(buffer.len() as u64);
                if !closed && buffer.first_hole(0, end).is_some() {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
            }
            if !seeked {
                match skip_ms(
                    &mut decoder,
                    start_ms,
                    rate,
                    channels,
                    &state,
                    &cmd_rx,
                    &buffer,
                    reader_epoch,
                ) {
                    Some(DecodeCmd::Stop) => {
                        state.cancel();
                        return;
                    }
                    Some(DecodeCmd::Seek) => continue,
                    None => {}
                }
            }
        }
        match pump_samples(
            &mut decoder,
            &pcm_tx,
            &cmd_rx,
            &state,
            &buffer,
            reader_epoch,
        ) {
            Pump::Stop => {
                state.cancel();
                return;
            }
            Pump::Disconnected => return,
            Pump::Seek => continue 'rebuild,
            Pump::Ended { samples } => {
                if state.stopped() {
                    return;
                }
                match take_cmd(&cmd_rx) {
                    Some(DecodeCmd::Stop) => {
                        state.cancel();
                        return;
                    }
                    Some(DecodeCmd::Seek) => continue 'rebuild,
                    None => {}
                }
                if buffer.read_epoch() != reader_epoch {
                    continue 'rebuild;
                }
                match wait_out_eof(&buffer, &state, &cmd_rx, samples, start_ms, rate, channels) {
                    EofNext::Stop | EofNext::Done => return,
                    EofNext::Seek => continue 'rebuild,
                    EofNext::Rebuild { start_ms: next } => {
                        state.target_ms.store(next, Ordering::SeqCst);
                        continue 'rebuild;
                    }
                }
            }
        }
    }
}

enum EofNext {
    Stop,
    Done,
    Seek,
    Rebuild { start_ms: u32 },
}

fn wait_out_eof(
    buffer: &SharedAudio,
    state: &Arc<DecodeState>,
    cmd_rx: &Receiver<DecodeCmd>,
    samples: u64,
    start_ms: u32,
    rate: u32,
    channels: u16,
) -> EofNext {
    if buffer.is_cancelled() {
        state.cancel();
        return EofNext::Stop;
    }
    if let Some(message) = buffer.error() {
        state.fail(message);
        return EofNext::Stop;
    }
    if let Some(cmd) = take_cmd(cmd_rx) {
        match cmd {
            DecodeCmd::Stop => {
                state.cancel();
                return EofNext::Stop;
            }
            DecodeCmd::Seek => return EofNext::Seek,
        }
    }
    if buffer.is_closed() || buffer.is_random_access() {
        state.end();
        loop {
            if buffer.is_cancelled() {
                state.cancel();
                return EofNext::Stop;
            }
            match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(DecodeCmd::Stop) => {
                    state.cancel();
                    return EofNext::Stop;
                }
                Ok(DecodeCmd::Seek) => {
                    state.resume_run();
                    return EofNext::Seek;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if state.phase.load(Ordering::SeqCst) == PHASE_CANCEL {
                        return EofNext::Stop;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return EofNext::Stop,
            }
        }
    }
    let mut seen = buffer.len();
    loop {
        if let Some(cmd) = take_cmd(cmd_rx) {
            match cmd {
                DecodeCmd::Stop => {
                    state.cancel();
                    return EofNext::Stop;
                }
                DecodeCmd::Seek => return EofNext::Seek,
            }
        }
        match buffer.wait_for_change(seen, Duration::from_millis(50)) {
            BufferWait::Cancelled => {
                state.cancel();
                return EofNext::Stop;
            }
            BufferWait::Failed => {
                state.fail(
                    buffer
                        .error()
                        .unwrap_or_else(|| "Couldn't read matched audio.".into()),
                );
                return EofNext::Stop;
            }
            BufferWait::Closed(_) => {
                state.end();
                return EofNext::Done;
            }
            BufferWait::Grew(len) if len > seen => {
                return EofNext::Rebuild {
                    start_ms: start_ms.saturating_add(position_ms(samples, rate, channels)),
                };
            }
            BufferWait::Grew(_) | BufferWait::Unchanged(_) => {
                seen = buffer.len();
            }
        }
    }
}

fn position_ms(samples: u64, rate: u32, channels: u16) -> u32 {
    let denom = u64::from(rate.max(1)).saturating_mul(u64::from(channels.max(1)));
    if denom == 0 {
        return 0;
    }
    (samples.saturating_mul(1000) / denom).min(u64::from(u32::MAX)) as u32
}

enum Pump {
    Stop,
    Seek,
    Ended { samples: u64 },
    Disconnected,
}

fn take_cmd(cmd_rx: &Receiver<DecodeCmd>) -> Option<DecodeCmd> {
    cmd_rx.try_recv().ok()
}

#[allow(clippy::too_many_arguments)]
fn skip_ms<D: Iterator<Item = f32>>(
    decoder: &mut D,
    start_ms: u32,
    rate: u32,
    channels: u16,
    state: &DecodeState,
    cmd_rx: &Receiver<DecodeCmd>,
    buffer: &SharedAudio,
    reader_epoch: u64,
) -> Option<DecodeCmd> {
    let frames = u64::from(start_ms) * u64::from(rate) / 1000;
    let skip = frames.saturating_mul(u64::from(channels.max(1)));
    for i in 0..skip {
        if i % 1024 == 0 {
            if state.stopped() {
                return Some(DecodeCmd::Stop);
            }
            if let Some(cmd) = take_cmd(cmd_rx) {
                return Some(cmd);
            }
        }
        match decoder.next() {
            Some(_) => {}
            None => {
                if let Some(cmd) = take_cmd(cmd_rx) {
                    return Some(cmd);
                }
                if buffer.read_epoch() != reader_epoch {
                    return Some(DecodeCmd::Seek);
                }
                return None;
            }
        }
    }
    None
}

fn pump_samples<D: Source>(
    decoder: &mut D,
    pcm_tx: &SyncSender<PcmChunk>,
    cmd_rx: &Receiver<DecodeCmd>,
    state: &Arc<DecodeState>,
    buffer: &SharedAudio,
    reader_epoch: u64,
) -> Pump {
    let mut chunk = Vec::with_capacity(CHUNK_SAMPLES);
    let mut samples = 0u64;
    let produce_epoch = state.epoch.load(Ordering::SeqCst);
    loop {
        if let Some(cmd) = take_cmd(cmd_rx) {
            match cmd {
                DecodeCmd::Stop => return Pump::Stop,
                DecodeCmd::Seek => {
                    chunk.clear();
                    return Pump::Seek;
                }
            }
        }
        if state.phase.load(Ordering::SeqCst) == PHASE_CANCEL || buffer.is_cancelled() {
            return Pump::Stop;
        }
        match decoder.next() {
            Some(sample) => {
                samples += 1;
                chunk.push(sample);
                if chunk.len() >= CHUNK_SAMPLES
                    && let Err(pump) = send_chunk(pcm_tx, cmd_rx, state, &mut chunk, produce_epoch)
                {
                    return pump;
                }
            }
            None => {
                if buffer.read_epoch() != reader_epoch {
                    return Pump::Seek;
                }
                if !chunk.is_empty()
                    && let Err(pump) = send_chunk(pcm_tx, cmd_rx, state, &mut chunk, produce_epoch)
                {
                    return pump;
                }
                return Pump::Ended { samples };
            }
        }
    }
}

fn native_seek_allowed(hint: &FormatHint) -> bool {
    !matches!(hint.extension.as_deref(), Some("mp3" | "mpeg" | "mpga"))
}

fn send_chunk(
    pcm_tx: &SyncSender<PcmChunk>,
    cmd_rx: &Receiver<DecodeCmd>,
    state: &Arc<DecodeState>,
    chunk: &mut Vec<f32>,
    produce_epoch: u64,
) -> Result<(), Pump> {
    let samples = std::mem::take(chunk);
    *chunk = Vec::with_capacity(CHUNK_SAMPLES);
    let mut pending = PcmChunk {
        epoch: produce_epoch,
        samples,
    };
    loop {
        if state.phase.load(Ordering::SeqCst) == PHASE_CANCEL {
            return Err(Pump::Stop);
        }
        if let Some(cmd) = take_cmd(cmd_rx) {
            return Err(match cmd {
                DecodeCmd::Stop => Pump::Stop,
                DecodeCmd::Seek => Pump::Seek,
            });
        }
        match pcm_tx.try_send(pending) {
            Ok(()) => {
                state.note_pcm();
                return Ok(());
            }
            Err(mpsc::TrySendError::Full(returned)) => {
                pending = returned;
                thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(Pump::Disconnected),
        }
    }
}

fn open_decoder<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    hint: &FormatHint,
    seekable: bool,
    byte_len: u64,
) -> Result<rodio::Decoder<R>, String> {
    let mut builder = rodio::Decoder::builder()
        .with_data(reader)
        .with_seekable(false);
    if let Some(ext) = &hint.extension {
        builder = builder.with_hint(ext);
    }
    if let Some(mime) = &hint.mime {
        builder = builder.with_mime_type(mime);
    }
    if seekable && byte_len > 0 {
        builder = builder
            .with_byte_len(byte_len)
            .with_seekable(true)
            .with_coarse_seek(true);
    }
    builder.build().map_err(|error| error.to_string())
}

#[cfg(test)]
pub fn wait_nonzero_sample(source: &mut PcmSource, timeout: Duration) -> bool {
    wait_matching_sample(source, |sample| sample != 0.0, timeout).is_some()
}

#[cfg(test)]
pub fn wait_matching_sample(
    source: &mut PcmSource,
    mut pred: impl FnMut(f32) -> bool,
    timeout: Duration,
) -> Option<f32> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(sample) = source.next()
            && pred(sample)
        {
            return Some(sample);
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alternate::buffer::SharedAudio;

    fn tiny_wav(samples: usize) -> Vec<u8> {
        let sample_rate: u32 = 8_000;
        let data_bytes = (samples * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..samples {
            let sample: i16 = if i % 2 == 0 { 800 } else { -800 };
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn pcm_available_before_writer_closes() {
        let wav = tiny_wav(8_000);
        let audio = SharedAudio::new(None).unwrap();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        for chunk in wav.chunks(1024) {
            audio.append(chunk).unwrap();
        }
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(3)),
            "expected PCM before close"
        );
        assert!(!audio.is_closed());
        audio.close();
        assert!(handle.format().is_some());
        handle.stop();
    }

    #[test]
    fn cancel_stops_decoder() {
        let wav = tiny_wav(8_000);
        let audio = SharedAudio::new(None).unwrap();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        audio.append(&wav).unwrap();
        assert!(wait_nonzero_sample(&mut pcm, Duration::from_secs(3)));
        audio.cancel();
        handle.stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut ended = false;
        while std::time::Instant::now() < deadline {
            if matches!(
                handle.status(),
                DecodeStatus::Cancelled | DecodeStatus::Ended
            ) {
                ended = true;
                break;
            }
            let _ = pcm.next();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ended);
    }

    fn count_nonzero(pcm: &mut PcmSource, duration: Duration) -> usize {
        let deadline = std::time::Instant::now() + duration;
        let mut count = 0usize;
        while std::time::Instant::now() < deadline {
            match pcm.next() {
                Some(sample) if sample.abs() > 0.001 => count += 1,
                Some(_) | None => {}
            }
        }
        count
    }

    #[test]
    fn eof_while_open_does_not_replay_from_start() {
        let wav = tiny_wav(4_000);
        let audio = SharedAudio::new(None).unwrap();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        for chunk in wav.chunks(512) {
            audio.append(chunk).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        let first = count_nonzero(&mut pcm, Duration::from_millis(400));
        assert!(first > 100, "expected decoded PCM, got {first}");
        let extra = count_nonzero(&mut pcm, Duration::from_millis(300));
        assert!(
            extra < first,
            "replayed PCM while buffer still open: first={first} extra={extra}"
        );
        assert!(!audio.is_closed());
        audio.close();
        handle.stop();
    }

    #[test]
    fn closed_buffer_seek_keeps_producing_pcm() {
        let wav = tiny_wav(8_000);
        let audio = SharedAudio::new(Some(wav.len() as u64)).unwrap();
        audio.append(&wav).unwrap();
        audio.close();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        assert!(wait_nonzero_sample(&mut pcm, Duration::from_secs(3)));
        handle.seek(0);
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(2)),
            "seek on closed buffer should keep producing PCM"
        );
        assert_ne!(handle.status(), DecodeStatus::Failed);
        handle.stop();
    }

    #[test]
    fn sparse_seek_uses_native_seek_without_filling_the_middle() {
        let wav = tiny_wav(200_000);
        let total = wav.len() as u64;
        let audio = SharedAudio::with_limit(Some(total), total as usize).unwrap();
        audio.enable_random_access(total).unwrap();
        let prefix = 280_000usize.min(wav.len());
        audio.write_at(0, &wav[..prefix]).unwrap();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        assert!(wait_nonzero_sample(&mut pcm, Duration::from_secs(3)));
        handle.seek(20_000);
        let later = 44 + 160_000 * 2;
        let later_len = 8_192;
        assert!(later + later_len < wav.len());
        let demand_deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < demand_deadline {
            if audio
                .current_demand()
                .is_some_and(|d| d.start >= later as u64 - 32_768)
            {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let write_at = audio
            .current_demand()
            .map(|d| d.start)
            .filter(|start| *start >= later as u64 - 32_768)
            .unwrap_or(later as u64);
        audio
            .write_at(
                write_at,
                &wav[write_at as usize..(write_at as usize + later_len).min(wav.len())],
            )
            .unwrap();
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(3)),
            "seek into a hole should resume after those bytes arrive; demand={:?} status={:?}",
            audio.current_demand(),
            handle.status()
        );
        assert!(
            !audio.is_range_filled(prefix as u64 + 8, later as u64),
            "middle should stay a hole: {:?}",
            audio.filled_intervals()
        );
        assert_ne!(handle.status(), DecodeStatus::Failed);
        handle.stop();
    }

    fn marked_wav(samples: usize, mark_at: usize) -> Vec<u8> {
        let sample_rate: u32 = 8_000;
        let data_bytes = (samples * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..samples {
            let sample: i16 = if i >= mark_at { -20_000 } else { 20_000 };
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn handle_seek_drops_stale_pcm_buffer() {
        let samples = 8_000;
        let mark_at = 1_000;
        let wav = marked_wav(samples, mark_at);
        let audio = SharedAudio::new(Some(wav.len() as u64)).unwrap();
        audio.append(&wav).unwrap();
        audio.close();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        assert!(
            wait_matching_sample(&mut pcm, |s| s > 0.5, Duration::from_secs(3)).is_some(),
            "expected pre-seek positive marker"
        );
        let mut leftover = 0usize;
        let drain_by = std::time::Instant::now() + Duration::from_millis(400);
        while leftover < 32 && std::time::Instant::now() < drain_by {
            if let Some(sample) = pcm.next()
                && sample > 0.5
            {
                leftover += 1;
            }
        }
        assert!(
            leftover >= 32,
            "need leftover pre-seek PCM in the source buf, got {leftover}"
        );
        handle.seek(500);
        let sample = wait_matching_sample(&mut pcm, |s| s.abs() > 0.5, Duration::from_secs(3));
        assert!(
            sample.is_some_and(|s| s < -0.5),
            "expected post-seek negative marker as first significant sample, got {sample:?}"
        );
        handle.stop();
    }

    #[test]
    fn stall_at_frontier_then_seek_retargets_quickly() {
        let samples = 200_000;
        let mark_at = 100_000;
        let wav = marked_wav(samples, mark_at);
        let total = wav.len() as u64;
        let audio = SharedAudio::with_limit(Some(total), total as usize).unwrap();
        audio.enable_random_access(total).unwrap();
        let prefix = 44 + 4_096 * 2;
        audio.write_at(0, &wav[..prefix]).unwrap();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        assert!(wait_nonzero_sample(&mut pcm, Duration::from_secs(3)));
        thread::sleep(Duration::from_millis(80));
        let frontier_demand = audio.current_demand().map(|d| d.start);
        let target_byte = (44 + mark_at * 2) as u64;
        let start = std::time::Instant::now();
        handle.seek(20_000);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut demand = audio.current_demand();
        while std::time::Instant::now() < deadline {
            demand = audio.current_demand();
            if demand.is_some_and(|d| d.start >= target_byte.saturating_sub(32_768)) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "retarget stalled {elapsed:?}; frontier={frontier_demand:?} demand={demand:?}"
        );
        assert!(
            demand.is_some_and(|d| d.start >= target_byte.saturating_sub(32_768)),
            "decoder stayed on old frontier {frontier_demand:?}, demand={demand:?}, target={target_byte}"
        );
        let write_at = demand.map(|d| d.start).unwrap_or(target_byte);
        let take = 8_192.min(wav.len().saturating_sub(write_at as usize));
        audio
            .write_at(write_at, &wav[write_at as usize..write_at as usize + take])
            .unwrap();
        let sample = wait_matching_sample(&mut pcm, |s| s.abs() > 0.5, Duration::from_secs(3));
        assert!(
            sample.is_some_and(|s| s < -0.5),
            "expected PCM from the new seek position, got {sample:?} status={:?} demand={demand:?}",
            handle.status()
        );
        handle.stop();
    }

    fn count_nonzero_until(pcm: &mut PcmSource, duration: Duration) -> usize {
        let deadline = Instant::now() + duration;
        let mut count = 0usize;
        while Instant::now() < deadline {
            if pcm.next().is_some_and(|sample| sample.abs() > 0.0001) {
                count += 1;
            }
        }
        count
    }

    fn play_compressed_from_zero(bytes: &[u8], ext: &str, mime: &str) {
        let audio = SharedAudio::new(Some(bytes.len() as u64)).unwrap();
        audio.append(bytes).unwrap();
        let hint = FormatHint::from_labels(Some(ext), Some(mime), None);
        let (mut pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(5)),
            "{ext} produced no PCM from 0, status={:?}",
            handle.status()
        );
        let extra = count_nonzero_until(&mut pcm, Duration::from_millis(350));
        assert!(
            extra > 80,
            "{ext} did not keep producing nonzero PCM, got {extra} status={:?}",
            handle.status()
        );
        assert_ne!(handle.status(), DecodeStatus::Failed);
        handle.stop();
    }

    #[test]
    fn mp3_continuous_nonzero_from_zero() {
        play_compressed_from_zero(TONE_MP3, "mp3", "audio/mpeg");
    }

    #[test]
    fn mp3_seek_on_complete_buffer_keeps_pcm() {
        let audio = SharedAudio::new(Some(TONE_MP3.len() as u64)).unwrap();
        audio.append(TONE_MP3).unwrap();
        audio.close();
        let hint = FormatHint::from_labels(Some("mp3"), Some("audio/mpeg"), None);
        let (mut pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(5)),
            "mp3 produced no PCM from 0, status={:?}",
            handle.status()
        );
        handle.seek(250);
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(5)),
            "mp3 produced no PCM after seek, status={:?}",
            handle.status()
        );
        assert_ne!(handle.status(), DecodeStatus::Failed);
        handle.stop();
    }

    #[test]
    fn m4a_faststart_continuous_nonzero_from_zero() {
        play_compressed_from_zero(TONE_M4A, "m4a", "audio/mp4");
    }

    #[test]
    fn blocked_decoder_is_starved_not_healthy() {
        let audio = SharedAudio::new(None).unwrap();
        let hint = FormatHint::from_labels(Some("mp3"), Some("audio/mpeg"), None);
        let (_pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        thread::sleep(Duration::from_millis(40));
        assert!(
            handle.starved(Duration::from_millis(10)),
            "a decoder with no bytes must not look healthy"
        );
        assert_ne!(handle.status(), DecodeStatus::Failed);
        handle.stop();
    }
}
