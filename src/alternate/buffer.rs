//! Sparse compressed-audio buffer. Decoder thread reads; fetch task writes.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

pub const MAX_BYTES: usize = 40 * 1024 * 1024;
pub(crate) const DEMAND_WINDOW: u64 = 256 * 1024;
/// Wakes a blocked reader so the decoder can rebuild. Not EOF and not cancel.
pub(crate) const SEEK_RETARGET: &str = "seek-retarget";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Option<Self> {
        (end > start).then_some(Self { start, end })
    }

    pub fn contains(self, pos: u64) -> bool {
        pos >= self.start && pos < self.end
    }

    pub fn overlaps_or_near(self, other: Self, gap: u64) -> bool {
        let a_end = self.end.saturating_add(gap);
        let b_end = other.end.saturating_add(gap);
        self.start < b_end && other.start < a_end
    }
}

#[derive(Clone, Debug)]
enum Status {
    Open,
    Closed,
    Failed(String),
    Cancelled,
}

struct Inner {
    data: Vec<u8>,
    filled: Vec<(u64, u64)>,
    content_length: Option<u64>,
    random_access: bool,
    status: Status,
    demand: Option<ByteRange>,
    demand_gen: u64,
    read_epoch: u64,
}

struct Shared {
    lock: Mutex<Inner>,
    wait: Condvar,
    notify: Notify,
}

#[derive(Clone)]
pub struct SharedAudio {
    shared: Arc<Shared>,
    limit: usize,
}

impl SharedAudio {
    pub fn new(content_length: Option<u64>) -> Result<Self, String> {
        Self::with_limit(content_length, MAX_BYTES)
    }

    pub fn with_limit(content_length: Option<u64>, limit: usize) -> Result<Self, String> {
        if let Some(length) = content_length
            && length as usize > limit
        {
            return Err("Matched audio is too large to play.".into());
        }
        let mut data = Vec::new();
        if let Some(length) = content_length {
            let reserve = (length as usize).min(limit);
            let _ = data.try_reserve(reserve);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                lock: Mutex::new(Inner {
                    data,
                    filled: Vec::new(),
                    content_length,
                    random_access: false,
                    status: Status::Open,
                    demand: None,
                    demand_gen: 0,
                    read_epoch: 0,
                }),
                wait: Condvar::new(),
                notify: Notify::new(),
            }),
            limit,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared
            .lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn wake_all(&self) {
        self.shared.wait.notify_all();
        self.shared.notify.notify_waiters();
    }

    pub fn set_content_length(&self, length: u64) -> Result<(), String> {
        if length as usize > self.limit {
            self.fail("Matched audio is too large to play.");
            return Err("Matched audio is too large to play.".into());
        }
        let mut inner = self.lock();
        match inner.status {
            Status::Cancelled => return Err("cancelled".into()),
            Status::Failed(ref message) => return Err(message.clone()),
            Status::Closed => return Ok(()),
            Status::Open => {}
        }
        if let Some(old) = inner.content_length
            && old != length
        {
            inner.status = Status::Failed("Couldn't read matched audio.".into());
            drop(inner);
            self.wake_all();
            return Err("Couldn't read matched audio.".into());
        }
        if filled_bytes(&inner.filled) > length || prefix_len(&inner) as u64 > length {
            inner.status = Status::Failed("Matched audio is too large to play.".into());
            drop(inner);
            self.wake_all();
            return Err("Matched audio is too large to play.".into());
        }
        inner.content_length = Some(length);
        drop(inner);
        self.wake_all();
        Ok(())
    }

    /// Known total plus random access. Resizes the store; holes stay invalid.
    pub fn enable_random_access(&self, total: u64) -> Result<(), String> {
        self.set_content_length(total)?;
        let mut inner = self.lock();
        match inner.status {
            Status::Cancelled => return Err("cancelled".into()),
            Status::Failed(ref message) => return Err(message.clone()),
            Status::Closed => return Ok(()),
            Status::Open => {}
        }
        if inner.random_access {
            return Ok(());
        }
        let total_usize = total as usize;
        if inner.data.len() > total_usize {
            inner.status = Status::Failed("Matched audio is too large to play.".into());
            drop(inner);
            self.wake_all();
            return Err("Matched audio is too large to play.".into());
        }
        if inner.data.capacity() < total_usize {
            let extra = total_usize - inner.data.len();
            if inner.data.try_reserve(extra).is_err() {
                inner.status = Status::Failed("Matched audio is too large to play.".into());
                drop(inner);
                self.wake_all();
                return Err("Matched audio is too large to play.".into());
            }
        }
        inner.data.resize(total_usize, 0);
        inner.random_access = true;
        drop(inner);
        self.wake_all();
        Ok(())
    }

