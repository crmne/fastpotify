//! Container probes for progressive play.
//!
//! Startup is format-aware: decode begins when headers and the first packets
//! are present. There is no fixed time buffer. `moov` at the end of an M4A
//! still needs a complete body, or a compact tail that actually contains `moov`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inspect {
    NeedMore,
    Mp3,
    Wav,
    FastStart,
    MoovAtEnd,
    Invalid,
}

pub const TAIL_PREFETCH: u64 = 1024 * 1024;

pub fn playback_ready(prefix: &[u8], downloaded: usize, complete: bool) -> bool {
    match inspect(prefix) {
        Inspect::NeedMore => complete && downloaded > 0,
        Inspect::Invalid => false,
        Inspect::MoovAtEnd => complete,
        Inspect::Mp3 | Inspect::Wav | Inspect::FastStart => true,
    }
}

/// Fast-start / MP3 / WAV stay prefix-ready. Late `moov` is ready when the
/// compact tail contains a `moov` atom, or when the body is complete.
pub fn playback_ready_sparse(prefix: &[u8], tail: &[u8], complete: bool) -> bool {
    if playback_ready(prefix, prefix.len(), complete) {
        return true;
    }
    inspect(prefix) == Inspect::MoovAtEnd && (complete || has_moov_atom(tail))
}

/// Bounded tail that may hold a late `moov`. None when the span is not compact.
pub fn tail_prefetch(prefix: &[u8], total: u64) -> Option<(u64, u64)> {
    if inspect(prefix) != Inspect::MoovAtEnd || total <= TAIL_PREFETCH {
        return None;
    }
    let bound = TAIL_PREFETCH.min(total.saturating_sub(1));
    if bound == 0 || bound > TAIL_PREFETCH {
        return None;
    }
    let start = total.saturating_sub(bound);
    if start == 0 {
        return None;
    }
    Some((start, total))
}

pub fn has_moov_atom(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let last = data.len() - 8;
    for i in 0..=last {
        if &data[i + 4..i + 8] != b"moov" {
            continue;
        }
        let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        if size == 1 {
            if i + 16 > data.len() {
                continue;
            }
            let size64 = u64::from_be_bytes([
                data[i + 8],
                data[i + 9],
                data[i + 10],
                data[i + 11],
                data[i + 12],
                data[i + 13],
                data[i + 14],
                data[i + 15],
            ]);
            if size64 >= 16 {
                return true;
            }
        } else if size >= 8 {
            return true;
        }
    }
    false
}

pub fn inspect(data: &[u8]) -> Inspect {
    if data.len() < 4 {
        return Inspect::NeedMore;
    }
    if data.starts_with(b"RIFF") {
        return inspect_wav(data);
    }
    if data.starts_with(b"ID3") || looks_like_mp3_frame(data) {
        return inspect_mp3(data);
    }
    if looks_like_isobmff(data) {
        return inspect_isobmff(data);
    }
    if find_mp3_sync(data).is_some() {
        return inspect_mp3(data);
    }
    if data.len() < 16 {
        Inspect::NeedMore
    } else {
        Inspect::Invalid
    }
}

fn inspect_wav(data: &[u8]) -> Inspect {
    if data.len() < 12 {
        return Inspect::NeedMore;
    }
    if &data[8..12] != b"WAVE" {
        return Inspect::Invalid;
    }
    if data.len() < 44 {
        Inspect::NeedMore
    } else {
        Inspect::Wav
    }
}

fn inspect_mp3(data: &[u8]) -> Inspect {
    let mut offset = 0usize;
    if data.starts_with(b"ID3") {
        if data.len() < 10 {
            return Inspect::NeedMore;
        }
        let size = id3v2_size(&data[6..10]);
        let Some(end) = 10usize.checked_add(size) else {
            return Inspect::Invalid;
        };
        if data.len() < end {
            return Inspect::NeedMore;
        }
        offset = end;
    }
    let slice = data.get(offset..).unwrap_or(&[]);
    if slice.is_empty() {
        return Inspect::NeedMore;
    }
    match find_mp3_sync(slice) {
        Some(_) => Inspect::Mp3,
        None if data.len() < 4096 => Inspect::NeedMore,
        None => Inspect::NeedMore,
    }
}

