//! HTTP fetch: prefix Range, sparse follow-up, sequential fallback.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, CONTENT_TYPE};

use super::buffer::{ByteRange, DEMAND_WINDOW, MAX_BYTES, SharedAudio};
use super::decode::FormatHint;
use super::probe;
use super::provider::{MediaLookup, ScriptedBody};
use super::streams::{AudioStream, select_audio_stream};

pub const INITIAL_PREFIX: u64 = 2 * 1024 * 1024;
const COALESCE_GAP: u64 = 128 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct FetchPolicy {
    pub header_timeout: Duration,
    pub stall_timeout: Duration,
    pub retry_initial: Duration,
    pub retry_cap: Duration,
    pub dishonest_retries: u32,
    pub max_refresh: u32,
}

impl FetchPolicy {
    pub const fn production() -> Self {
        Self {
            header_timeout: Duration::from_secs(12),
            stall_timeout: Duration::from_secs(20),
            retry_initial: Duration::from_millis(250),
            retry_cap: Duration::from_secs(20),
            dishonest_retries: 2,
            max_refresh: 6,
        }
    }

    #[cfg(test)]
    pub const fn for_test() -> Self {
        Self {
            header_timeout: Duration::from_millis(80),
            stall_timeout: Duration::from_millis(120),
            retry_initial: Duration::from_millis(15),
            retry_cap: Duration::from_millis(80),
            dishonest_retries: 2,
            max_refresh: 4,
        }
    }
}

enum FetchErr {
    Cancelled,
    Permanent(String),
    Transient(String),
    Expired,
}

impl FetchErr {
    fn into_string(self) -> String {
        match self {
            Self::Cancelled => "cancelled".into(),
            Self::Permanent(message) | Self::Transient(message) => message,
            Self::Expired => "Matched audio host refused the request.".into(),
        }
    }
}

fn permanent(message: impl Into<String>) -> FetchErr {
    FetchErr::Permanent(message.into())
}

fn transient(message: impl Into<String>) -> FetchErr {
    FetchErr::Transient(message.into())
}

pub fn header_allowed(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "user-agent" | "referer" | "origin"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentRange {
    pub start: u64,
    pub end_inclusive: u64,
    pub total: Option<u64>,
}

impl ContentRange {
    pub fn end_exclusive(self) -> u64 {
        self.end_inclusive.saturating_add(1)
    }
}

pub fn parse_content_range(raw: &str) -> Result<ContentRange, String> {
    let err = || "Couldn't read matched audio.".to_string();
    let rest = raw.trim().strip_prefix("bytes").ok_or_else(err)?.trim();
    if rest.starts_with('*') {
        return Err(err());
    }
    let (span, total_part) = rest.split_once('/').ok_or_else(err)?;
    let (start_s, end_s) = span.split_once('-').ok_or_else(err)?;
    if start_s.trim().is_empty() || end_s.trim().is_empty() {
        return Err(err());
    }
    let start: u64 = start_s.trim().parse().map_err(|_| err())?;
    let end_inclusive: u64 = end_s.trim().parse().map_err(|_| err())?;
    if end_inclusive < start {
        return Err(err());
    }
    let total = match total_part.trim() {
        "" => return Err(err()),
        "*" => None,
        n => {
            if n.chars().any(|ch| !ch.is_ascii_digit()) {
                return Err(err());
            }
            let total: u64 = n.parse().map_err(|_| err())?;
            if total == 0 || start >= total || end_inclusive >= total {
                return Err(err());
            }
            Some(total)
        }
    };
    Ok(ContentRange {
        start,
        end_inclusive,
        total,
    })
}

struct PrefixBuf {
    data: Vec<u8>,
}

impl PrefixBuf {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn note(&mut self, offset: u64, chunk: &[u8]) {
        let max = INITIAL_PREFIX as usize;
        if self.data.len() >= max || chunk.is_empty() {
            return;
        }
        let Ok(start) = usize::try_from(offset) else {
            return;
        };
        if start > self.data.len() {
            return;
        }
        let take = chunk.len().min(max.saturating_sub(start));
        if take == 0 {
            return;
        }
        if start == self.data.len() {
            self.data.extend_from_slice(&chunk[..take]);
            return;
        }
        let overlap = (self.data.len() - start).min(take);
        self.data[start..start + overlap].copy_from_slice(&chunk[..overlap]);
        if take > overlap {
            self.data.extend_from_slice(&chunk[overlap..take]);
        }
    }
}

enum Outcome {
    Aborted,
    Sequential(reqwest::Response),
    Partial {
        start: u64,
        total: u64,
        end_exclusive: u64,
        response: reqwest::Response,
    },
}

#[cfg(test)]
pub async fn fetch_media(
    http: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    buffer: &SharedAudio,
    on_ready: &mut impl FnMut(),
) -> Result<(), String> {
    fetch_with(
        http,
        url.to_string(),
        headers.to_vec(),
        buffer,
        on_ready,
        FetchPolicy::production(),
        None,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_with(
    http: &reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    buffer: &SharedAudio,
    on_ready: &mut impl FnMut(),
    policy: FetchPolicy,
    lookup: Option<&dyn MediaLookup>,
    video_id: Option<&str>,
    hint: Option<&FormatHint>,
) -> Result<(), String> {
    match fetch_inner(
        http, url, headers, buffer, on_ready, policy, lookup, video_id, hint,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(error) => Err(error.into_string()),
    }
}

struct FetchCtx<'a> {
    http: &'a reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    buffer: &'a SharedAudio,
    policy: FetchPolicy,
    lookup: Option<&'a dyn MediaLookup>,
    video_id: Option<&'a str>,
    hint: Option<&'a FormatHint>,
    backoff: Duration,
    refreshes: u32,
}

#[allow(clippy::too_many_arguments)]
async fn fetch_inner(
    http: &reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    buffer: &SharedAudio,
    on_ready: &mut impl FnMut(),
    policy: FetchPolicy,
    lookup: Option<&dyn MediaLookup>,
    video_id: Option<&str>,
    hint: Option<&FormatHint>,
) -> Result<(), FetchErr> {
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    let mut ctx = FetchCtx {
        http,
        url,
        headers,
        buffer,
        policy,
        lookup,
        video_id,
        hint,
        backoff: policy.retry_initial,
        refreshes: 0,
    };
    loop {
        if ctx.buffer.is_cancelled() {
            return Err(FetchErr::Cancelled);
        }
        if let Some(message) = ctx.buffer.error() {
            return Err(permanent(message));
        }
        let first_end = INITIAL_PREFIX.saturating_sub(1);
        match request_range(
            ctx.http,
            &ctx.url,
            &ctx.headers,
            ctx.buffer,
            0,
            first_end,
            false,
            ctx.policy,
        )
        .await
        {
            Ok(Outcome::Aborted) => continue,
            Ok(Outcome::Sequential(response)) => {
                let mut prefix = PrefixBuf::new();
                match stream_from_zero(
                    response,
                    ctx.buffer,
                    Some(&mut prefix),
                    on_ready,
                    ctx.policy,
                )
                .await
                {
                    Ok(StreamDone::Complete) => {
                        ctx.buffer.close();
                        return finish_complete_body(
                            &prefix.data,
                            ctx.buffer.len(),
                            true,
                            on_ready,
                        )
                        .map_err(permanent);
                    }
                    Ok(StreamDone::Incomplete) => {
                        if ctx.buffer.filled_bytes() == 0 {
                            ctx.buffer.fail("Couldn't read matched audio.");
                            return Err(permanent("Couldn't read matched audio."));
                        }
                        ctx.reset_backoff();
                        ctx.wait_backoff().await?;
                    }
                    Err(FetchErr::Transient(_)) => ctx.wait_backoff().await?,
                    Ok(StreamDone::Retarget) => continue,
                    Err(error) => ctx.on_request_err(error).await?,
                }
            }
            Ok(Outcome::Partial {
                start,
                total,
                end_exclusive,
                response,
            }) => {
                if start != 0 {
                    ctx.buffer.fail("Couldn't fetch matched audio.");
                    return Err(permanent("Couldn't fetch matched audio."));
                }
                if total == 0 {
                    return Err(permanent("Matched audio was empty."));
                }
                if total as usize > MAX_BYTES {
                    return Err(permanent("Matched audio is too large to play."));
                }
                ctx.buffer.enable_random_access(total).map_err(permanent)?;
                let mut prefix = PrefixBuf::new();
                match stream_range(
                    response,
                    start,
                    end_exclusive,
                    ctx.buffer,
                    Some(&mut prefix),
                    None,
                    on_ready,
                    ctx.policy,
                )
                .await
                {
                    Ok(_) => {}
                    Err(FetchErr::Transient(_)) => {
                        ctx.wait_backoff().await?;
                        if ctx.buffer.filled_bytes() > 0 && ctx.buffer.is_random_access() {
                            if ctx.buffer.is_cancelled() {
                                return Err(FetchErr::Cancelled);
                            }
                            maybe_ready(ctx.buffer, &prefix.data, false, on_ready);
                            return range_followup(&mut ctx, prefix, on_ready).await;
                        }
                        continue;
                    }
                    Err(error) => {
                        ctx.on_request_err(error).await?;
                        continue;
                    }
                }
                if ctx.buffer.is_cancelled() {
                    return Err(FetchErr::Cancelled);
                }
                if let Some(message) = ctx.buffer.error() {
                    return Err(permanent(message));
                }
                if ctx.buffer.filled_bytes() == 0 {
                    ctx.buffer.fail("Couldn't read matched audio.");
                    return Err(permanent("Couldn't read matched audio."));
                }
                maybe_ready(ctx.buffer, &prefix.data, false, on_ready);
                if ctx.buffer.filled_bytes() >= total {
                    ctx.buffer.close();
                    return finish_complete_body(&prefix.data, ctx.buffer.len(), true, on_ready)
                        .map_err(permanent);
                }
                return range_followup(&mut ctx, prefix, on_ready).await;
            }
            Err(error) => ctx.on_request_err(error).await?,
        }
    }
}

enum StreamDone {
    Complete,
    Incomplete,
    Retarget,
}

impl FetchCtx<'_> {
    async fn wait_backoff(&mut self) -> Result<(), FetchErr> {
        let delay = self.backoff;
        self.backoff = self.backoff.saturating_mul(2).min(self.policy.retry_cap);
        backoff_wait(self.buffer, delay).await
    }

    fn reset_backoff(&mut self) {
        self.backoff = self.policy.retry_initial;
    }

    async fn on_request_err(&mut self, error: FetchErr) -> Result<(), FetchErr> {
        match error {
            FetchErr::Cancelled => Err(FetchErr::Cancelled),
            FetchErr::Permanent(message) => Err(FetchErr::Permanent(message)),
            FetchErr::Transient(_) => self.wait_backoff().await,
            FetchErr::Expired => self.refresh_url().await,
        }
    }

    async fn refresh_url(&mut self) -> Result<(), FetchErr> {
        if self.refreshes >= self.policy.max_refresh {
            return Err(permanent("Matched audio host refused the request."));
        }
        self.refreshes = self.refreshes.saturating_add(1);
        match refresh_stream(self.lookup, self.video_id, self.hint).await {
            Ok((url, headers)) => {
                self.url = url;
                self.headers = headers;
            }
            Err(FetchErr::Permanent(message)) => return Err(FetchErr::Permanent(message)),
            Err(FetchErr::Cancelled) => return Err(FetchErr::Cancelled),
            Err(FetchErr::Expired | FetchErr::Transient(_)) => {}
        }
        self.wait_backoff().await
    }
}

async fn backoff_wait(buffer: &SharedAudio, delay: Duration) -> Result<(), FetchErr> {
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    if delay.is_zero() {
        return Ok(());
    }
    let notified = buffer.notified();
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    tokio::select! {
        _ = tokio::time::sleep(delay) => {
            if buffer.is_cancelled() {
                Err(FetchErr::Cancelled)
            } else {
                Ok(())
            }
        }
        _ = notified => {
            if buffer.is_cancelled() {
                Err(FetchErr::Cancelled)
            } else {
                Ok(())
            }
        }
    }
}

async fn refresh_stream(
    lookup: Option<&dyn MediaLookup>,
    video_id: Option<&str>,
    hint: Option<&FormatHint>,
) -> Result<(String, Vec<(String, String)>), FetchErr> {
    let lookup = lookup.ok_or_else(|| permanent("Matched audio host refused the request."))?;
    let id = video_id.ok_or_else(|| permanent("Matched audio host refused the request."))?;
    let (streams, _) = lookup.streams(id).await.map_err(transient)?;
    let stream = select_compatible(&streams, hint).ok_or_else(|| {
        permanent("No playable audio stream (need AAC/M4A or MP3; Opus/WebM is not decoded).")
    })?;
    Ok((stream.url.clone(), stream.http_headers.clone()))
}

fn select_compatible<'a>(
    streams: &'a [AudioStream],
    hint: Option<&FormatHint>,
) -> Option<&'a AudioStream> {
    let Some(want) = hint.and_then(|hint| hint.extension.as_deref()) else {
        return select_audio_stream(streams);
    };
    let matched: Vec<AudioStream> = streams
        .iter()
        .filter(|stream| {
            FormatHint::from_labels(
                stream.format.as_deref(),
                stream.mime.as_deref(),
                Some(stream.url.as_str()),
            )
            .extension
            .as_deref()
                == Some(want)
        })
        .cloned()
        .collect();
    let picked = select_audio_stream(&matched)?;
    streams.iter().find(|stream| stream.url == picked.url)
}