    pub fn append(&self, chunk: &[u8]) -> Result<(), String> {
        if chunk.is_empty() {
            return Ok(());
        }
        let offset = {
            let inner = self.lock();
            match &inner.status {
                Status::Cancelled => return Err("cancelled".into()),
                Status::Failed(message) => return Err(message.clone()),
                Status::Closed => return Err("download already closed".into()),
                Status::Open => prefix_len(&inner) as u64,
            }
        };
        self.write_at(offset, chunk)
    }

    pub fn write_at(&self, offset: u64, chunk: &[u8]) -> Result<(), String> {
        if chunk.is_empty() {
            return Ok(());
        }
        let Some(end) = offset.checked_add(chunk.len() as u64) else {
            self.fail("Matched audio is too large to play.");
            return Err("Matched audio is too large to play.".into());
        };
        if end as usize > self.limit {
            self.fail("Matched audio is too large to play.");
            return Err("Matched audio is too large to play.".into());
        }
        let mut inner = self.lock();
        match &inner.status {
            Status::Cancelled => return Err("cancelled".into()),
            Status::Failed(message) => return Err(message.clone()),
            Status::Closed => return Err("download already closed".into()),
            Status::Open => {}
        }
        if let Some(length) = inner.content_length
            && end > length
        {
            inner.status = Status::Failed("Matched audio is too large to play.".into());
            drop(inner);
            self.wake_all();
            return Err("Matched audio is too large to play.".into());
        }
        let prefix = prefix_len(&inner) as u64;
        if !inner.random_access && offset > prefix {
            inner.status = Status::Failed("Couldn't read matched audio.".into());
            drop(inner);
            self.wake_all();
            return Err("Couldn't read matched audio.".into());
        }
        if (end as usize) > inner.data.len() {
            if inner.random_access {
                inner.status = Status::Failed("Couldn't read matched audio.".into());
                drop(inner);
                self.wake_all();
                return Err("Couldn't read matched audio.".into());
            }
            inner.data.resize(end as usize, 0);
        }
        let start = offset as usize;
        inner.data[start..start + chunk.len()].copy_from_slice(chunk);
        insert_filled(&mut inner.filled, offset, end);
        if let Some(demand) = inner.demand
            && first_hole(&inner.filled, demand.start, demand.end).is_none()
        {
            inner.demand = None;
        }
        drop(inner);
        self.wake_all();
        Ok(())
    }

    pub fn close(&self) {
        let mut inner = self.lock();
        if matches!(inner.status, Status::Open) {
            inner.status = Status::Closed;
            drop(inner);
            self.wake_all();
        }
    }

    pub fn fail(&self, message: impl Into<String>) {
        let mut inner = self.lock();
        if matches!(inner.status, Status::Open) {
            inner.status = Status::Failed(message.into());
            drop(inner);
            self.wake_all();
        }
    }

    pub fn cancel(&self) {
        let mut inner = self.lock();
        if !matches!(inner.status, Status::Cancelled) {
            inner.status = Status::Cancelled;
            drop(inner);
            self.wake_all();
        }
    }

    pub fn len(&self) -> usize {
        prefix_len(&self.lock())
    }

    pub fn filled_bytes(&self) -> u64 {
        filled_bytes(&self.lock().filled)
    }

    pub fn content_length(&self) -> Option<u64> {
        self.lock().content_length
    }