fn id3v2_size(bytes: &[u8]) -> usize {
    if bytes.len() < 4 {
        return 0;
    }
    (((bytes[0] & 0x7f) as usize) << 21)
        | (((bytes[1] & 0x7f) as usize) << 14)
        | (((bytes[2] & 0x7f) as usize) << 7)
        | ((bytes[3] & 0x7f) as usize)
}

fn looks_like_mp3_frame(data: &[u8]) -> bool {
    find_mp3_sync(data).is_some()
}

fn find_mp3_sync(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    let last = (data.len() - 4).min(8191);
    for i in 0..=last {
        if valid_mp3_header(&data[i..i + 4]) {
            return Some(i);
        }
    }
    None
}

fn valid_mp3_header(h: &[u8]) -> bool {
    if h.len() < 4 {
        return false;
    }
    if h[0] != 0xff || h[1] & 0xe0 != 0xe0 {
        return false;
    }
    let version = (h[1] >> 3) & 0x03;
    let layer = (h[1] >> 1) & 0x03;
    let bitrate = h[2] >> 4;
    let sample = (h[2] >> 2) & 0x03;
    version != 1 && layer != 0 && bitrate != 0 && bitrate != 0x0f && sample != 0x03
}

fn looks_like_isobmff(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let kind = &data[4..8];
    kind.iter().all(|b| b.is_ascii_alphanumeric() || *b == b' ')
        && (kind == b"ftyp"
            || kind == b"moov"
            || kind == b"mdat"
            || kind == b"free"
            || kind == b"wide")
}

fn inspect_isobmff(data: &[u8]) -> Inspect {
    let mut offset = 0usize;
    let mut moov = false;
    let mut mdat = false;
    let mut mdat_before_moov = false;
    let mut saw_atom = false;

    while offset < data.len() {
        if data.len().saturating_sub(offset) < 8 {
            return if saw_atom {
                classify_isobmff(moov, mdat, mdat_before_moov)
            } else {
                Inspect::NeedMore
            };
        }
        let size32 = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        let kind = [
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ];
        let (header, atom_size) = if size32 == 1 {
            if data.len().saturating_sub(offset) < 16 {
                return classify_isobmff(moov, mdat, mdat_before_moov);
            }
            let size64 = u64::from_be_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
                data[offset + 12],
                data[offset + 13],
                data[offset + 14],
                data[offset + 15],
            ]);
            if size64 < 16 {
                return Inspect::Invalid;
            }
            (16u64, size64)
        } else if size32 == 0 {
            if &kind == b"mdat" {
                mdat = true;
                if !moov {
                    mdat_before_moov = true;
                }
            }
            if &kind == b"moov" {
                // Extends to EOF: only complete once the caller marks the file done.
                return classify_isobmff(false, mdat, mdat_before_moov);
            }
            return classify_isobmff(moov, mdat, mdat_before_moov);
        } else {
            if size32 < 8 {
                return Inspect::Invalid;
            }
            (8u64, u64::from(size32))
        };

        let Some(end) = (offset as u64).checked_add(atom_size) else {
            return Inspect::Invalid;
        };
        if end > usize::MAX as u64 {
            return Inspect::Invalid;
        }
        let end = end as usize;
        if header > atom_size {
            return Inspect::Invalid;
        }
        saw_atom = true;

        if &kind == b"mdat" {
            mdat = true;
            if !moov {
                mdat_before_moov = true;
            }
        }
        if &kind == b"moov" {
            if end > data.len() {
                return classify_isobmff(false, mdat, mdat_before_moov);
            }
            moov = true;
        }

        if end > data.len() {
            return classify_isobmff(moov, mdat, mdat_before_moov);
        }
        offset = end;
    }

    classify_isobmff(moov, mdat, mdat_before_moov)
}