pub async fn fetch_scripted(
    buffer: &SharedAudio,
    script: ScriptedBody,
    on_ready: &mut impl FnMut(),
) -> Result<(), String> {
    if let Some(length) = script.content_length {
        buffer.set_content_length(length)?;
    }
    let mut prefix = Vec::new();
    let mut downloaded = 0usize;
    for chunk in script.chunks {
        buffer.append(&chunk)?;
        downloaded = downloaded.saturating_add(chunk.len());
        if prefix.len() < INITIAL_PREFIX as usize {
            let take = (INITIAL_PREFIX as usize - prefix.len()).min(chunk.len());
            prefix.extend_from_slice(&chunk[..take]);
        }
        if probe::playback_ready(&prefix, downloaded, false) {
            on_ready();
        }
        if buffer.is_cancelled() {
            return Err("cancelled".into());
        }
    }
    if let Some(error) = script.fail {
        if script.fail_after_ms > 0 {
            tokio::time::sleep(Duration::from_millis(script.fail_after_ms)).await;
            if buffer.is_cancelled() {
                return Err("cancelled".into());
            }
        }
        buffer.fail(error.clone());
        return Err(error);
    }
    buffer.close();
    finish_complete_body(&prefix, downloaded, true, on_ready)
}

async fn range_followup(
    ctx: &mut FetchCtx<'_>,
    mut prefix: PrefixBuf,
    on_ready: &mut impl FnMut(),
) -> Result<(), FetchErr> {
    let mut did_tail = false;
    let mut last_filled = ctx.buffer.filled_bytes();
    let mut no_progress = 0u32;
    loop {
        if ctx.buffer.is_cancelled() {
            return Err(FetchErr::Cancelled);
        }
        if let Some(message) = ctx.buffer.error() {
            return Err(permanent(message));
        }
        if !ctx.buffer.is_open() {
            return Ok(());
        }
        maybe_ready(ctx.buffer, &prefix.data, false, on_ready);
        let Some(total) = ctx.buffer.content_length() else {
            return Err(permanent("Couldn't fetch matched audio."));
        };
        if ctx.buffer.filled_bytes() >= total {
            ctx.buffer.close();
            return finish_complete_body(&prefix.data, ctx.buffer.len(), true, on_ready)
                .map_err(permanent);
        }
        let ready = probe::playback_ready_sparse(
            &prefix.data,
            &tail_bytes(ctx.buffer, &prefix.data),
            false,
        );
        let Some(work) = next_work(ctx.buffer, &prefix.data, total, did_tail, ready) else {
            tokio::select! {
                _ = ctx.buffer.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
            continue;
        };
        if let Some((start, _)) = probe::tail_prefetch(&prefix.data, total)
            && work.start >= start
        {
            did_tail = true;
        }
        let end_inclusive = work.end.saturating_sub(1);
        match request_range(
            ctx.http,
            &ctx.url,
            &ctx.headers,
            ctx.buffer,
            work.start,
            end_inclusive,
            true,
            ctx.policy,
        )
        .await
        {
            Ok(Outcome::Aborted) => continue,
            Ok(Outcome::Sequential(response)) => {
                match stream_from_zero(
                    response,
                    ctx.buffer,
                    Some(&mut prefix),
                    on_ready,
                    ctx.policy,
                )
                .await
                {
                    Ok(_) => {
                        note_progress(ctx, &mut last_filled, &mut no_progress);
                    }
                    Err(FetchErr::Transient(_)) => ctx.wait_backoff().await?,
                    Err(error) => ctx.on_request_err(error).await?,
                }
            }
            Ok(Outcome::Partial {
                start,
                total: got_total,
                end_exclusive,
                response,
            }) => {
                if got_total != total {
                    ctx.buffer.fail("Couldn't read matched audio.");
                    return Err(permanent("Couldn't read matched audio."));
                }
                if start > work.start || end_exclusive <= work.start {
                    ctx.buffer.fail("Couldn't read matched audio.");
                    return Err(permanent("Couldn't read matched audio."));
                }
                let flight = ByteRange::new(start, end_exclusive);
                match stream_range(
                    response,
                    start,
                    end_exclusive,
                    ctx.buffer,
                    Some(&mut prefix),
                    flight,
                    on_ready,
                    ctx.policy,
                )
                .await
                {
                    Ok(StreamDone::Complete | StreamDone::Retarget) => {
                        note_progress(ctx, &mut last_filled, &mut no_progress);
                    }
                    Ok(StreamDone::Incomplete) => {
                        if ctx.buffer.filled_bytes() > last_filled {
                            note_progress(ctx, &mut last_filled, &mut no_progress);
                        } else {
                            no_progress = no_progress.saturating_add(1);
                            if no_progress > ctx.policy.dishonest_retries {
                                ctx.buffer.fail("Couldn't read matched audio.");
                                return Err(permanent("Couldn't read matched audio."));
                            }
                        }
                        ctx.wait_backoff().await?;
                    }
                    Err(FetchErr::Transient(_)) => ctx.wait_backoff().await?,
                    Err(error) => ctx.on_request_err(error).await?,
                }
            }
            Err(error) => ctx.on_request_err(error).await?,
        }
    }
}

fn note_progress(ctx: &mut FetchCtx<'_>, last_filled: &mut u64, no_progress: &mut u32) {
    let filled = ctx.buffer.filled_bytes();
    if filled > *last_filled {
        *last_filled = filled;
        *no_progress = 0;
        ctx.reset_backoff();
    }
}

fn next_work(
    buffer: &SharedAudio,
    prefix: &[u8],
    total: u64,
    did_tail: bool,
    ready: bool,
) -> Option<ByteRange> {
    if let Some(demand) = buffer.current_demand()
        && let Some(hole) = buffer.first_hole(demand.start, demand.end)
    {
        return clip_work(hole, DEMAND_WINDOW, total);
    }
    if ready {
        return None;
    }
    if !did_tail
        && let Some((start, end)) = probe::tail_prefetch(prefix, total)
        && let Some(hole) = buffer.first_hole(start, end)
    {
        return clip_work(hole, probe::TAIL_PREFETCH, total);
    }
    let hole = buffer.first_hole(0, total)?;
    clip_work(hole, INITIAL_PREFIX, total)
}

fn clip_work(hole: ByteRange, cap: u64, total: u64) -> Option<ByteRange> {
    let end = hole.start.saturating_add(cap).min(hole.end).min(total);
    ByteRange::new(hole.start, end)
}

fn tail_bytes(buffer: &SharedAudio, prefix: &[u8]) -> Vec<u8> {
    let Some(total) = buffer.content_length() else {
        return Vec::new();
    };
    let Some((start, end)) = probe::tail_prefetch(prefix, total) else {
        return Vec::new();
    };
    buffer.copy_filled(start, end).unwrap_or_default()
}

fn maybe_ready(buffer: &SharedAudio, prefix: &[u8], complete: bool, on_ready: &mut impl FnMut()) {
    if probe::playback_ready_sparse(prefix, &tail_bytes(buffer, prefix), complete) {
        on_ready();
    }
}

fn finish_complete_body(
    prefix: &[u8],
    downloaded: usize,
    complete: bool,
    on_ready: &mut impl FnMut(),
) -> Result<(), String> {
    if downloaded == 0 {
        return Err("Matched audio was empty.".into());
    }
    if probe::playback_ready(prefix, downloaded, complete) {
        on_ready();
        Ok(())
    } else {
        Err("Matched audio is not a playable format.".into())
    }
}

#[allow(clippy::too_many_arguments)]
async fn request_range(
    http: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    buffer: &SharedAudio,
    start: u64,
    end_inclusive: u64,
    abortable: bool,
    policy: FetchPolicy,
) -> Result<Outcome, FetchErr> {
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    let request =
        build_request(http, url, headers, Some((start, end_inclusive))).map_err(permanent)?;
    let execute = tokio::time::timeout(policy.header_timeout, http.execute(request));
    let response = if abortable {
        let flight = ByteRange::new(start, end_inclusive.saturating_add(1));
        tokio::select! {
            result = execute => result,
            _ = wait_retarget(buffer, flight) => return Ok(Outcome::Aborted),
        }
    } else {
        execute.await
    }
    .map_err(|_| transient("Matched audio timed out."))?
    .map_err(|_| transient("Couldn't fetch matched audio."))?;

    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    match response.status() {
        StatusCode::OK => {
            if let Some(length) = response.content_length()
                && length as usize > MAX_BYTES
            {
                return Err(permanent("Matched audio is too large to play."));
            }
            Ok(Outcome::Sequential(response))
        }
        StatusCode::PARTIAL_CONTENT => {
            if is_multipart(&response) {
                return Err(permanent("Couldn't read matched audio."));
            }
            let header = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| permanent("Couldn't read matched audio."))?;
            let parsed = parse_content_range(header).map_err(permanent)?;
            let Some(total) = parsed.total else {
                return Err(permanent("Couldn't fetch matched audio."));
            };
            if total as usize > MAX_BYTES {
                return Err(permanent("Matched audio is too large to play."));
            }
            if let Some(known) = buffer.content_length()
                && known != total
            {
                return Err(permanent("Couldn't read matched audio."));
            }
            Ok(Outcome::Partial {
                start: parsed.start,
                total,
                end_exclusive: parsed.end_exclusive(),
                response,
            })
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::GONE => {
            Err(FetchErr::Expired)
        }
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::BAD_GATEWAY
        | StatusCode::GATEWAY_TIMEOUT => Err(transient("Matched audio host refused the request.")),
        status if status.is_server_error() => {
            Err(transient("Matched audio host refused the request."))
        }
        _ => Err(permanent("Matched audio host refused the request.")),
    }
}

async fn wait_retarget(buffer: &SharedAudio, flight: Option<ByteRange>) {
    loop {
        if should_retarget(buffer, flight) || !buffer.is_open() {
            return;
        }
        let notified = buffer.notified();
        if should_retarget(buffer, flight) || !buffer.is_open() {
            return;
        }
        notified.await;
    }
}

fn should_retarget(buffer: &SharedAudio, flight: Option<ByteRange>) -> bool {
    if buffer.is_cancelled() || !buffer.is_open() {
        return true;
    }
    let Some(flight) = flight else {
        return false;
    };
    let Some(demand) = buffer.current_demand() else {
        return false;
    };
    let Some(hole) = buffer.first_hole(demand.start, demand.end) else {
        return false;
    };
    !flight.overlaps_or_near(hole, COALESCE_GAP)
}

fn is_multipart(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("multipart"))
}

