//! Alternate local engine: resolve, fetch, decode, and drive the session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::AlternateConfig;
use super::audio::{AudioOutput, OutputStatus, RodioOutput, volume_f32};
use super::buffer::SharedAudio;
use super::decode::{DecodeHandle, FormatHint, PcmSource, spawn_decoder};
use super::fetch::{self, FetchPolicy};
use super::hydrate::{expand_load, offset_index, seed_queue};
use super::matching::{TrackQuery, rank_candidates};
use super::provider::{MediaLookup, Resolver};
use super::session::{Advance, Session};
use super::streams::select_audio_stream;
use crate::api::ApiClient;
use crate::player::{EngineEvent, LoadSpec, LocalTrack, Notify, Playback, PlayerCommand};
use crate::util::uri_kind;

const TICK: Duration = Duration::from_millis(50);
const MAX_MISS_SKIPS: u32 = 16;
const SEEK_MATERIAL_MS: u32 = 50;
const DEVICE_BACKOFF_INITIAL: Duration = Duration::from_millis(200);
const DEVICE_BACKOFF_CAP: Duration = Duration::from_secs(8);

enum Internal {
    Command(PlayerCommand),
    Job(Job),
    Shutdown,
    #[cfg(test)]
    TestLoad {
        tracks: Vec<LocalTrack>,
        play: bool,
    },
}

enum Job {
    Hydrated {
        token: u64,
        spec: LoadSpec,
        result: Result<Vec<LocalTrack>, String>,
    },
    Canned {
        token: u64,
        uri: String,
        bytes: Vec<u8>,
        label: String,
        video_id: String,
    },
    MatchFailed {
        token: u64,
        uri: String,
        error: String,
    },
    Ready {
        token: u64,
        uri: String,
        buffer: SharedAudio,
        label: String,
        video_id: String,
        hint: FormatHint,
    },
    TransportFailed {
        token: u64,
        uri: String,
        error: String,
    },
}

struct PendingPlay {
    pcm: Option<PcmSource>,
    decode: Option<DecodeHandle>,
    label: String,
    start_ms: u32,
}

struct CachedMatch {
    video_id: String,
}

pub struct AlternateHandle {
    tx: mpsc::UnboundedSender<Internal>,
    cancel: watch::Sender<bool>,
    join: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl AlternateHandle {
    pub fn command(&self, command: PlayerCommand) -> Result<()> {
        self.tx
            .send(Internal::Command(command))
            .map_err(|_| anyhow!("alternate engine is not running"))
    }

    pub async fn shutdown(&self) {
        let _ = self.cancel.send(true);
        let _ = self.tx.send(Internal::Shutdown);
        let join = self.join.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(join) = join {
            join.abort();
            let _ = join.await;
        }
    }

    #[cfg(test)]
    fn test_load(&self, tracks: Vec<LocalTrack>, play: bool) {
        let _ = self.tx.send(Internal::TestLoad { tracks, play });
    }
}

pub fn spawn(
    config: AlternateConfig,
    api: Arc<ApiClient>,
    http: reqwest::Client,
    notify: Notify,
    output: Option<Box<dyn AudioOutput>>,
    ytdlp_dir: PathBuf,
) -> Result<AlternateHandle, String> {
    config.validate()?;
    let lookup: Arc<dyn MediaLookup> = Arc::new(Resolver::from_config(&config, &ytdlp_dir)?);
    let output = match output {
        Some(output) => output,
        None => Box::new(RodioOutput::open().map_err(|error| error.to_string())?),
    };
    Ok(spawn_inner(config, api, http, notify, output, lookup))
}

fn spawn_inner(
    config: AlternateConfig,
    api: Arc<ApiClient>,
    http: reqwest::Client,
    notify: Notify,
    output: Box<dyn AudioOutput>,
    lookup: Arc<dyn MediaLookup>,
) -> AlternateHandle {
    let media_http = reqwest::Client::builder()
        .user_agent(concat!("fastpotify/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| http.clone());
    let (tx, rx) = mpsc::unbounded_channel();
    let (cancel, cancel_rx) = watch::channel(false);
    let join = tokio::spawn(run(
        config,
        api,
        media_http,
        notify,
        output,
        lookup,
        tx.clone(),
        rx,
        cancel_rx,
    ));
    AlternateHandle {
        tx,
        cancel,
        join: std::sync::Mutex::new(Some(join)),
    }
}

struct Engine {
    config: AlternateConfig,
    api: Arc<ApiClient>,
    media_http: reqwest::Client,
    notify: Notify,
    output: Box<dyn AudioOutput>,
    lookup: Arc<dyn MediaLookup>,
    tx: mpsc::UnboundedSender<Internal>,
    cancel_rx: watch::Receiver<bool>,
    session: Session,
    matches: HashMap<String, CachedMatch>,
    play_generation: u64,
    jobs: Vec<JoinHandle<()>>,
    active_buffer: Option<SharedAudio>,
    active_hint: Option<FormatHint>,
    pending: Option<PendingPlay>,
    miss_skips: u32,
    seeded_play: bool,
    device_retry_at: Option<Instant>,
    device_backoff: Duration,
}

#[allow(clippy::too_many_arguments)]
async fn run(
    config: AlternateConfig,
    api: Arc<ApiClient>,
    media_http: reqwest::Client,
    notify: Notify,
    output: Box<dyn AudioOutput>,
    lookup: Arc<dyn MediaLookup>,
    tx: mpsc::UnboundedSender<Internal>,
    mut commands: mpsc::UnboundedReceiver<Internal>,
    cancel_rx: watch::Receiver<bool>,
) {
    let volume = config.volume;
    let mut engine = Engine {
        config,
        api,
        media_http,
        notify,
        output,
        lookup,
        tx,
        cancel_rx: cancel_rx.clone(),
        session: Session::new(volume),
        matches: HashMap::new(),
        play_generation: 0,
        jobs: Vec::new(),
        active_buffer: None,
        active_hint: None,
        pending: None,
        miss_skips: 0,
        seeded_play: false,
        device_retry_at: None,
        device_backoff: DEVICE_BACKOFF_INITIAL,
    };
    engine.output.set_volume(volume_f32(volume));
    engine.emit();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cancel_rx = cancel_rx;

    loop {
        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    break;
                }
            }
            command = commands.recv() => {
                match command {
                    None | Some(Internal::Shutdown) => break,
                    Some(Internal::Command(command)) => engine.handle_command(command),
                    Some(Internal::Job(job)) => engine.handle_job(job),
                    #[cfg(test)]
                    Some(Internal::TestLoad { tracks, play }) => engine.test_load(tracks, play),
                }
            }
            _ = tick.tick() => {
                engine.try_start_pending();
                engine.poll_output();
            }
        }
        if *cancel_rx.borrow() {
            break;
        }
    }
    engine.abort_jobs();
    engine.output.stop();
}