    pub fn is_random_access(&self) -> bool {
        self.lock().random_access
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.lock().status, Status::Closed)
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self.lock().status, Status::Cancelled)
    }

    pub fn is_open(&self) -> bool {
        matches!(self.lock().status, Status::Open)
    }

    #[allow(dead_code)]
    pub fn is_failed(&self) -> bool {
        matches!(self.lock().status, Status::Failed(_))
    }

    pub fn error(&self) -> Option<String> {
        match &self.lock().status {
            Status::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }

    pub fn current_demand(&self) -> Option<ByteRange> {
        self.lock().demand
    }

    pub(crate) fn read_epoch(&self) -> u64 {
        self.lock().read_epoch
    }

    pub(crate) fn abandon_reads(&self) {
        let mut inner = self.lock();
        inner.read_epoch = inner.read_epoch.wrapping_add(1);
        drop(inner);
        self.wake_all();
    }

    #[cfg(test)]
    pub fn demand_gen(&self) -> u64 {
        self.lock().demand_gen
    }

    pub fn first_hole(&self, start: u64, end: u64) -> Option<ByteRange> {
        let inner = self.lock();
        first_hole(&inner.filled, start, end)
    }

    #[cfg(test)]
    pub fn is_range_filled(&self, start: u64, end: u64) -> bool {
        if end <= start {
            return true;
        }
        let inner = self.lock();
        first_hole(&inner.filled, start, end).is_none()
    }

    pub fn copy_filled(&self, start: u64, end: u64) -> Option<Vec<u8>> {
        if end < start {
            return None;
        }
        let inner = self.lock();
        if first_hole(&inner.filled, start, end).is_some() {
            return None;
        }
        let start_u = start as usize;
        let end_u = (end as usize).min(inner.data.len());
        if start_u > inner.data.len() || start_u > end_u {
            return Some(Vec::new());
        }
        Some(inner.data[start_u..end_u].to_vec())
    }

    #[cfg(test)]
    pub fn filled_intervals(&self) -> Vec<(u64, u64)> {
        self.lock().filled.clone()
    }

    pub fn reader(&self) -> GrowableReader {
        GrowableReader {
            audio: self.clone(),
            pos: 0,
            epoch: self.read_epoch(),
        }
    }

    pub async fn notified(&self) {
        self.shared.notify.notified().await;
    }

    pub fn wait_for_change(&self, seen_len: usize, timeout: Duration) -> BufferWait {
        let mut inner = self.lock();
        loop {
            if let Some(wait) = classify_wait(&inner, seen_len) {
                return wait;
            }
            let waited = self.shared.wait.wait_timeout(inner, timeout);
            let (guard, result) = match waited {
                Ok(pair) => pair,
                Err(poison) => poison.into_inner(),
            };
            inner = guard;
            if result.timed_out() {
                return classify_wait(&inner, seen_len)
                    .unwrap_or(BufferWait::Unchanged(prefix_len(&inner)));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferWait {
    Grew(usize),
    Unchanged(usize),
    Closed(usize),
    Failed,
    Cancelled,
}

fn classify_wait(inner: &Inner, seen_len: usize) -> Option<BufferWait> {
    match &inner.status {
        Status::Cancelled => Some(BufferWait::Cancelled),
        Status::Failed(_) => Some(BufferWait::Failed),
        Status::Closed => Some(BufferWait::Closed(prefix_len(inner))),
        Status::Open if prefix_len(inner) > seen_len => Some(BufferWait::Grew(prefix_len(inner))),
        Status::Open => None,
    }
}

fn prefix_len(inner: &Inner) -> usize {
    match inner.filled.first() {
        Some((0, end)) => *end as usize,
        _ => 0,
    }
}

fn filled_bytes(filled: &[(u64, u64)]) -> u64 {
    filled
        .iter()
        .map(|(start, end)| end.saturating_sub(*start))
        .sum()
}

fn insert_filled(filled: &mut Vec<(u64, u64)>, mut start: u64, mut end: u64) {
    if end <= start {
        return;
    }
    let mut i = 0;
    while i < filled.len() {
        let (span_start, span_end) = filled[i];
        if span_end < start {
            i += 1;
            continue;
        }
        if span_start > end {
            break;
        }
        start = start.min(span_start);
        end = end.max(span_end);
        filled.remove(i);
    }
    filled.insert(i, (start, end));
}

fn first_hole(filled: &[(u64, u64)], start: u64, end: u64) -> Option<ByteRange> {
    if end <= start {
        return None;
    }
    let mut cursor = start;
    for (span_start, span_end) in filled {
        if *span_end <= cursor {
            continue;
        }
        if *span_start > cursor {
            return ByteRange::new(cursor, (*span_start).min(end));
        }
        cursor = *span_end;
        if cursor >= end {
            return None;
        }
    }
    ByteRange::new(cursor, end)
}

fn available_from(filled: &[(u64, u64)], pos: u64) -> u64 {
    for (start, end) in filled {
        if pos >= *start && pos < *end {
            return end - pos;
        }
    }
    0
}

fn publish_demand(inner: &mut Inner, notify: &Notify, pos: u64) {
    let mut end = pos.saturating_add(DEMAND_WINDOW);
    if let Some(total) = inner.content_length {
        end = end.min(total);
    }
    let Some(range) = ByteRange::new(pos, end) else {
        return;
    };
    if let Some(demand) = inner.demand
        && demand.contains(pos)
    {
        return;
    }
    inner.demand = Some(range);
    inner.demand_gen = inner.demand_gen.wrapping_add(1);
    notify.notify_waiters();
}

pub struct GrowableReader {
    audio: SharedAudio,
    pos: u64,
    epoch: u64,
}

impl GrowableReader {
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
}

fn seek_retarget() -> io::Error {
    // Not Interrupted: std and Symphonia retry that kind forever, and this
    // reader keeps the old epoch so a retry never makes progress.
    io::Error::other(SEEK_RETARGET)
}

fn cancelled_error() -> io::Error {
    io::Error::other("cancelled")
}

fn wait_more<'a>(
    shared: &'a Shared,
    inner: std::sync::MutexGuard<'a, Inner>,
) -> std::sync::MutexGuard<'a, Inner> {
    shared
        .wait
        .wait(inner)
        .unwrap_or_else(|poison| poison.into_inner())
}

impl Read for GrowableReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut inner = self.audio.lock();
        loop {
            match &inner.status {
                Status::Cancelled => {
                    return Err(cancelled_error());
                }
                Status::Failed(message) => {
                    return Err(io::Error::other(message.clone()));
                }
                Status::Open | Status::Closed => {
                    if inner.read_epoch != self.epoch {
                        return Err(seek_retarget());
                    }
                    let pos = self.pos;
                    if let Some(total) = inner.content_length
                        && pos >= total
                    {
                        return Ok(0);
                    }
                    let available = available_from(&inner.filled, pos);
                    if available > 0 {
                        let start = pos as usize;
                        let n = (available as usize)
                            .min(buf.len())
                            .min(inner.data.len().saturating_sub(start));
                        if n == 0 {
                            return Ok(0);
                        }
                        buf[..n].copy_from_slice(&inner.data[start..start + n]);
                        self.pos += n as u64;
                        return Ok(n);
                    }
                    if matches!(inner.status, Status::Closed) {
                        return Ok(0);
                    }
                    publish_demand(&mut inner, &self.audio.shared.notify, pos);
                    inner = wait_more(&self.audio.shared, inner);
                }
            }
        }
    }
}