fn classify_isobmff(moov: bool, mdat: bool, mdat_before_moov: bool) -> Inspect {
    if moov {
        Inspect::FastStart
    } else if mdat || mdat_before_moov {
        Inspect::MoovAtEnd
    } else {
        Inspect::NeedMore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 8u32 + payload.len() as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn atom64(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 16u64 + payload.len() as u64;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn mp3_probe_id3_and_frame_sync() {
        let mut id3 = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
        id3.extend_from_slice(&[0xff, 0xfb, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(inspect(&id3), Inspect::Mp3);
        assert_eq!(inspect(&[0xff, 0xfb, 0x90, 0x00]), Inspect::Mp3);
        assert_eq!(inspect(b"ID3\x04\x00\x00"), Inspect::NeedMore);
        assert!(playback_ready(&id3, id3.len(), false));
        assert!(playback_ready(&id3, id3.len(), true));
        assert!(!playback_ready(b"ID3\x04\x00\x00", 7, false));
    }

    #[test]
    fn headers_start_without_ten_second_buffer() {
        let mut mp3 = vec![0xff, 0xfb, 0x90, 0x00];
        mp3.resize(64, 0);
        assert!(playback_ready(&mp3, mp3.len(), false));
        assert!(mp3.len() < 160_000);

        let mut fast = atom(b"ftyp", b"M4A ");
        fast.extend_from_slice(&atom(b"moov", &[0; 32]));
        assert_eq!(inspect(&fast), Inspect::FastStart);
        assert!(playback_ready(&fast, fast.len(), false));
        assert!(fast.len() < 64 * 1024);
    }

    #[test]
    fn m4a_fast_start_vs_moov_at_end() {
        let mut fast = atom(b"ftyp", b"M4A ");
        fast.extend_from_slice(&atom(b"moov", &[0; 32]));
        fast.extend_from_slice(&atom(b"mdat", &[0; 8]));
        assert_eq!(inspect(&fast), Inspect::FastStart);
        assert!(playback_ready(&fast, fast.len(), false));

        let mut late = atom(b"ftyp", b"M4A ");
        late.extend_from_slice(&atom(b"mdat", &[0; 64]));
        assert_eq!(inspect(&late), Inspect::MoovAtEnd);
        assert!(!playback_ready(&late, late.len(), false));
        late.extend_from_slice(&atom(b"moov", &[0; 16]));
        assert_eq!(inspect(&late), Inspect::FastStart);

        let mut waiting = atom(b"ftyp", b"M4A ");
        waiting.extend_from_slice(&atom(b"mdat", &[0; 8]));
        assert_eq!(inspect(&waiting), Inspect::MoovAtEnd);
        assert!(playback_ready(&waiting, waiting.len(), true));
    }

    #[test]
    fn incomplete_and_64bit_atoms() {
        let mut short = atom(b"ftyp", b"M4A ");
        short.extend_from_slice(&20u32.to_be_bytes());
        short.extend_from_slice(b"moov");
        short.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(inspect(&short), Inspect::NeedMore);

        let mut big = atom(b"ftyp", b"M4A ");
        big.extend_from_slice(&atom64(b"moov", &[9; 8]));
        assert_eq!(inspect(&big), Inspect::FastStart);

        let mut bad = Vec::from(4u32.to_be_bytes());
        bad.extend_from_slice(b"ftyp");
        assert_eq!(inspect(&bad), Inspect::Invalid);
    }

    #[test]
    fn wav_header() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(&[0; 32]);
        assert_eq!(inspect(&wav), Inspect::Wav);
        assert_eq!(inspect(b"RIFF"), Inspect::NeedMore);
    }

    #[test]
    fn late_moov_tail_is_compact_and_detectable() {
        let mut prefix = atom(b"ftyp", b"M4A ");
        prefix.extend_from_slice(&atom(b"mdat", &[0; 64]));
        assert_eq!(inspect(&prefix), Inspect::MoovAtEnd);
        let total = 4 * 1024 * 1024;
        let (start, end) = tail_prefetch(&prefix, total).unwrap();
        assert_eq!(end, total);
        assert_eq!(start, total - TAIL_PREFETCH);
        assert!(tail_prefetch(&prefix, 16).is_none());

        let moov = atom(b"moov", &[1; 24]);
        assert!(has_moov_atom(&moov));
        assert!(playback_ready_sparse(&prefix, &moov, false));
        assert!(!playback_ready_sparse(&prefix, &[0; 32], false));
        assert!(playback_ready_sparse(&prefix, &[], true));
    }
}