impl Engine {
    fn cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    fn emit(&self) {
        if self.cancelled() {
            return;
        }
        (self.notify)(EngineEvent::State(self.session.snapshot()));
    }

    fn abort_jobs(&mut self) {
        for job in self.jobs.drain(..) {
            job.abort();
        }
    }

    fn bump(&mut self) -> u64 {
        self.seeded_play = false;
        self.abort_jobs();
        self.active_hint = None;
        self.device_retry_at = None;
        self.device_backoff = DEVICE_BACKOFF_INITIAL;
        if let Some(buffer) = self.active_buffer.take() {
            buffer.cancel();
        }
        if let Some(pending) = self.pending.take()
            && let Some(decode) = pending.decode
        {
            decode.stop();
        }
        self.play_generation = self.play_generation.wrapping_add(1);
        self.play_generation
    }

    fn handle_command(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Toggle => match self.session.toggle() {
                Advance::Stay => {
                    match self.session.playback() {
                        Playback::Paused => self.output.pause(),
                        Playback::Playing => self.output.resume(),
                        _ => {}
                    }
                    self.emit();
                }
                Advance::CancelLoad => {
                    self.bump();
                    self.output.stop();
                    self.emit();
                }
                other => self.follow(other),
            },
            PlayerCommand::Next => {
                self.bump();
                let advance = self.session.skip_forward();
                self.follow(advance);
            }
            PlayerCommand::Previous => {
                self.bump();
                let advance = self.session.previous();
                self.follow(advance);
            }
            PlayerCommand::Seek(position_ms) => {
                self.session.seek(position_ms);
                if let Some(pending) = &mut self.pending {
                    pending.start_ms = position_ms;
                    if let Some(decode) = &pending.decode {
                        decode.seek(position_ms);
                    }
                }
                let _ = self.output.seek(position_ms);
                self.emit();
            }
            PlayerCommand::Volume(volume) | PlayerCommand::VolumePreview(volume) => {
                self.session.set_volume(volume);
                self.output.set_volume(volume_f32(volume));
                self.emit();
            }
            PlayerCommand::Shuffle(enabled) => {
                self.session.set_shuffle(enabled);
                self.emit();
            }
            PlayerCommand::Repeat(mode) => {
                self.session.set_repeat(mode);
                self.emit();
            }
            PlayerCommand::Activate => self.emit(),
            PlayerCommand::Stop => {
                self.bump();
                self.session.stop();
                self.output.stop();
                self.emit();
            }
            PlayerCommand::AddToQueue(track) => {
                if track.is_episode || uri_kind(&track.uri) == Some("episode") {
                    self.session
                        .set_error("Podcasts are not supported in alternate playback.".into());
                    self.emit();
                    return;
                }
                self.session.add_to_queue(track);
                self.emit();
            }
            PlayerCommand::Load(spec) => self.start_hydrate(spec),
        }
    }

    fn follow(&mut self, advance: Advance) {
        match advance {
            Advance::Stay => self.emit(),
            Advance::SeekZero => {
                let _ = self.output.seek(0);
                self.emit();
            }
            Advance::Stop => {
                self.output.stop();
                self.emit();
            }
            Advance::PlayCurrent => self.start_resolve(),
            Advance::CancelLoad => {
                self.bump();
                self.output.stop();
                self.emit();
            }
        }
    }

    fn start_hydrate(&mut self, spec: LoadSpec) {
        let token = self.bump();
        let seed = seed_queue(&spec);
        if seed.is_empty() {
            self.session.set_loading();
            self.emit();
        } else {
            let offset = offset_index(&spec, &seed);
            self.session
                .load(seed, offset, spec.play, spec.shuffle, spec.position_ms);
            self.seeded_play = true;
            if spec.play {
                self.start_resolve();
            } else {
                self.output.stop();
                self.emit();
            }
        }
        let api = Arc::clone(&self.api);
        let tx = self.tx.clone();
        let mut cancel_rx = self.cancel_rx.clone();
        self.jobs.push(tokio::spawn(async move {
            let result = tokio::select! {
                _ = wait_cancel(&mut cancel_rx) => return,
                result = expand_load(&api, &spec) => result,
            };
            let _ = tx.send(Internal::Job(Job::Hydrated {
                token,
                spec,
                result: result.map_err(|error| error.to_string()),
            }));
        }));
    }

    fn start_resolve(&mut self) {
        let Some(track) = self.session.current().cloned() else {
            self.session.stop();
            self.output.stop();
            self.emit();
            return;
        };
        if track.is_episode {
            self.fail_or_skip("Podcasts are not supported in alternate playback.".into());
            return;
        }
        let token = self.play_generation;
        self.session.set_loading();
        self.emit();
        let lookup = Arc::clone(&self.lookup);
        let http = self.media_http.clone();
        let config = self.config.clone();
        let cached = self
            .matches
            .get(&track.uri)
            .map(|entry| entry.video_id.clone());
        let tx = self.tx.clone();
        let mut cancel_rx = self.cancel_rx.clone();
        self.jobs.push(tokio::spawn(async move {
            tokio::select! {
                _ = wait_cancel(&mut cancel_rx) => {}
                _ = resolve_and_stream(
                    &config,
                    lookup.as_ref(),
                    &http,
                    cached,
                    &track,
                    token,
                    &tx,
                ) => {}
            }
        }));
    }

    fn handle_job(&mut self, job: Job) {
        if self.cancelled() {
            return;
        }
        match job {
            Job::Hydrated {
                token,
                spec,
                result,
            } => {
                if token != self.play_generation {
                    return;
                }
                match result {
                    Ok(tracks) => {
                        self.miss_skips = 0;
                        let offset = offset_index(&spec, &tracks);
                        if self.seeded_play {
                            let prev = self.session.current().map(|track| track.uri.clone());
                            self.session.adopt_tracks(tracks, offset);
                            let now = self.session.current().map(|track| track.uri.clone());
                            if spec.play && now != prev {
                                let _ = self.bump();
                                self.start_resolve();
                            } else {
                                self.emit();
                            }
                        } else {
                            self.session.load(
                                tracks,
                                offset,
                                spec.play,
                                spec.shuffle,
                                spec.position_ms,
                            );
                            if spec.play {
                                self.start_resolve();
                            } else {
                                self.output.stop();
                                self.emit();
                            }
                        }
                    }
                    Err(error) => {
                        if self.seeded_play {
                            return;
                        }
                        self.session.set_error(error);
                        self.emit();
                    }
                }
            }
            Job::Canned {
                token,
                uri,
                bytes,
                label,
                video_id,
            } => {
                if !self.job_current(token, &uri) {
                    return;
                }
                self.cache_match(uri, video_id);
                let start = self.session.position_now();
                match self.output.play_bytes(bytes, start) {
                    Ok(_) => {
                        self.miss_skips = 0;
                        self.session.set_playing(Some(label));
                        self.emit();
                    }
                    Err(error) => self.fail_transport(format!("Couldn't decode audio: {error}")),
                }
            }
            Job::MatchFailed { token, uri, error } => {
                if !self.job_current(token, &uri) {
                    return;
                }
                self.fail_or_skip(error);
            }
            Job::Ready {
                token,
                uri,
                buffer,
                label,
                video_id,
                hint,
            } => {
                if !self.job_current(token, &uri) {
                    buffer.cancel();
                    return;
                }
                self.cache_match(uri, video_id);
                self.active_buffer = Some(buffer.clone());
                self.active_hint = Some(hint.clone());
                let start_ms = self.session.position_now();
                match spawn_decoder(buffer, hint, start_ms) {
                    Ok((pcm, decode)) => {
                        self.pending = Some(PendingPlay {
                            pcm: Some(pcm),
                            decode: Some(decode),
                            label,
                            start_ms,
                        });
                        self.try_start_pending();
                    }
                    Err(error) => self.fail_transport(format!("Couldn't decode audio: {error}")),
                }
            }
            Job::TransportFailed { token, uri, error } => {
                if !self.job_current(token, &uri) {
                    return;
                }
                self.fail_transport(error);
            }
        }
    }

    fn job_current(&self, token: u64, uri: &str) -> bool {
        token == self.play_generation
            && self.session.current().map(|track| track.uri.as_str()) == Some(uri)
    }

    fn cache_match(&mut self, uri: String, video_id: String) {
        if !video_id.is_empty() {
            self.matches.insert(uri, CachedMatch { video_id });
        }
    }

    fn try_start_pending(&mut self) {
        let decode_error = self
            .pending
            .as_ref()
            .and_then(|pending| pending.decode.as_ref())
            .and_then(DecodeHandle::error);
        if let Some(message) = decode_error {
            self.pending = None;
            self.fail_transport(format!("Couldn't decode audio: {message}"));
            return;
        }
        if matches!(self.output.status(), OutputStatus::DeviceLost) {
            return;
        }
        let ready = self
            .pending
            .as_ref()
            .and_then(|pending| pending.decode.as_ref())
            .is_some_and(|decode| decode.format().is_some());
        if !ready {
            return;
        }
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let (Some(pcm), Some(decode)) = (pending.pcm.take(), pending.decode.take()) else {
            return;
        };
        let now = self.session.position_now();
        if now.abs_diff(pending.start_ms) >= SEEK_MATERIAL_MS {
            decode.seek(now);
        }
        match self.output.play_pcm(pcm, decode) {
            Ok(_) => {
                self.miss_skips = 0;
                self.session.set_playing(Some(pending.label));
                self.emit();
            }
            Err(error) => self.fail_transport(format!("Couldn't start audio: {error}")),
        }
    }

    fn fail_or_skip(&mut self, message: String) {
        self.output.stop();
        self.session.set_error(message);
        self.emit();
        if self.config.skip_on_miss && self.miss_skips < MAX_MISS_SKIPS {
            self.miss_skips = self.miss_skips.saturating_add(1);
            match self.session.fail_next() {
                Advance::PlayCurrent => self.start_resolve(),
                other => self.follow(other),
            }
        }
    }

    fn fail_transport(&mut self, message: String) {
        self.bump();
        self.output.stop();
        self.session.set_error(message);
        self.emit();
    }

    fn poll_output(&mut self) {
        match self.output.status() {
            OutputStatus::DeviceLost => self.handle_device_lost(),
            OutputStatus::Buffering if self.session.playback() == Playback::Playing => {
                self.freeze_output_clock();
            }
            OutputStatus::Playing if self.session.playback() == Playback::Playing => {
                self.resume_output_clock();
            }
            OutputStatus::Ended if self.session.playback() == Playback::Playing => {
                let advance = self.session.on_ended();
                self.follow(advance);
            }
            OutputStatus::Failed(message) if self.session.playback() == Playback::Playing => {
                self.fail_transport(message);
            }
            OutputStatus::Playing
            | OutputStatus::Buffering
            | OutputStatus::Ended
            | OutputStatus::Failed(_) => {}
        }
    }

    fn freeze_output_clock(&mut self) {
        if self.session.clock_running() {
            self.session.freeze_clock();
            self.emit();
        }
    }

    fn resume_output_clock(&mut self) {
        if self.session.playback() == Playback::Playing && !self.session.clock_running() {
            self.session.resume_clock();
            self.emit();
        }
    }

    fn handle_device_lost(&mut self) {
        if self.session.playback() == Playback::Playing {
            self.freeze_output_clock();
        }
        if let Some(at) = self.device_retry_at
            && Instant::now() < at
        {
            return;
        }
        match self.output.recover() {
            Ok(()) => {
                self.device_retry_at = None;
                self.device_backoff = DEVICE_BACKOFF_INITIAL;
                if self.active_buffer.is_some()
                    && self.pending.is_none()
                    && let Err(error) = self.reattach_pcm()
                {
                    self.fail_transport(error);
                }
            }
            Err(_) => {
                self.device_retry_at = Some(Instant::now() + self.device_backoff);
                self.device_backoff = self
                    .device_backoff
                    .saturating_mul(2)
                    .min(DEVICE_BACKOFF_CAP);
            }
        }
    }

    fn reattach_pcm(&mut self) -> Result<(), String> {
        let buffer = self
            .active_buffer
            .clone()
            .ok_or_else(|| "Couldn't start audio.".to_string())?;
        let hint = self
            .active_hint
            .clone()
            .ok_or_else(|| "Couldn't start audio.".to_string())?;
        let pos = self.session.position_now();
        let paused = self.session.playback() == Playback::Paused;
        let (pcm, decode) = spawn_decoder(buffer, hint, pos)
            .map_err(|error| format!("Couldn't decode audio: {error}"))?;
        self.output
            .play_pcm(pcm, decode)
            .map_err(|error| format!("Couldn't start audio: {error}"))?;
        if paused {
            self.output.pause();
        }
        if self.session.playback() == Playback::Playing {
            self.session.resume_clock();
            self.emit();
        }
        Ok(())
    }

    #[cfg(test)]
    fn test_load(&mut self, tracks: Vec<LocalTrack>, play: bool) {
        self.bump();
        self.miss_skips = 0;
        self.session.load(tracks, 0, play, Some(false), 0);
        if play {
            self.start_resolve();
        } else {
            self.emit();
        }
    }
}