fn build_request(
    http: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    range: Option<(u64, u64)>,
) -> Result<reqwest::Request, String> {
    let mut builder = http.get(url);
    for (key, value) in headers {
        if header_allowed(key) {
            builder = builder.header(key, value);
        }
    }
    if let Some((start, end)) = range {
        builder = builder.header("range", format!("bytes={start}-{end}"));
    }
    let mut request = builder
        .build()
        .map_err(|_| "Couldn't fetch matched audio.".to_string())?;
    *request.timeout_mut() = None;
    Ok(request)
}

async fn stream_from_zero(
    mut response: reqwest::Response,
    buffer: &SharedAudio,
    mut prefix: Option<&mut PrefixBuf>,
    on_ready: &mut impl FnMut(),
    policy: FetchPolicy,
) -> Result<StreamDone, FetchErr> {
    if let Some(length) = response.content_length() {
        if length as usize > MAX_BYTES {
            return Err(permanent("Matched audio is too large to play."));
        }
        if buffer.content_length().is_none() {
            buffer.set_content_length(length).map_err(permanent)?;
        }
    }
    let mut offset = 0u64;
    loop {
        let chunk = match tokio::time::timeout(policy.stall_timeout, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => return Err(transient("Couldn't read matched audio.")),
            Err(_) => return Err(transient("Matched audio stalled.")),
        };
        if let Some(prefix) = prefix.as_mut() {
            prefix.note(offset, &chunk);
        }
        buffer.write_at(offset, &chunk).map_err(permanent)?;
        offset = offset.saturating_add(chunk.len() as u64);
        match prefix.as_ref() {
            Some(prefix) => maybe_ready(buffer, &prefix.data, false, on_ready),
            None => maybe_ready(buffer, &[], false, on_ready),
        }
        if buffer.is_cancelled() {
            return Err(FetchErr::Cancelled);
        }
    }
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    let short_of_length = buffer.content_length().is_some_and(|total| offset < total);
    if offset == 0 || short_of_length {
        Ok(StreamDone::Incomplete)
    } else {
        Ok(StreamDone::Complete)
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_range(
    mut response: reqwest::Response,
    mut offset: u64,
    limit: u64,
    buffer: &SharedAudio,
    mut prefix: Option<&mut PrefixBuf>,
    flight: Option<ByteRange>,
    on_ready: &mut impl FnMut(),
    policy: FetchPolicy,
) -> Result<StreamDone, FetchErr> {
    loop {
        if offset >= limit {
            return Ok(StreamDone::Complete);
        }
        let chunk = match tokio::time::timeout(policy.stall_timeout, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => return Err(transient("Couldn't read matched audio.")),
            Err(_) => return Err(transient("Matched audio stalled.")),
        };
        let mut chunk = chunk.as_ref();
        let remain = (limit - offset) as usize;
        if chunk.len() > remain {
            chunk = &chunk[..remain];
        }
        if let Some(prefix) = prefix.as_mut() {
            prefix.note(offset, chunk);
        }
        buffer.write_at(offset, chunk).map_err(permanent)?;
        offset = offset.saturating_add(chunk.len() as u64);
        match prefix.as_ref() {
            Some(prefix) => maybe_ready(buffer, &prefix.data, false, on_ready),
            None => maybe_ready(buffer, &[], false, on_ready),
        }
        if buffer.is_cancelled() {
            return Err(FetchErr::Cancelled);
        }
        if should_retarget(buffer, flight) {
            return Ok(StreamDone::Retarget);
        }
    }
    if buffer.is_cancelled() {
        return Err(FetchErr::Cancelled);
    }
    if offset < limit {
        Ok(StreamDone::Incomplete)
    } else {
        Ok(StreamDone::Complete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alternate::decode::{
        FormatHint, TONE_M4A, TONE_MP3, spawn_decoder, wait_matching_sample, wait_nonzero_sample,
    };
    use crate::alternate::matching::Candidate;
    use crate::alternate::provider::MediaLookup;
    use crate::alternate::streams::AudioStream;
    use std::io::{Read, Seek, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn content_range_parse_accepts_valid_and_rejects_junk() {
        let ok = parse_content_range("bytes 0-1023/2048").unwrap();
        assert_eq!(ok.start, 0);
        assert_eq!(ok.end_inclusive, 1023);
        assert_eq!(ok.total, Some(2048));
        assert_eq!(ok.end_exclusive(), 1024);

        let unknown = parse_content_range("  bytes 100-199/* ").unwrap();
        assert_eq!(unknown.start, 100);
        assert_eq!(unknown.total, None);

        assert!(parse_content_range("bytes */2048").is_err());
        assert!(parse_content_range("bytes 5-4/10").is_err());
        assert!(parse_content_range("bytes 0-10/10").is_err());
        assert!(parse_content_range("bytes 0-10/0").is_err());
        assert!(parse_content_range("bytes abc-1/10").is_err());
        assert!(parse_content_range("items 0-1/2").is_err());
        assert!(parse_content_range("bytes 0-1").is_err());
        assert!(parse_content_range("bytes 0-/10").is_err());
    }

    #[derive(Clone, Copy)]
    enum StubMode {
        Range,
        IgnoreRange,
        InvalidRange,
        MismatchTotal,
        Multipart,
        EmptyPartial,
        WrongStart,
        TruncatedPartial,
        DropMid,
        Forbidden,
        CloseEarly,
        HangRange,
    }

    struct StubCfg {
        body: Vec<u8>,
        first: StubMode,
        rest: StubMode,
        header_delay: Duration,
        body_chunk: usize,
        body_delay: Duration,
        requests: Arc<AtomicUsize>,
        fail_n: usize,
    }

    #[derive(Clone, Debug)]
    struct Logged {
        range: Option<String>,
        served: Option<(u64, u64)>,
    }

    fn spawn_stub(
        body: Vec<u8>,
        mode: StubMode,
        delay: Duration,
    ) -> (String, Arc<Mutex<Vec<Logged>>>) {
        spawn_stub_cfg(StubCfg {
            body,
            first: mode,
            rest: mode,
            header_delay: delay,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::new(AtomicUsize::new(0)),
            fail_n: 0,
        })
        .0
    }

    type StubHandle = (String, Arc<Mutex<Vec<Logged>>>);

    fn spawn_stub_cfg(cfg: StubCfg) -> (StubHandle, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let thread_log = Arc::clone(&log);
        let requests = Arc::clone(&cfg.requests);
        thread::spawn(move || {
            let body = Arc::new(cfg.body);
            let first = cfg.first;
            let rest = cfg.rest;
            let header_delay = cfg.header_delay;
            let body_chunk = cfg.body_chunk;
            let body_delay = cfg.body_delay;
            let fail_n = cfg.fail_n;
            let req_counter = cfg.requests;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let n = req_counter.fetch_add(1, Ordering::SeqCst);
                let mode = if fail_n > 0 {
                    if n < fail_n { first } else { rest }
                } else if n == 0 {
                    first
                } else {
                    rest
                };
                let body = Arc::clone(&body);
                let thread_log = Arc::clone(&thread_log);
                thread::spawn(move || {
                    handle_client(
                        stream,
                        &body,
                        mode,
                        header_delay,
                        body_chunk,
                        body_delay,
                        &thread_log,
                    );
                });
            }
        });
        ((format!("http://{addr}/audio.bin"), log), requests)
    }

    fn handle_client(
        mut stream: TcpStream,
        body: &[u8],
        mode: StubMode,
        delay: Duration,
        body_chunk: usize,
        body_delay: Duration,
        log: &Mutex<Vec<Logged>>,
    ) {
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let mut buf = Vec::new();
        let mut tmp = [0u8; 512];
        loop {
            let n = match stream.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n") || buf.len() > 32_768 {
                break;
            }
        }
        let request = String::from_utf8_lossy(&buf);
        let range = request.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.eq_ignore_ascii_case("range")).then(|| value.trim().to_string())
        });
        log.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(Logged {
                range: range.clone(),
                served: None,
            });
        if matches!(mode, StubMode::CloseEarly) {
            return;
        }
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        let (status, headers, slice, served) = match mode {
            StubMode::IgnoreRange => (
                "HTTP/1.1 200 OK",
                format!("Content-Length: {}\r\nContent-Type: application/octet-stream\r\n", body.len()),
                body,
                Some((0, body.len() as u64)),
            ),
            StubMode::InvalidRange => (
                "HTTP/1.1 206 Partial Content",
                "Content-Range: bytes junk\r\nContent-Length: 1\r\n".into(),
                &body[..1.min(body.len())],
                None,
            ),
            StubMode::MismatchTotal => {
                let total = MAX_BYTES as u64 + 1;
                (
                    "HTTP/1.1 206 Partial Content",
                    format!("Content-Range: bytes 0-0/{total}\r\nContent-Length: 1\r\n"),
                    &body[..1.min(body.len())],
                    None,
                )
            }
            StubMode::Multipart => (
                "HTTP/1.1 206 Partial Content",
                "Content-Type: multipart/byteranges; boundary=x\r\nContent-Range: bytes 0-0/2\r\nContent-Length: 1\r\n".into(),
                &body[..1.min(body.len())],
                None,
            ),
            StubMode::Range => {
                let (start, end) = parse_req_range(range.as_deref(), body.len());
                let slice = &body[start as usize..=end as usize];
                (
                    "HTTP/1.1 206 Partial Content",
                    format!(
                        "Content-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n",
                        body.len(),
                        slice.len()
                    ),
                    slice,
                    Some((start, end + 1)),
                )
            }
            StubMode::EmptyPartial => empty_partial(range.as_deref(), body),
            StubMode::WrongStart => wrong_start_partial(range.as_deref(), body),
            StubMode::TruncatedPartial => truncated_partial(range.as_deref(), body),
            StubMode::DropMid => {
                let (start, end) = parse_req_range(range.as_deref(), body.len());
                let slice = &body[start as usize..=end as usize];
                (
                    "HTTP/1.1 206 Partial Content",
                    format!(
                        "Content-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n",
                        body.len(),
                        slice.len()
                    ),
                    slice,
                    Some((start, end + 1)),
                )
            }
            StubMode::Forbidden => (
                "HTTP/1.1 403 Forbidden",
                "Content-Length: 0\r\n".into(),
                &body[..0],
                None,
            ),
            StubMode::HangRange => {
                thread::sleep(Duration::from_millis(250));
                let (start, end) = parse_req_range(range.as_deref(), body.len());
                let slice = &body[start as usize..=end as usize];
                (
                    "HTTP/1.1 206 Partial Content",
                    format!(
                        "Content-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n",
                        body.len(),
                        slice.len()
                    ),
                    slice,
                    Some((start, end + 1)),
                )
            }
            StubMode::CloseEarly => unreachable!(),
        };
        if let Some(served) = served
            && let Some(last) = log
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .last_mut()
        {
            last.served = Some(served);
        }
        let response = format!("{status}\r\n{headers}Connection: close\r\n\r\n");
        let _ = stream.write_all(response.as_bytes());
        if matches!(mode, StubMode::DropMid) {
            let take = 64.min(slice.len());
            let _ = stream.write_all(&slice[..take]);
            let _ = stream.flush();
            return;
        }
        if body_chunk == 0 || body_delay.is_zero() {
            let _ = stream.write_all(slice);
        } else {
            for part in slice.chunks(body_chunk) {
                let _ = stream.write_all(part);
                let _ = stream.flush();
                thread::sleep(body_delay);
            }
        }
        let _ = stream.flush();
    }

    fn range_headers(start: u64, end: u64, total: usize, body_len: usize) -> String {
        format!(
            "Content-Range: bytes {start}-{end}/{total}\r\nContent-Length: {body_len}\r\nContent-Type: application/octet-stream\r\n"
        )
    }

    fn empty_partial<'a>(
        range: Option<&str>,
        body: &'a [u8],
    ) -> (&'static str, String, &'a [u8], Option<(u64, u64)>) {
        let (start, end) = parse_req_range(range, body.len());
        (
            "HTTP/1.1 206 Partial Content",
            range_headers(start, end, body.len(), 0),
            &body[..0],
            Some((start, end + 1)),
        )
    }

    fn wrong_start_partial<'a>(
        range: Option<&str>,
        body: &'a [u8],
    ) -> (&'static str, String, &'a [u8], Option<(u64, u64)>) {
        let (req_start, _) = parse_req_range(range, body.len());
        let last = body.len().saturating_sub(1) as u64;
        let (start, end) = if req_start == 0 {
            let start = 1u64.min(last);
            (start, 50u64.min(last).max(start))
        } else {
            (0, req_start.saturating_sub(1).min(50))
        };
        let slice = &body[start as usize..=end as usize];
        (
            "HTTP/1.1 206 Partial Content",
            range_headers(start, end, body.len(), slice.len()),
            slice,
            Some((start, end + 1)),
        )
    }

    fn truncated_partial<'a>(
        range: Option<&str>,
        body: &'a [u8],
    ) -> (&'static str, String, &'a [u8], Option<(u64, u64)>) {
        let (start, end) = parse_req_range(range, body.len());
        let declared = &body[start as usize..=end as usize];
        let send = 16.min(declared.len().saturating_sub(1));
        let slice = &declared[..send];
        (
            "HTTP/1.1 206 Partial Content",
            range_headers(start, end, body.len(), slice.len()),
            slice,
            Some((start, end + 1)),
        )
    }

    fn parse_req_range(range: Option<&str>, len: usize) -> (u64, u64) {
        let last = len.saturating_sub(1) as u64;
        let Some(range) = range else {
            return (0, last);
        };
        let rest = range.trim().strip_prefix("bytes=").unwrap_or(range);
        let (start, end) = rest.split_once('-').unwrap_or(("0", ""));
        let start: u64 = start.parse().unwrap_or(0);
        let end: u64 = if end.is_empty() {
            last
        } else {
            end.parse().unwrap_or(last).min(last)
        };
        (start.min(last), end.max(start))
    }

    fn served_bytes(log: &Mutex<Vec<Logged>>) -> u64 {
        log.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter_map(|entry| entry.served.map(|(start, end)| end.saturating_sub(start)))
            .sum()
    }

    async fn wait_until(mut pred: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        pred()
    }

    async fn wait_nonzero_async(
        pcm: &mut crate::alternate::decode::PcmSource,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pcm.next().is_some_and(|sample| sample != 0.0) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        false
    }

    fn tiny_wav(samples: usize, rate: u32) -> Vec<u8> {
        let data_bytes = (samples * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..samples {
            let sample: i16 = if i % 64 == 0 { 900 } else { 0 };
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    fn mark_sample(wav: &mut [u8], sample: usize, value: i16) {
        let offset = 44 + sample * 2;
        if offset + 1 < wav.len() {
            wav[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
    }

    #[tokio::test]
    async fn range_206_writes_at_content_range_start() {
        let body = tiny_wav(200, 8_000);
        let (url, log) = spawn_stub(body.clone(), StubMode::Range, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        fetch_media_capped(&http, &url, &audio, &mut || {})
            .await
            .unwrap();
        assert!(audio.is_random_access() || audio.is_closed());
        assert_eq!(audio.copy_filled(0, 4).unwrap(), b"RIFF");
        assert_eq!(audio.content_length(), Some(body.len() as u64));
        let logged = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.iter().any(|entry| entry
                .range
                .as_deref()
                .is_some_and(|range| range.contains("bytes="))),
            "{logged:?}"
        );
    }

    #[tokio::test]
    async fn range_ignored_with_200_falls_back_to_sequential() {
        let body = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
        let (url, _) = spawn_stub(body.clone(), StubMode::IgnoreRange, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let mut ready = false;
        let result = fetch_media_capped(&http, &url, &audio, &mut || ready = true).await;
        assert!(!audio.is_random_access());
        assert_eq!(audio.len(), body.len());
        assert!(result.is_err() || audio.is_closed());
    }

    #[tokio::test]
    async fn invalid_and_mismatched_ranges_fail() {
        let body = vec![1u8; 64];
        let http = reqwest::Client::new();
        for mode in [
            StubMode::InvalidRange,
            StubMode::MismatchTotal,
            StubMode::Multipart,
        ] {
            let (url, _) = spawn_stub(body.clone(), mode, Duration::ZERO);
            let audio = SharedAudio::new(None).unwrap();
            let err = fetch_media_capped(&http, &url, &audio, &mut || {})
                .await
                .unwrap_err();
            assert!(
                err.contains("Couldn't")
                    || err.contains("refused")
                    || err.contains("large")
                    || err.contains("playable"),
                "{err}"
            );
            assert!(!audio.is_random_access() || audio.is_failed() || audio.error().is_some());
        }
    }

    #[tokio::test]
    async fn rapid_demands_keep_latest_and_abort_obsolete() {
        let mut body = vec![0u8; 4 * 1024 * 1024];
        let header = tiny_wav(200, 8_000);
        body[..header.len()].copy_from_slice(&header);
        let (url, log) = spawn_stub(body, StubMode::Range, Duration::from_millis(120));
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let url = url.clone();
            tokio::spawn(async move {
                let http = http;
                fetch_media(&http, &url, &[], &audio, &mut || {}).await
            })
        };
        assert!(
            wait_until(
                || audio.is_random_access() && audio.len() >= 1024,
                Duration::from_secs(3)
            )
            .await
        );
        let blocked = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(std::io::SeekFrom::Start(2_500_000)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        assert!(
            wait_until(
                || audio.current_demand().is_some_and(|d| d.start == 2_500_000),
                Duration::from_secs(2)
            )
            .await
        );
        let blocked2 = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(std::io::SeekFrom::Start(3_500_000)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        assert!(
            wait_until(
                || audio.current_demand().is_some_and(|d| d.start == 3_500_000),
                Duration::from_secs(2)
            )
            .await
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        audio.cancel();
        let _ = join_fetch(fetch).await;
        let _ = blocked.join();
        let _ = blocked2.join();
        let logged = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let ranges: Vec<_> = logged
            .iter()
            .filter_map(|entry| entry.range.clone())
            .collect();
        assert!(
            ranges.iter().any(|range| range.contains("3500000")
                || range.contains("3_500_000")
                || range.contains("bytes=3500000")),
            "latest demand missing: {ranges:?}"
        );
    }

    fn range_covers(range: &str, byte: u64) -> bool {
        let rest = range
            .trim()
            .strip_prefix("bytes=")
            .or_else(|| range.trim().strip_prefix("bytes "))
            .unwrap_or(range);
        let rest = rest.split('/').next().unwrap_or(rest);
        let (start, end) = rest.split_once('-').unwrap_or(("0", ""));
        let start: u64 = start.trim().parse().unwrap_or(0);
        let end: u64 = if end.trim().is_empty() {
            u64::MAX
        } else {
            end.trim().parse().unwrap_or(0)
        };
        start <= byte && byte <= end
    }

    #[tokio::test]
    async fn forward_seek_does_not_download_the_middle() {
        let rate = 16_000u32;
        let samples = (rate as usize) * 150;
        let seek_sample = (rate as usize) * 120;
        let mut wav = tiny_wav(samples, rate);
        for i in 0..(rate as usize) {
            mark_sample(&mut wav, i, 30_000);
        }
        for i in 0..(rate as usize * 2) {
            mark_sample(&mut wav, seek_sample + i, -30_000);
        }
        let total = wav.len() as u64;
        let seek_byte = 44u64 + seek_sample as u64 * 2;
        let (url, log) = spawn_stub(wav, StubMode::Range, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let url = url.clone();
            tokio::spawn(async move { fetch_media(&http, &url, &[], &audio, &mut || {}).await })
        };
        assert!(
            wait_until(
                || audio.is_random_access() && audio.len() > 44,
                Duration::from_secs(3)
            )
            .await
        );
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        assert!(
            wait_matching_sample(&mut pcm, |s| s > 0.5, Duration::from_secs(3)).is_some(),
            "expected pre-seek positive marker"
        );
        handle.seek(120_000);
        let mut sample = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut pulled = 0u32;
        while Instant::now() < deadline {
            match pcm.next() {
                Some(s) if s.abs() > 0.5 => {
                    sample = Some(s);
                    break;
                }
                Some(_) => {
                    pulled = pulled.saturating_add(1);
                    if pulled.is_multiple_of(512) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                None => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
        assert!(
            sample.is_some_and(|s| s < -0.5),
            "post-seek audio was not distinguishable from leftover PCM, got {sample:?} intervals={:?} ranges={:?}",
            audio.filled_intervals(),
            log.lock().unwrap_or_else(|p| p.into_inner()).clone()
        );
        let middle_start = 2_100_000u64;
        let middle_end = 2_400_000u64;
        assert!(
            middle_end < total,
            "fixture too small to leave a middle gap"
        );
        assert!(
            !audio.is_range_filled(middle_start, middle_end),
            "downloaded the middle: intervals={:?}",
            audio.filled_intervals()
        );
        assert!(served_bytes(&log) < total);
        let logged = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.iter().any(|entry| entry
                .range
                .as_deref()
                .is_some_and(|range| range_covers(range, seek_byte))),
            "HTTP Range did not cover decoder target {seek_byte}: {logged:?}"
        );
        audio.cancel();
        handle.stop();
        let _ = join_fetch(fetch).await;
    }

    #[tokio::test]
    async fn backward_seek_reuses_filled_bytes_without_new_request() {
        let body = vec![3u8; 3 * 1024 * 1024];
        let (url, log) = spawn_stub(body, StubMode::Range, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            tokio::spawn(async move { fetch_media(&http, &url, &[], &audio, &mut || {}).await })
        };
        assert!(
            wait_until(
                || audio.len() as u64 >= INITIAL_PREFIX / 2,
                Duration::from_secs(3)
            )
            .await
        );
        let before = log.lock().unwrap_or_else(|p| p.into_inner()).len();
        let mut reader = audio.reader();
        reader.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(reader.read(&mut buf).unwrap(), 8);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let logged = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
        for extra in logged.iter().skip(before) {
            if let Some(range) = extra.range.as_deref() {
                assert!(
                    !range_covers(range, 0),
                    "backward seek issued a new prefix request: {range}"
                );
            }
        }
        audio.cancel();
        let _ = join_fetch(fetch).await;
    }

    #[tokio::test]
    async fn cancel_stops_range_followup() {
        let body = vec![1u8; 3 * 1024 * 1024];
        let (url, _) = spawn_stub(body, StubMode::Range, Duration::from_millis(40));
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            tokio::spawn(async move { fetch_media(&http, &url, &[], &audio, &mut || {}).await })
        };
        assert!(wait_until(|| audio.is_random_access(), Duration::from_secs(3)).await);
        audio.cancel();
        let result = join_fetch(fetch).await;
        assert_eq!(result.unwrap_err(), "cancelled");
    }

    async fn assert_bad_206_fails(mode: StubMode, label: &str) {
        let mut body = tiny_wav(400, 8_000);
        body.resize(96 * 1024, 0);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), requests) = spawn_stub_cfg(StubCfg {
            body,
            first: mode,
            rest: mode,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 0,
        });
        let audio = SharedAudio::new(None).unwrap();
        let blocked = {
            let audio = audio.clone();
            thread::spawn(move || {
                let start = Instant::now();
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline
                    && audio.error().is_none()
                    && !audio.is_failed()
                    && !audio.is_random_access()
                {
                    thread::sleep(Duration::from_millis(1));
                }
                if audio.error().is_some() || audio.is_failed() {
                    return (Some("buffer failed".into()), start.elapsed());
                }
                let mut reader = audio.reader();
                let _ = reader.seek(std::io::SeekFrom::Start(80_000));
                let mut buf = [0u8; 1];
                let result = reader.read(&mut buf);
                (result.err().map(|e| e.to_string()), start.elapsed())
            })
        };
        let http = reqwest::Client::new();
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fetch_media(&http, &url, &[], &audio, &mut || {}),
        )
        .await;
        let elapsed = started.elapsed();
        let err = match result {
            Ok(Err(error)) => error,
            other => panic!("{label}: expected transport error, got {other:?} in {elapsed:?}"),
        };
        assert!(
            elapsed < Duration::from_secs(2),
            "{label}: tight-looped or stalled {elapsed:?} err={err}"
        );
        assert!(
            audio.error().is_some() || audio.is_failed(),
            "{label}: buffer did not fail, err={err}"
        );
        let count = requests.load(Ordering::SeqCst);
        assert!(
            count <= 3,
            "{label}: looped with {count} requests, err={err}"
        );
        let (read_err, blocked_for) = blocked.join().unwrap();
        assert!(
            blocked_for < Duration::from_secs(2),
            "{label}: reader waited {blocked_for:?}"
        );
        assert!(
            read_err.is_some(),
            "{label}: reader was not woken with an error"
        );
    }

    #[tokio::test]
    async fn empty_206_fails_without_loop() {
        assert_bad_206_fails(StubMode::EmptyPartial, "empty 206").await;
    }

    #[tokio::test]
    async fn wrong_start_206_fails_without_loop() {
        assert_bad_206_fails(StubMode::WrongStart, "wrong-start 206").await;
    }

    #[tokio::test]
    async fn truncated_206_fails_without_loop() {
        let mut body = tiny_wav(400, 8_000);
        body.resize(96 * 1024, 0);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), requests) = spawn_stub_cfg(StubCfg {
            body,
            first: StubMode::TruncatedPartial,
            rest: StubMode::EmptyPartial,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 0,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fetch_media(&http, &url, &[], &audio, &mut || {}),
        )
        .await;
        let elapsed = started.elapsed();
        let err = match result {
            Ok(Err(error)) => error,
            other => panic!("truncated 206: expected error, got {other:?} in {elapsed:?}"),
        };
        assert!(
            elapsed < Duration::from_secs(2),
            "truncated 206: looped {elapsed:?} err={err}"
        );
        assert!(
            audio.error().is_some() || audio.is_failed(),
            "truncated 206: buffer did not fail, err={err}"
        );
        let count = requests.load(Ordering::SeqCst);
        assert!(count <= 8, "truncated 206: looped with {count} requests");
    }

    #[tokio::test]
    async fn ready_before_full_initial_prefix() {
        let samples = 1_200_000;
        let wav = tiny_wav(samples, 8_000);
        assert!((wav.len() as u64) > INITIAL_PREFIX);
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body: wav,
            first: StubMode::Range,
            rest: StubMode::Range,
            header_delay: Duration::ZERO,
            body_chunk: 4_096,
            body_delay: Duration::from_millis(4),
            requests: Arc::new(AtomicUsize::new(0)),
            fail_n: 0,
        });
        let audio = SharedAudio::new(None).unwrap();
        let ready_at = Arc::new(Mutex::new(None::<u64>));
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let ready_at = Arc::clone(&ready_at);
            tokio::spawn(async move {
                fetch_media(&http, &url, &[], &audio, &mut || {
                    let mut slot = ready_at.lock().unwrap_or_else(|p| p.into_inner());
                    if slot.is_none() {
                        *slot = Some(audio.filled_bytes());
                    }
                })
                .await
            })
        };
        assert!(
            wait_until(
                || ready_at.lock().unwrap_or_else(|p| p.into_inner()).is_some(),
                Duration::from_secs(3)
            )
            .await,
            "Ready never fired"
        );
        let filled = ready_at.lock().unwrap_or_else(|p| p.into_inner()).unwrap();
        assert!(
            filled < INITIAL_PREFIX,
            "waited for the whole initial range before Ready: {filled}"
        );
        assert!(filled > 0);
        audio.cancel();
        let _ = join_fetch(fetch).await;
    }

    async fn fetch_fast(
        http: &reqwest::Client,
        url: &str,
        buffer: &SharedAudio,
        on_ready: &mut impl FnMut(),
        lookup: Option<&dyn MediaLookup>,
        video_id: Option<&str>,
        hint: Option<&FormatHint>,
    ) -> Result<(), String> {
        match tokio::time::timeout(
            Duration::from_secs(8),
            fetch_with(
                http,
                url.to_string(),
                Vec::new(),
                buffer,
                on_ready,
                FetchPolicy::for_test(),
                lookup,
                video_id,
                hint,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                buffer.cancel();
                Err("test fetch timed out".into())
            }
        }
    }

    async fn join_fetch(fetch: tokio::task::JoinHandle<Result<(), String>>) -> Result<(), String> {
        match tokio::time::timeout(Duration::from_secs(3), fetch).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(join.to_string()),
            Err(_) => Err("test fetch timed out".into()),
        }
    }

    async fn fetch_media_capped(
        http: &reqwest::Client,
        url: &str,
        buffer: &SharedAudio,
        on_ready: &mut impl FnMut(),
    ) -> Result<(), String> {
        match tokio::time::timeout(
            Duration::from_secs(8),
            fetch_media(http, url, &[], buffer, on_ready),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                buffer.cancel();
                Err("test fetch timed out".into())
            }
        }
    }

    fn playable_stream(url: &str, format: &str, mime: &str) -> AudioStream {
        AudioStream {
            url: url.into(),
            mime: Some(mime.into()),
            codec: None,
            format: Some(format.into()),
            bitrate: Some(96_000),
            video_only: false,
            quality: None,
            http_headers: Vec::new(),
        }
    }

    struct UrlLookup {
        url: String,
        hits: Arc<AtomicUsize>,
        format: &'static str,
        mime: &'static str,
    }

    impl MediaLookup for UrlLookup {
        fn search(
            &self,
            _query: &str,
        ) -> crate::alternate::provider::LookupFuture<Result<(Vec<Candidate>, bool), String>>
        {
            Box::pin(async { Err("search should not run on URL refresh".into()) })
        }

        fn streams(
            &self,
            _id: &str,
        ) -> crate::alternate::provider::LookupFuture<Result<(Vec<AudioStream>, bool), String>>
        {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let url = self.url.clone();
            let format = self.format;
            let mime = self.mime;
            Box::pin(async move { Ok((vec![playable_stream(&url, format, mime)], true)) })
        }
    }

    async fn fetch_compressed_206(bytes: &[u8], ext: &str, mime: &str) {
        let (url, _) = spawn_stub(bytes.to_vec(), StubMode::Range, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        fetch_media_capped(&http, &url, &audio, &mut || {})
            .await
            .unwrap();
        assert!(audio.is_random_access() || audio.is_closed());
        let hint = FormatHint::from_labels(Some(ext), Some(mime), None);
        let (mut pcm, handle) = spawn_decoder(audio, hint, 0).unwrap();
        assert!(
            wait_nonzero_sample(&mut pcm, Duration::from_secs(5)),
            "{ext} 206 produced no PCM from 0"
        );
        handle.stop();
    }

    #[tokio::test]
    async fn mp3_206_continuous_nonzero_from_zero() {
        fetch_compressed_206(TONE_MP3, "mp3", "audio/mpeg").await;
    }

    #[tokio::test]
    async fn m4a_faststart_206_continuous_nonzero_from_zero() {
        fetch_compressed_206(TONE_M4A, "m4a", "audio/mp4").await;
    }

    async fn stream_compressed_seek_via_stub(bytes: &[u8], ext: &str, mime: &str) {
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, log), _) = spawn_stub_cfg(StubCfg {
            body: bytes.to_vec(),
            first: StubMode::Range,
            rest: StubMode::Range,
            header_delay: Duration::ZERO,
            body_chunk: 1_024,
            body_delay: Duration::from_millis(3),
            requests: Arc::clone(&requests),
            fail_n: 0,
        });
        let audio = SharedAudio::new(None).unwrap();
        let ready = Arc::new(Mutex::new(false));
        let fetch = {
            let audio = audio.clone();
            let ready = Arc::clone(&ready);
            let http = reqwest::Client::new();
            tokio::spawn(async move {
                fetch_media(&http, &url, &[], &audio, &mut || {
                    *ready.lock().unwrap_or_else(|p| p.into_inner()) = true;
                })
                .await
            })
        };
        assert!(
            wait_until(
                || *ready.lock().unwrap_or_else(|p| p.into_inner()),
                Duration::from_secs(5),
            )
            .await,
            "{ext} 206 never became ready"
        );
        let hint = FormatHint::from_labels(Some(ext), Some(mime), None);
        let (mut pcm, handle) = spawn_decoder(audio.clone(), hint, 0).unwrap();
        assert!(
            wait_nonzero_async(&mut pcm, Duration::from_secs(5)).await,
            "{ext} produced no PCM from 0 before seek"
        );
        handle.seek(250);
        assert!(
            wait_nonzero_async(&mut pcm, Duration::from_secs(5)).await,
            "{ext} produced no PCM after seek; demand={:?} status={:?} ranges={:?}",
            audio.current_demand(),
            handle.status(),
            log.lock().unwrap_or_else(|p| p.into_inner()).clone()
        );
        assert_ne!(
            handle.status(),
            crate::alternate::decode::DecodeStatus::Failed
        );
        audio.cancel();
        handle.stop();
        let _ = join_fetch(fetch).await;
    }

    #[tokio::test]
    async fn mp3_206_seek_while_streaming_keeps_pcm() {
        stream_compressed_seek_via_stub(TONE_MP3, "mp3", "audio/mpeg").await;
    }

    #[tokio::test]
    async fn m4a_206_seek_while_streaming_keeps_pcm() {
        stream_compressed_seek_via_stub(TONE_M4A, "m4a", "audio/mp4").await;
    }

    #[tokio::test]
    async fn connection_drop_mid_range_then_recovers() {
        let body = tiny_wav(8_000, 8_000);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body: body.clone(),
            first: StubMode::DropMid,
            rest: StubMode::Range,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 1,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let url = url.clone();
            tokio::spawn(async move {
                fetch_fast(&http, &url, &audio, &mut || {}, None, None, None).await
            })
        };
        assert!(
            wait_until(|| audio.is_random_access(), Duration::from_secs(3)).await,
            "never became random-access"
        );
        let blocked = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                let _ = reader.seek(std::io::SeekFrom::Start(1_000));
                let mut buf = [0u8; 8];
                let _ = reader.read(&mut buf);
            })
        };
        assert!(
            wait_until(
                || audio.copy_filled(0, 4).as_deref() == Some(&b"RIFF"[..])
                    && (audio.is_closed() || audio.filled_bytes() >= 1_008),
                Duration::from_secs(3)
            )
            .await,
            "drop did not resume; requests={} filled={} intervals={:?}",
            requests.load(Ordering::SeqCst),
            audio.filled_bytes(),
            audio.filled_intervals()
        );
        assert!(!audio.is_failed());
        assert!(requests.load(Ordering::SeqCst) >= 2);
        audio.cancel();
        let _ = join_fetch(fetch).await;
        let _ = blocked.join();
    }

    #[tokio::test]
    async fn timeout_then_recovers() {
        let body = tiny_wav(800, 8_000);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body: body.clone(),
            first: StubMode::HangRange,
            rest: StubMode::Range,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 1,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        fetch_fast(&http, &url, &audio, &mut || {}, None, None, None)
            .await
            .unwrap();
        assert_eq!(audio.copy_filled(0, 4).unwrap(), b"RIFF");
        assert!(!audio.is_failed());
    }

    #[tokio::test]
    async fn offline_for_multiple_backoff_intervals() {
        let body = tiny_wav(800, 8_000);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body: body.clone(),
            first: StubMode::CloseEarly,
            rest: StubMode::Range,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 3,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let started = Instant::now();
        fetch_fast(&http, &url, &audio, &mut || {}, None, None, None)
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(30));
        assert_eq!(audio.copy_filled(0, 4).unwrap(), b"RIFF");
        assert!(requests.load(Ordering::SeqCst) >= 4);
        assert!(!audio.is_failed());
    }

    #[tokio::test]
    async fn url_403_then_refreshed_url() {
        let body = tiny_wav(800, 8_000);
        let (bad, _) = spawn_stub(body.clone(), StubMode::Forbidden, Duration::ZERO);
        let (good, _) = spawn_stub(body.clone(), StubMode::Range, Duration::ZERO);
        let hits = Arc::new(AtomicUsize::new(0));
        let lookup = UrlLookup {
            url: good,
            hits: Arc::clone(&hits),
            format: "wav",
            mime: "audio/wav",
        };
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let hint = FormatHint::from_labels(Some("wav"), Some("audio/wav"), None);
        fetch_fast(
            &http,
            &bad,
            &audio,
            &mut || {},
            Some(&lookup),
            Some("dQw4w9WgXcQ"),
            Some(&hint),
        )
        .await
        .unwrap();
        assert!(hits.load(Ordering::SeqCst) >= 1);
        assert_eq!(audio.copy_filled(0, 4).unwrap(), b"RIFF");
        assert!(!audio.is_failed());
    }

    #[tokio::test]
    async fn stop_cancels_retry_immediately() {
        let body = tiny_wav(800, 8_000);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body,
            first: StubMode::CloseEarly,
            rest: StubMode::CloseEarly,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 100,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let url = url.clone();
            tokio::spawn(async move {
                fetch_fast(&http, &url, &audio, &mut || {}, None, None, None).await
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        audio.cancel();
        let started = Instant::now();
        let err = join_fetch(fetch).await.unwrap_err();
        assert_eq!(err, "cancelled");
        assert!(started.elapsed() < Duration::from_millis(400));
    }

    #[tokio::test]
    async fn seek_during_outage_changes_priority() {
        let mut body = vec![0u8; 4 * 1024 * 1024];
        let header = tiny_wav(200, 8_000);
        body[..header.len()].copy_from_slice(&header);
        let (url, log) = spawn_stub(body, StubMode::DropMid, Duration::ZERO);
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            let url = url.clone();
            tokio::spawn(async move {
                fetch_fast(&http, &url, &audio, &mut || {}, None, None, None).await
            })
        };
        assert!(
            wait_until(|| audio.is_random_access(), Duration::from_secs(3)).await,
            "never became random-access"
        );
        let blocked = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(std::io::SeekFrom::Start(3_000_000)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        assert!(
            wait_until(
                || audio.current_demand().is_some_and(|d| d.start == 3_000_000),
                Duration::from_secs(2)
            )
            .await
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
        audio.cancel();
        let _ = join_fetch(fetch).await;
        let _ = blocked.join();
        let logged = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(
            logged.iter().any(|entry| entry
                .range
                .as_deref()
                .is_some_and(|range| range.contains("3000000"))),
            "seek during outage did not change Range: {logged:?}"
        );
    }

    #[tokio::test]
    async fn sequential_200_at_nonzero_reconnects_from_zero() {
        let samples = 1_200_000;
        let wav = tiny_wav(samples, 8_000);
        assert!((wav.len() as u64) > INITIAL_PREFIX);
        let requests = Arc::new(AtomicUsize::new(0));
        let ((url, _), _) = spawn_stub_cfg(StubCfg {
            body: wav.clone(),
            first: StubMode::Range,
            rest: StubMode::IgnoreRange,
            header_delay: Duration::ZERO,
            body_chunk: 0,
            body_delay: Duration::ZERO,
            requests: Arc::clone(&requests),
            fail_n: 0,
        });
        let audio = SharedAudio::new(None).unwrap();
        let http = reqwest::Client::new();
        let fetch = {
            let audio = audio.clone();
            tokio::spawn(async move { fetch_media_capped(&http, &url, &audio, &mut || {}).await })
        };
        assert!(
            wait_until(
                || audio.is_random_access() && audio.len() as u64 > 44,
                Duration::from_secs(3)
            )
            .await,
            "prefix never arrived"
        );
        let blocked = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                let _ = reader.seek(std::io::SeekFrom::Start(INITIAL_PREFIX + 4096));
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        assert!(
            wait_until(
                || requests.load(Ordering::SeqCst) >= 2
                    && audio.copy_filled(0, 4).as_deref() == Some(&b"RIFF"[..]),
                Duration::from_secs(3)
            )
            .await,
            "ignored Range never reconnected from 0; requests={} intervals={:?}",
            requests.load(Ordering::SeqCst),
            audio.filled_intervals()
        );
        assert!(!audio.is_failed());
        audio.cancel();
        let _ = join_fetch(fetch).await;
        let _ = blocked.join();
    }
}