impl Seek for GrowableReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let inner = self.audio.lock();
        match &inner.status {
            Status::Cancelled => Err(cancelled_error()),
            Status::Failed(message) => Err(io::Error::other(message.clone())),
            Status::Open | Status::Closed => {
                if inner.read_epoch != self.epoch {
                    return Err(seek_retarget());
                }
                let end_base = if inner.random_access {
                    inner
                        .content_length
                        .unwrap_or_else(|| prefix_len(&inner) as u64)
                } else {
                    prefix_len(&inner) as u64
                };
                let target = match from {
                    SeekFrom::Start(offset) => offset,
                    SeekFrom::Current(delta) => {
                        let pos = i128::from(self.pos) + i128::from(delta);
                        if pos < 0 {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "seek before start",
                            ));
                        }
                        pos as u64
                    }
                    SeekFrom::End(delta) => {
                        let pos = i128::from(end_base) + i128::from(delta);
                        if pos < 0 {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "seek before start",
                            ));
                        }
                        pos as u64
                    }
                };
                self.pos = target;
                Ok(self.pos)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn append_read_seek_and_eof() {
        let audio = SharedAudio::new(Some(8)).unwrap();
        audio.append(b"abcd").unwrap();
        let mut reader = audio.reader();
        let mut buf = [0u8; 2];
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf, b"ab");
        assert_eq!(reader.seek(SeekFrom::Start(1)).unwrap(), 1);
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf, b"bc");
        audio.append(b"efgh").unwrap();
        assert_eq!(reader.seek(SeekFrom::Start(4)).unwrap(), 4);
        let mut rest = Vec::new();
        let mut tmp = [0u8; 8];
        let n = reader.read(&mut tmp).unwrap();
        rest.extend_from_slice(&tmp[..n]);
        assert_eq!(&rest, b"efgh");
        audio.close();
        assert_eq!(reader.read(&mut tmp).unwrap(), 0);
        assert_eq!(audio.len(), 8);
    }

    #[test]
    fn size_cap_rejects_overflow() {
        let audio = SharedAudio::with_limit(None, 8).unwrap();
        audio.append(b"12345678").unwrap();
        assert_eq!(
            audio.append(b"x").unwrap_err(),
            "Matched audio is too large to play."
        );
        assert!(audio.is_failed());
        assert!(SharedAudio::new(Some((MAX_BYTES as u64) + 1)).is_err());
    }

    #[test]
    fn cancel_wakes_blocked_reader() {
        let audio = SharedAudio::new(None).unwrap();
        let mut reader = audio.reader();
        let handle = {
            let audio = audio.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(30));
                audio.cancel();
            })
        };
        let mut buf = [0u8; 4];
        let start = Instant::now();
        let err = reader.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
        handle.join().unwrap();
    }

    #[test]
    fn seek_rebuilds_from_prefix() {
        let audio = SharedAudio::new(None).unwrap();
        audio.append(b"hello world").unwrap();
        audio.close();
        let mut reader = audio.reader();
        let mut buf = [0u8; 5];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn fail_wakes_reader_with_message() {
        let audio = SharedAudio::new(None).unwrap();
        let mut reader = audio.reader();
        thread::spawn({
            let audio = audio.clone();
            move || {
                thread::sleep(Duration::from_millis(20));
                audio.fail("nope");
            }
        });
        let mut buf = [0u8; 1];
        let err = reader.read(&mut buf).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn wait_for_change_sees_growth_and_close() {
        let audio = SharedAudio::new(None).unwrap();
        audio.append(b"ab").unwrap();
        assert!(matches!(
            audio.wait_for_change(0, Duration::from_millis(10)),
            BufferWait::Grew(2)
        ));
        let handle = {
            let audio = audio.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                audio.append(b"cd").unwrap();
            })
        };
        assert!(matches!(
            audio.wait_for_change(2, Duration::from_secs(1)),
            BufferWait::Grew(4)
        ));
        handle.join().unwrap();
        audio.close();
        assert!(matches!(
            audio.wait_for_change(4, Duration::from_millis(10)),
            BufferWait::Closed(4)
        ));
    }

    #[test]
    fn sparse_writes_coalesce_and_count_filled_bytes() {
        let audio = SharedAudio::with_limit(Some(32), 32).unwrap();
        audio.enable_random_access(32).unwrap();
        audio.write_at(10, b"abcd").unwrap();
        audio.write_at(12, b"CDEF").unwrap();
        audio.write_at(8, b"xxab").unwrap();
        audio.write_at(20, b"zz").unwrap();
        audio.write_at(18, b"yyzz").unwrap();
        assert_eq!(audio.filled_intervals(), vec![(8, 16), (18, 22)]);
        assert_eq!(audio.filled_bytes(), 12);
        assert_eq!(audio.len(), 0);
        assert!(audio.is_range_filled(8, 16));
        assert!(!audio.is_range_filled(8, 19));
        audio.write_at(16, b"ww").unwrap();
        assert_eq!(audio.filled_intervals(), vec![(8, 22)]);
        assert_eq!(audio.filled_bytes(), 14);
        audio.write_at(10, b"abcd").unwrap();
        assert_eq!(audio.filled_bytes(), 14);
        assert!(audio.enable_random_access(33).is_err());
        assert!(SharedAudio::with_limit(Some(8), 4).is_err());
    }

    #[test]
    fn sequential_write_at_hole_is_rejected() {
        let audio = SharedAudio::with_limit(None, 32).unwrap();
        audio.append(b"abcd").unwrap();
        assert!(audio.write_at(10, b"x").is_err());
        assert!(audio.is_failed());
    }

    #[test]
    fn seek_into_hole_is_immediate_read_demands_then_resumes() {
        let audio = SharedAudio::with_limit(Some(80), 80).unwrap();
        audio.enable_random_access(80).unwrap();
        audio.write_at(0, &[1u8; 8]).unwrap();
        let mut reader = audio.reader();
        let start = Instant::now();
        assert_eq!(reader.seek(SeekFrom::Start(50)).unwrap(), 50);
        assert!(start.elapsed() < Duration::from_millis(50));
        assert!(audio.current_demand().is_none());

        let feeder = {
            let audio = audio.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    if let Some(demand) = audio.current_demand() {
                        assert_eq!(demand.start, 50);
                        assert!(demand.end > 50);
                        audio.write_at(50, &[9u8; 10]).unwrap();
                        return;
                    }
                    assert!(Instant::now() < deadline, "demand was not published");
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 4);
        assert_eq!(buf, [9, 9, 9, 9]);
        feeder.join().unwrap();
    }

    #[test]
    fn cancel_and_fail_wake_hole_read() {
        let audio = SharedAudio::with_limit(Some(40), 40).unwrap();
        audio.enable_random_access(40).unwrap();
        let mut reader = audio.reader();
        reader.seek(SeekFrom::Start(20)).unwrap();
        let handle = {
            let audio = audio.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                audio.cancel();
            })
        };
        let mut buf = [0u8; 2];
        let err = reader.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("cancelled"));
        handle.join().unwrap();

        let audio = SharedAudio::with_limit(Some(40), 40).unwrap();
        audio.enable_random_access(40).unwrap();
        let mut reader = audio.reader();
        reader.seek(SeekFrom::Start(20)).unwrap();
        thread::spawn({
            let audio = audio.clone();
            move || {
                thread::sleep(Duration::from_millis(20));
                audio.fail("hole failed");
            }
        });
        let err = reader.read(&mut buf).unwrap_err();
        assert!(err.to_string().contains("hole failed"));
    }

    #[test]
    fn latest_demand_replaces_obsolete_range() {
        let total = 1_000_000u64;
        let audio = SharedAudio::with_limit(Some(total), total as usize).unwrap();
        audio.enable_random_access(total).unwrap();
        let waiter = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(SeekFrom::Start(100)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while audio.current_demand().map(|d| d.start) != Some(100) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let seen_gen = audio.demand_gen();
        let waiter2 = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(SeekFrom::Start(800_000)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        while audio.current_demand().map(|d| d.start) != Some(800_000) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(audio.demand_gen() > seen_gen);
        audio.cancel();
        let _ = waiter.join();
        let _ = waiter2.join();
    }

    #[test]
    fn backward_seek_into_filled_range_does_not_demand() {
        let audio = SharedAudio::with_limit(Some(64), 64).unwrap();
        audio.enable_random_access(64).unwrap();
        audio.write_at(0, &[3u8; 32]).unwrap();
        let mut reader = audio.reader();
        reader.seek(SeekFrom::Start(40)).unwrap();
        let waiter = {
            let audio = audio.clone();
            thread::spawn(move || {
                let mut reader = audio.reader();
                reader.seek(SeekFrom::Start(40)).unwrap();
                let mut buf = [0u8; 1];
                let _ = reader.read(&mut buf);
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while audio.current_demand().map(|d| d.start) != Some(40) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        audio.write_at(40, &[4u8; 8]).unwrap();
        let _ = waiter.join();
        let mut back = audio.reader();
        back.seek(SeekFrom::Start(0)).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(back.read(&mut buf).unwrap(), 4);
        assert_eq!(buf, [3, 3, 3, 3]);
        assert_ne!(audio.current_demand().map(|demand| demand.start), Some(0));
    }

    #[test]
    fn abandon_reads_wakes_blocked_reader_with_retarget() {
        let audio = SharedAudio::with_limit(Some(40), 40).unwrap();
        audio.enable_random_access(40).unwrap();
        let mut reader = audio.reader();
        reader.seek(SeekFrom::Start(20)).unwrap();
        let handle = {
            let audio = audio.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                audio.abandon_reads();
            })
        };
        let mut buf = [0u8; 2];
        let start = Instant::now();
        let err = reader.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains(SEEK_RETARGET));
        assert!(start.elapsed() < Duration::from_secs(2));
        handle.join().unwrap();
        assert_ne!(audio.read_epoch(), 0);
        let mut next = audio.reader();
        next.seek(SeekFrom::Start(0)).unwrap();
        audio.write_at(0, &[7u8; 4]).unwrap();
        assert_eq!(next.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf, &[7, 7]);
    }
}