async fn wait_cancel(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_and_stream(
    config: &AlternateConfig,
    lookup: &dyn MediaLookup,
    http: &reqwest::Client,
    cached_id: Option<String>,
    track: &LocalTrack,
    token: u64,
    tx: &mpsc::UnboundedSender<Internal>,
) {
    let send = |job: Job| {
        let _ = tx.send(Internal::Job(job));
    };
    let query = TrackQuery {
        title: track.title.clone(),
        artists: track.artists.clone(),
        duration_ms: (track.duration_ms > 0).then_some(track.duration_ms),
    };
    let search_text = {
        let mut parts = query.artists.clone();
        parts.push(query.title.clone());
        parts.join(" ")
    };
    let (video_id, used_ytdlp) = if let Some(id) = cached_id {
        (id, false)
    } else {
        match lookup.search(&search_text).await {
            Ok((candidates, used_ytdlp)) => {
                match rank_candidates(&query, &candidates, config.min_score) {
                    Some(ranked) => (ranked.candidate.id, used_ytdlp),
                    None => {
                        send(Job::MatchFailed {
                            token,
                            uri: track.uri.clone(),
                            error: format!(
                                "No confident match for {} — {}",
                                track.title,
                                track.artist_names()
                            ),
                        });
                        return;
                    }
                }
            }
            Err(error) => {
                send(Job::TransportFailed {
                    token,
                    uri: track.uri.clone(),
                    error,
                });
                return;
            }
        }
    };
    if let Some(bytes) = lookup.canned_audio() {
        send(Job::Canned {
            token,
            uri: track.uri.clone(),
            bytes,
            label: "test match · not Spotify audio".into(),
            video_id,
        });
        return;
    }
    let (streams, stream_ytdlp) = match lookup.streams(&video_id).await {
        Ok(value) => value,
        Err(error) => {
            send(Job::TransportFailed {
                token,
                uri: track.uri.clone(),
                error,
            });
            return;
        }
    };
    let Some(stream) = select_audio_stream(&streams) else {
        send(Job::MatchFailed {
            token,
            uri: track.uri.clone(),
            error: "No playable audio stream (need AAC/M4A or MP3; Opus/WebM is not decoded)."
                .into(),
        });
        return;
    };
    let label = if used_ytdlp || stream_ytdlp {
        "yt-dlp match · not Spotify audio"
    } else {
        "Piped match · not Spotify audio"
    };
    let hint = FormatHint::from_labels(
        stream.format.as_deref(),
        stream.mime.as_deref(),
        Some(stream.url.as_str()),
    );
    let buffer = match SharedAudio::new(None) {
        Ok(buffer) => buffer,
        Err(error) => {
            send(Job::TransportFailed {
                token,
                uri: track.uri.clone(),
                error,
            });
            return;
        }
    };
    let mut ready_sent = false;
    let mut on_ready = {
        let buffer = buffer.clone();
        let uri = track.uri.clone();
        let label = label.to_string();
        let video_id = video_id.clone();
        let hint = hint.clone();
        let tx = tx.clone();
        move || {
            if ready_sent {
                return;
            }
            ready_sent = true;
            let _ = tx.send(Internal::Job(Job::Ready {
                token,
                uri: uri.clone(),
                buffer: buffer.clone(),
                label: label.clone(),
                video_id: video_id.clone(),
                hint: hint.clone(),
            }));
        }
    };
    let result = if let Some(script) = lookup.scripted_body() {
        fetch::fetch_scripted(&buffer, script, &mut on_ready).await
    } else {
        fetch::fetch_with(
            http,
            stream.url.clone(),
            stream.http_headers.clone(),
            &buffer,
            &mut on_ready,
            FetchPolicy::production(),
            Some(lookup),
            Some(video_id.as_str()),
            Some(&hint),
        )
        .await
    };
    match result {
        Ok(()) => on_ready(),
        Err(error) if error == "cancelled" => {}
        Err(error) => send(Job::TransportFailed {
            token,
            uri: track.uri.clone(),
            error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alternate::decode::{
        DecodeHandle, PcmSource, wait_matching_sample, wait_nonzero_sample,
    };
    use crate::alternate::provider::ScriptedBody;
    use crate::player::LocalTrack;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::oneshot;

    struct RecordingOutput {
        plays: Arc<AtomicUsize>,
        resumes: Arc<AtomicUsize>,
        pauses: Arc<AtomicUsize>,
    }

    impl AudioOutput for RecordingOutput {
        fn play_bytes(
            &mut self,
            _bytes: Vec<u8>,
            _start_ms: u32,
        ) -> Result<super::super::audio::PlayInfo> {
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn pause(&mut self) {
            self.pauses.fetch_add(1, Ordering::SeqCst);
        }
        fn resume(&mut self) {
            self.resumes.fetch_add(1, Ordering::SeqCst);
        }
        fn stop(&mut self) {}
        fn seek(&mut self, _ms: u32) -> Result<()> {
            Ok(())
        }
        fn set_volume(&mut self, _volume: f32) {}
        fn is_finished(&self) -> bool {
            false
        }
        fn play_pcm(
            &mut self,
            _source: super::super::decode::PcmSource,
            _decode: super::super::decode::DecodeHandle,
        ) -> Result<super::super::audio::PlayInfo> {
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn status(&self) -> super::super::audio::OutputStatus {
            super::super::audio::OutputStatus::Playing
        }
    }

    struct CaptureOutput {
        plays: Arc<AtomicUsize>,
        pcm: Arc<Mutex<Option<PcmSource>>>,
        decode: Arc<Mutex<Option<DecodeHandle>>>,
        start_ms: Arc<Mutex<Vec<u32>>>,
    }

    impl AudioOutput for CaptureOutput {
        fn play_bytes(
            &mut self,
            _bytes: Vec<u8>,
            start_ms: u32,
        ) -> Result<super::super::audio::PlayInfo> {
            self.start_ms
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(start_ms);
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn pause(&mut self) {}
        fn resume(&mut self) {}
        fn stop(&mut self) {}
        fn seek(&mut self, ms: u32) -> Result<()> {
            if let Some(decode) = self
                .decode
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                decode.seek(ms);
            }
            self.start_ms
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(ms);
            Ok(())
        }
        fn set_volume(&mut self, _volume: f32) {}
        fn is_finished(&self) -> bool {
            false
        }
        fn play_pcm(
            &mut self,
            source: PcmSource,
            decode: DecodeHandle,
        ) -> Result<super::super::audio::PlayInfo> {
            *self.pcm.lock().unwrap_or_else(|p| p.into_inner()) = Some(source);
            *self.decode.lock().unwrap_or_else(|p| p.into_inner()) = Some(decode);
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn status(&self) -> super::super::audio::OutputStatus {
            super::super::audio::OutputStatus::Playing
        }
    }

    struct HoldLookup {
        hold: Mutex<Option<oneshot::Receiver<()>>>,
        audio: Vec<u8>,
    }

    impl MediaLookup for HoldLookup {
        fn search(
            &self,
            _query: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::matching::Candidate>, bool), String>,
        > {
            let rx = self.hold.lock().unwrap_or_else(|p| p.into_inner()).take();
            Box::pin(async move {
                if let Some(rx) = rx {
                    let _ = rx.await;
                }
                Ok((
                    vec![super::super::matching::Candidate {
                        id: "dQw4w9WgXcQ".into(),
                        title: "Song".into(),
                        uploader: "Artist - Topic".into(),
                        duration_ms: Some(1_000),
                    }],
                    true,
                ))
            })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::streams::AudioStream>, bool), String>,
        > {
            Box::pin(async {
                Ok((
                    vec![super::super::streams::AudioStream {
                        url: "https://example.invalid/a.m4a".into(),
                        mime: Some("audio/mp4".into()),
                        codec: Some("mp4a.40.2".into()),
                        format: Some("m4a".into()),
                        bitrate: Some(128_000),
                        video_only: false,
                        quality: None,
                        http_headers: Vec::new(),
                    }],
                    true,
                ))
            })
        }

        fn canned_audio(&self) -> Option<Vec<u8>> {
            Some(self.audio.clone())
        }
    }

    fn track(title: &str) -> LocalTrack {
        LocalTrack {
            uri: format!("spotify:track:{title}"),
            title: title.into(),
            artists: vec!["Artist".into()],
            album: "Album".into(),
            duration_ms: 1_000,
            ..LocalTrack::default()
        }
    }

    fn test_config() -> AlternateConfig {
        AlternateConfig {
            piped_api_base: Some("https://piped.example".into()),
            ytdlp_path: None,
            min_score: 0.1,
            skip_on_miss: false,
            volume: 1000,
        }
    }

    #[tokio::test]
    async fn stale_resolve_after_stop_does_not_play() {
        let (hold_tx, hold_rx) = oneshot::channel();
        let plays = Arc::new(AtomicUsize::new(0));
        let output = RecordingOutput {
            plays: Arc::clone(&plays),
            resumes: Arc::new(AtomicUsize::new(0)),
            pauses: Arc::new(AtomicUsize::new(0)),
        };
        let lookup = Arc::new(HoldLookup {
            hold: Mutex::new(Some(hold_rx)),
            audio: vec![1, 2, 3],
        });
        let handle = spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(|_| {}),
            Box::new(output),
            lookup,
        );
        handle.test_load(vec![track("a")], true);
        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.command(PlayerCommand::Stop).unwrap();
        let _ = hold_tx.send(());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn pause_resume_does_not_play_bytes_again() {
        let plays = Arc::new(AtomicUsize::new(0));
        let resumes = Arc::new(AtomicUsize::new(0));
        let pauses = Arc::new(AtomicUsize::new(0));
        let output = RecordingOutput {
            plays: Arc::clone(&plays),
            resumes: Arc::clone(&resumes),
            pauses: Arc::clone(&pauses),
        };
        let lookup = Arc::new(HoldLookup {
            hold: Mutex::new(None),
            audio: vec![1, 2, 3],
        });
        let handle = spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(|_| {}),
            Box::new(output),
            lookup,
        );
        handle.test_load(vec![track("a")], true);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        handle.command(PlayerCommand::Toggle).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.command(PlayerCommand::Toggle).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        assert!(pauses.load(Ordering::SeqCst) >= 1);
        assert!(resumes.load(Ordering::SeqCst) >= 1);
        handle.shutdown().await;
    }

    struct MissLookup {
        searches: Arc<AtomicUsize>,
    }

    impl MediaLookup for MissLookup {
        fn search(
            &self,
            _query: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::matching::Candidate>, bool), String>,
        > {
            self.searches.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok((
                    vec![super::super::matching::Candidate {
                        id: "abcdefghijk".into(),
                        title: "totally unrelated karaoke nightcore mix".into(),
                        uploader: "RandomChannel".into(),
                        duration_ms: Some(9_000),
                    }],
                    true,
                ))
            })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::streams::AudioStream>, bool), String>,
        > {
            Box::pin(async { Err("streams should not run on a ranked miss".into()) })
        }
    }

    struct SearchErrLookup {
        searches: Arc<AtomicUsize>,
        streams: Arc<AtomicUsize>,
    }

    impl MediaLookup for SearchErrLookup {
        fn search(
            &self,
            _query: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::matching::Candidate>, bool), String>,
        > {
            self.searches.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("search provider failed".into()) })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::streams::AudioStream>, bool), String>,
        > {
            self.streams.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("no streams".into()) })
        }
    }

    struct StreamsErrLookup {
        searches: Arc<AtomicUsize>,
        streams: Arc<AtomicUsize>,
    }

    impl MediaLookup for StreamsErrLookup {
        fn search(
            &self,
            _query: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::matching::Candidate>, bool), String>,
        > {
            self.searches.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok((
                    vec![super::super::matching::Candidate {
                        id: "dQw4w9WgXcQ".into(),
                        title: "Song".into(),
                        uploader: "Artist - Topic".into(),
                        duration_ms: Some(1_000),
                    }],
                    true,
                ))
            })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::streams::AudioStream>, bool), String>,
        > {
            self.streams.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("stream lookup failed".into()) })
        }
    }

    #[tokio::test]
    async fn toggle_while_loading_cancels_held_lookup() {
        let (hold_tx, hold_rx) = oneshot::channel();
        let plays = Arc::new(AtomicUsize::new(0));
        let output = RecordingOutput {
            plays: Arc::clone(&plays),
            resumes: Arc::new(AtomicUsize::new(0)),
            pauses: Arc::new(AtomicUsize::new(0)),
        };
        let lookup = Arc::new(HoldLookup {
            hold: Mutex::new(Some(hold_rx)),
            audio: vec![1, 2, 3],
        });
        let handle = spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(|_| {}),
            Box::new(output),
            lookup,
        );
        handle.test_load(vec![track("a")], true);
        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.command(PlayerCommand::Toggle).unwrap();
        let _ = hold_tx.send(());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn skip_on_miss_does_not_loop_under_repeat_context() {
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let output = RecordingOutput {
            plays: Arc::clone(&plays),
            resumes: Arc::new(AtomicUsize::new(0)),
            pauses: Arc::new(AtomicUsize::new(0)),
        };
        let mut config = test_config();
        config.skip_on_miss = true;
        let handle = spawn_inner(
            config,
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(|_| {}),
            Box::new(output),
            Arc::new(MissLookup {
                searches: Arc::clone(&searches),
            }),
        );
        handle
            .command(PlayerCommand::Repeat(crate::player::RepeatMode::Context))
            .unwrap();
        handle.test_load(vec![track("a"), track("b")], true);
        tokio::time::sleep(Duration::from_millis(80)).await;
        let count = searches.load(Ordering::SeqCst);
        assert!(count > 0, "expected at least one search");
        assert!(count <= 2, "looped under repeat-context: {count} searches");
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(searches.load(Ordering::SeqCst), count);
        handle.shutdown().await;
    }

    fn wav_bytes(samples: usize) -> Vec<u8> {
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
            let sample: i16 = if i % 2 == 0 { 400 } else { -400 };
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
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

    struct ScriptLookup {
        searches: Arc<AtomicUsize>,
        body: ScriptedBody,
        hold: Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl MediaLookup for ScriptLookup {
        fn search(
            &self,
            _query: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::matching::Candidate>, bool), String>,
        > {
            self.searches.fetch_add(1, Ordering::SeqCst);
            let rx = self.hold.lock().unwrap_or_else(|p| p.into_inner()).take();
            Box::pin(async move {
                if let Some(rx) = rx {
                    let _ = rx.await;
                }
                Ok((
                    vec![super::super::matching::Candidate {
                        id: "dQw4w9WgXcQ".into(),
                        title: "Song".into(),
                        uploader: "Artist - Topic".into(),
                        duration_ms: Some(1_000),
                    }],
                    true,
                ))
            })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> super::super::provider::LookupFuture<
            Result<(Vec<super::super::streams::AudioStream>, bool), String>,
        > {
            Box::pin(async {
                Ok((
                    vec![super::super::streams::AudioStream {
                        url: "https://example.invalid/a.wav".into(),
                        mime: Some("audio/wav".into()),
                        codec: Some("pcm".into()),
                        format: Some("wav".into()),
                        bitrate: Some(8_000),
                        video_only: false,
                        quality: None,
                        http_headers: Vec::new(),
                    }],
                    true,
                ))
            })
        }

        fn scripted_body(&self) -> Option<ScriptedBody> {
            Some(self.body.clone())
        }
    }

    fn spawn_test(
        config: AlternateConfig,
        output: RecordingOutput,
        lookup: Arc<dyn MediaLookup>,
        notify: Notify,
    ) -> AlternateHandle {
        spawn_inner(
            config,
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            notify,
            Box::new(output),
            lookup,
        )
    }

    #[tokio::test]
    async fn io_failure_after_playback_does_not_skip() {
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(Mutex::new(Vec::<String>::new()));
        let wav = wav_bytes(40_000);
        let split = wav.len() / 2;
        let lookup = Arc::new(ScriptLookup {
            searches: Arc::clone(&searches),
            hold: Mutex::new(None),
            body: ScriptedBody {
                chunks: vec![wav[..split].to_vec(), wav[split..].to_vec()],
                fail: Some("Matched audio stalled.".into()),
                content_length: None,
                fail_after_ms: 400,
            },
        });
        let mut config = test_config();
        config.skip_on_miss = true;
        let notify_errors = Arc::clone(&errors);
        let handle = spawn_test(
            config,
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            lookup,
            Arc::new(move |event| {
                if let EngineEvent::State(state) = event
                    && let Some(error) = state.error
                {
                    notify_errors
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(error);
                }
            }),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let logged = errors.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if logged.iter().any(|error| error.contains("stalled")) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected stall error, got {logged:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn stop_cancels_active_progressive_source() {
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let wav = wav_bytes(40_000);
        let lookup = Arc::new(ScriptLookup {
            searches: Arc::clone(&searches),
            hold: Mutex::new(None),
            body: ScriptedBody {
                chunks: vec![wav],
                fail: None,
                content_length: None,
                fail_after_ms: 0,
            },
        });
        let handle = spawn_test(
            test_config(),
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            lookup,
            Arc::new(|_| {}),
        );
        handle.test_load(vec![track("a")], true);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        handle.command(PlayerCommand::Stop).unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn seek_does_not_re_resolve_or_skip() {
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let positions = Arc::new(Mutex::new(Vec::<u32>::new()));
        let wav = wav_bytes(40_000);
        let lookup = Arc::new(ScriptLookup {
            searches: Arc::clone(&searches),
            hold: Mutex::new(None),
            body: ScriptedBody {
                chunks: vec![wav],
                fail: None,
                content_length: None,
                fail_after_ms: 0,
            },
        });
        let mut config = test_config();
        config.skip_on_miss = true;
        let notify_pos = Arc::clone(&positions);
        let handle = spawn_test(
            config,
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            lookup,
            Arc::new(move |event| {
                if let EngineEvent::State(state) = event {
                    notify_pos
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(state.position_ms);
                }
            }),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        handle.command(PlayerCommand::Seek(500)).unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        let logged = positions.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.contains(&500),
            "seek did not keep session position, got {logged:?}"
        );
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn seek_before_ready_starts_decoder_at_requested_position() {
        let (hold_tx, hold_rx) = oneshot::channel();
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let pcm_slot = Arc::new(Mutex::new(None));
        let decode_slot = Arc::new(Mutex::new(None));
        let positions = Arc::new(Mutex::new(Vec::<u32>::new()));
        let wav = marked_wav(8_000, 1_000);
        let lookup = Arc::new(ScriptLookup {
            searches: Arc::clone(&searches),
            hold: Mutex::new(Some(hold_rx)),
            body: ScriptedBody {
                chunks: vec![wav],
                fail: None,
                content_length: None,
                fail_after_ms: 0,
            },
        });
        let notify_pos = Arc::clone(&positions);
        let handle = spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(move |event| {
                if let EngineEvent::State(state) = event {
                    notify_pos
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(state.position_ms);
                }
            }),
            Box::new(CaptureOutput {
                plays: Arc::clone(&plays),
                pcm: Arc::clone(&pcm_slot),
                decode: Arc::clone(&decode_slot),
                start_ms: Arc::new(Mutex::new(Vec::new())),
            }),
            lookup,
        );
        handle.test_load(vec![track("a")], true);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        handle.command(PlayerCommand::Seek(500)).unwrap();
        let seek_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while tokio::time::Instant::now() < seek_deadline {
            let logged = positions.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if logged.contains(&500) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = hold_tx.send(());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 1);
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        let logged = positions.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.contains(&500),
            "session lost the pre-ready seek, got {logged:?}"
        );
        let mut pcm = pcm_slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .expect("decoder PCM");
        let sample = wait_matching_sample(&mut pcm, |s| s.abs() > 0.5, Duration::from_secs(3));
        assert!(
            sample.is_some_and(|s| s < -0.5),
            "first post-ready audio was not at the requested seek, got {sample:?}"
        );
        drop(pcm);
        *decode_slot.lock().unwrap_or_else(|p| p.into_inner()) = None;
        handle.shutdown().await;
    }

    type StateLog = Vec<(Playback, Option<String>)>;

    fn collect_states(notify_states: Arc<Mutex<StateLog>>) -> Notify {
        Arc::new(move |event| {
            if let EngineEvent::State(state) = event {
                notify_states
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((state.playback, state.error));
            }
        })
    }

    #[tokio::test]
    async fn search_err_does_not_skip_on_miss() {
        let searches = Arc::new(AtomicUsize::new(0));
        let streams = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let states = Arc::new(Mutex::new(Vec::new()));
        let mut config = test_config();
        config.skip_on_miss = true;
        let handle = spawn_test(
            config,
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            Arc::new(SearchErrLookup {
                searches: Arc::clone(&searches),
                streams: Arc::clone(&streams),
            }),
            collect_states(Arc::clone(&states)),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        assert_eq!(streams.load(Ordering::SeqCst), 0);
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        let logged = states.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.iter().any(|(playback, error)| {
                *playback != Playback::Loading && error.as_deref() == Some("search provider failed")
            }),
            "expected transport stop, got {logged:?}"
        );
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn streams_err_does_not_skip_on_miss() {
        let searches = Arc::new(AtomicUsize::new(0));
        let streams = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let states = Arc::new(Mutex::new(Vec::new()));
        let mut config = test_config();
        config.skip_on_miss = true;
        let handle = spawn_test(
            config,
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            Arc::new(StreamsErrLookup {
                searches: Arc::clone(&searches),
                streams: Arc::clone(&streams),
            }),
            collect_states(Arc::clone(&states)),
        );
        handle.test_load(vec![track("Song"), track("Other")], true);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        assert_eq!(streams.load(Ordering::SeqCst), 1);
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        let logged = states.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.iter().any(|(playback, error)| {
                *playback != Playback::Loading && error.as_deref() == Some("stream lookup failed")
            }),
            "expected transport stop, got {logged:?}"
        );
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn invalid_complete_body_fails_instead_of_loading_forever() {
        let searches = Arc::new(AtomicUsize::new(0));
        let plays = Arc::new(AtomicUsize::new(0));
        let states = Arc::new(Mutex::new(Vec::new()));
        let mut config = test_config();
        config.skip_on_miss = true;
        let handle = spawn_test(
            config,
            RecordingOutput {
                plays: Arc::clone(&plays),
                resumes: Arc::new(AtomicUsize::new(0)),
                pauses: Arc::new(AtomicUsize::new(0)),
            },
            Arc::new(ScriptLookup {
                searches: Arc::clone(&searches),
                hold: Mutex::new(None),
                body: ScriptedBody {
                    chunks: vec![vec![0u8; 8_192]],
                    fail: None,
                    content_length: None,
                    fail_after_ms: 0,
                },
            }),
            collect_states(Arc::clone(&states)),
        );
        handle.test_load(vec![track("Song"), track("Other")], true);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let logged = states.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if logged.iter().any(|(playback, error)| {
                *playback != Playback::Loading
                    && error
                        .as_deref()
                        .is_some_and(|message| message.contains("not a playable format"))
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "stuck loading on invalid body: {logged:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        assert_eq!(plays.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }

    struct DeviceOutput {
        status: Arc<Mutex<OutputStatus>>,
        recover_fails: Arc<AtomicUsize>,
        recovers: Arc<AtomicUsize>,
        plays: Arc<AtomicUsize>,
        pauses: Arc<AtomicUsize>,
        pcm: Arc<Mutex<Option<PcmSource>>>,
        decode: Arc<Mutex<Option<DecodeHandle>>>,
    }

    impl AudioOutput for DeviceOutput {
        fn play_bytes(
            &mut self,
            _bytes: Vec<u8>,
            _start_ms: u32,
        ) -> Result<super::super::audio::PlayInfo> {
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn play_pcm(
            &mut self,
            source: PcmSource,
            decode: DecodeHandle,
        ) -> Result<super::super::audio::PlayInfo> {
            *self.pcm.lock().unwrap_or_else(|p| p.into_inner()) = Some(source);
            *self.decode.lock().unwrap_or_else(|p| p.into_inner()) = Some(decode);
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(super::super::audio::PlayInfo {
                duration_ms: Some(1_000),
            })
        }
        fn pause(&mut self) {
            self.pauses.fetch_add(1, Ordering::SeqCst);
        }
        fn resume(&mut self) {}
        fn stop(&mut self) {}
        fn seek(&mut self, _ms: u32) -> Result<()> {
            Ok(())
        }
        fn set_volume(&mut self, _volume: f32) {}
        fn is_finished(&self) -> bool {
            matches!(
                *self.status.lock().unwrap_or_else(|p| p.into_inner()),
                OutputStatus::Ended
            )
        }
        fn status(&self) -> OutputStatus {
            self.status
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
        fn recover(&mut self) -> Result<(), String> {
            self.recovers.fetch_add(1, Ordering::SeqCst);
            if self.recover_fails.load(Ordering::SeqCst) > 0 {
                self.recover_fails.fetch_sub(1, Ordering::SeqCst);
                return Err("no device".into());
            }
            *self.status.lock().unwrap_or_else(|p| p.into_inner()) = OutputStatus::Playing;
            Ok(())
        }
    }

    fn spawn_device(
        output: DeviceOutput,
        lookup: Arc<dyn MediaLookup>,
        notify: Notify,
    ) -> AlternateHandle {
        spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            notify,
            Box::new(output),
            lookup,
        )
    }

    fn wav_script() -> ScriptLookup {
        ScriptLookup {
            searches: Arc::new(AtomicUsize::new(0)),
            hold: Mutex::new(None),
            body: ScriptedBody {
                chunks: vec![wav_bytes(8_000)],
                fail: None,
                content_length: None,
                fail_after_ms: 0,
            },
        }
    }

    async fn wait_plays(plays: &Arc<AtomicUsize>, want: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) < want {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), want);
    }

    #[tokio::test]
    async fn playing_device_loss_recovers_same_track() {
        let plays = Arc::new(AtomicUsize::new(0));
        let recovers = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(Mutex::new(OutputStatus::Playing));
        let lookup = Arc::new(wav_script());
        let searches = Arc::clone(&lookup.searches);
        let handle = spawn_device(
            DeviceOutput {
                status: Arc::clone(&status),
                recover_fails: Arc::new(AtomicUsize::new(0)),
                recovers: Arc::clone(&recovers),
                plays: Arc::clone(&plays),
                pauses: Arc::new(AtomicUsize::new(0)),
                pcm: Arc::new(Mutex::new(None)),
                decode: Arc::new(Mutex::new(None)),
            },
            lookup,
            Arc::new(|_| {}),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        wait_plays(&plays, 1).await;
        *status.lock().unwrap_or_else(|p| p.into_inner()) = OutputStatus::DeviceLost;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 2);
        assert!(recovers.load(Ordering::SeqCst) >= 1);
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn paused_device_loss_recovers_paused() {
        let plays = Arc::new(AtomicUsize::new(0));
        let pauses = Arc::new(AtomicUsize::new(0));
        let recovers = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(Mutex::new(OutputStatus::Playing));
        let lookup = Arc::new(wav_script());
        let searches = Arc::clone(&lookup.searches);
        let handle = spawn_device(
            DeviceOutput {
                status: Arc::clone(&status),
                recover_fails: Arc::new(AtomicUsize::new(0)),
                recovers: Arc::clone(&recovers),
                plays: Arc::clone(&plays),
                pauses: Arc::clone(&pauses),
                pcm: Arc::new(Mutex::new(None)),
                decode: Arc::new(Mutex::new(None)),
            },
            lookup,
            Arc::new(|_| {}),
        );
        handle.test_load(vec![track("a")], true);
        wait_plays(&plays, 1).await;
        handle.command(PlayerCommand::Toggle).unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        *status.lock().unwrap_or_else(|p| p.into_inner()) = OutputStatus::DeviceLost;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 2);
        assert!(recovers.load(Ordering::SeqCst) >= 1);
        assert!(pauses.load(Ordering::SeqCst) >= 2);
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn device_open_backoff_then_success() {
        let plays = Arc::new(AtomicUsize::new(0));
        let recovers = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(Mutex::new(OutputStatus::Playing));
        let lookup = Arc::new(wav_script());
        let searches = Arc::clone(&lookup.searches);
        let handle = spawn_device(
            DeviceOutput {
                status: Arc::clone(&status),
                recover_fails: Arc::new(AtomicUsize::new(3)),
                recovers: Arc::clone(&recovers),
                plays: Arc::clone(&plays),
                pauses: Arc::new(AtomicUsize::new(0)),
                pcm: Arc::new(Mutex::new(None)),
                decode: Arc::new(Mutex::new(None)),
            },
            lookup,
            Arc::new(|_| {}),
        );
        handle.test_load(vec![track("a")], true);
        wait_plays(&plays, 1).await;
        *status.lock().unwrap_or_else(|p| p.into_inner()) = OutputStatus::DeviceLost;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        while tokio::time::Instant::now() < deadline && plays.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert_eq!(plays.load(Ordering::SeqCst), 2);
        assert!(recovers.load(Ordering::SeqCst) >= 4);
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn ended_and_decode_failed_unchanged() {
        let plays = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(Mutex::new(OutputStatus::Playing));
        let lookup = Arc::new(wav_script());
        let searches = Arc::clone(&lookup.searches);
        let handle = spawn_device(
            DeviceOutput {
                status: Arc::clone(&status),
                recover_fails: Arc::new(AtomicUsize::new(0)),
                recovers: Arc::new(AtomicUsize::new(0)),
                plays: Arc::clone(&plays),
                pauses: Arc::new(AtomicUsize::new(0)),
                pcm: Arc::new(Mutex::new(None)),
                decode: Arc::new(Mutex::new(None)),
            },
            lookup,
            Arc::new(|_| {}),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        wait_plays(&plays, 1).await;
        *status.lock().unwrap_or_else(|p| p.into_inner()) = OutputStatus::Ended;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && searches.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(searches.load(Ordering::SeqCst), 2);
        handle.shutdown().await;

        let plays = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(Mutex::new(OutputStatus::Playing));
        let states = Arc::new(Mutex::new(Vec::new()));
        let lookup = Arc::new(wav_script());
        let searches = Arc::clone(&lookup.searches);
        let handle = spawn_device(
            DeviceOutput {
                status: Arc::clone(&status),
                recover_fails: Arc::new(AtomicUsize::new(0)),
                recovers: Arc::new(AtomicUsize::new(0)),
                plays: Arc::clone(&plays),
                pauses: Arc::new(AtomicUsize::new(0)),
                pcm: Arc::new(Mutex::new(None)),
                decode: Arc::new(Mutex::new(None)),
            },
            lookup,
            collect_states(Arc::clone(&states)),
        );
        handle.test_load(vec![track("a"), track("b")], true);
        wait_plays(&plays, 1).await;
        *status.lock().unwrap_or_else(|p| p.into_inner()) =
            OutputStatus::Failed("Couldn't decode audio.".into());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let logged = states.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if logged.iter().any(|(_, error)| {
                error
                    .as_deref()
                    .is_some_and(|message| message.contains("Couldn't decode audio"))
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected decode fail, got {logged:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(searches.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn ready_start_mp3_does_not_seek_zero() {
        let plays = Arc::new(AtomicUsize::new(0));
        let pcm_slot = Arc::new(Mutex::new(None));
        let decode_slot = Arc::new(Mutex::new(None));
        let lookup = Arc::new(ScriptLookup {
            searches: Arc::new(AtomicUsize::new(0)),
            hold: Mutex::new(None),
            body: ScriptedBody {
                chunks: vec![crate::alternate::decode::TONE_MP3.to_vec()],
                fail: None,
                content_length: None,
                fail_after_ms: 0,
            },
        });
        let handle = spawn_inner(
            test_config(),
            Arc::new(ApiClient::new(
                reqwest::Client::new(),
                Arc::new(crate::api::NetActivity::default()),
            )),
            reqwest::Client::new(),
            Arc::new(|_| {}),
            Box::new(CaptureOutput {
                plays: Arc::clone(&plays),
                pcm: Arc::clone(&pcm_slot),
                decode: Arc::clone(&decode_slot),
                start_ms: Arc::new(Mutex::new(Vec::new())),
            }),
            lookup,
        );
        handle.test_load(vec![track("a")], true);
        wait_plays(&plays, 1).await;
        let epoch = {
            let decode = decode_slot.lock().unwrap_or_else(|p| p.into_inner());
            decode.as_ref().map(DecodeHandle::epoch).unwrap_or(99)
        };
        assert_eq!(epoch, 0, "Ready→start issued a redundant seek");
        let mut pcm = pcm_slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .expect("pcm");
        assert!(wait_nonzero_sample(&mut pcm, Duration::from_secs(5)));
        drop(pcm);
        drop(decode_slot.lock().unwrap_or_else(|p| p.into_inner()).take());
        handle.shutdown().await;
    }
}

// Keep a generation counter type in this module so stale-event tests stay honest.
#[allow(dead_code)]
pub(crate) struct EventGuard {
    current: Arc<AtomicU64>,
    mine: u64,
}

#[allow(dead_code)]
impl EventGuard {
    pub(crate) fn new(current: Arc<AtomicU64>) -> Self {
        let mine = current.load(Ordering::SeqCst);
        Self { current, mine }
    }

    pub(crate) fn allows(&self) -> bool {
        self.current.load(Ordering::SeqCst) == self.mine
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn bumped_generation_rejects_stale_events() {
        let current = Arc::new(AtomicU64::new(3));
        let guard = EventGuard::new(Arc::clone(&current));
        assert!(guard.allows());
        current.fetch_add(1, Ordering::SeqCst);
        assert!(!guard.allows());
    }
}
