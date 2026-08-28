//! Pick a locally decodable audio stream. AAC/M4A first, then MP3.
//!
//! Alternate playback does not select Opus, WebM, Vorbis, or Ogg.
//! Opus/WebM stay out because this crate does not enable an Opus decoder
//! (`symphonia-aac` / `symphonia-mp3` / `symphonia-isomp4` only).
//! Ogg/Vorbis may exist via librespot/symphonia; they are still rejected
//! because YouTube's useful native alternative is WebM/Opus, so enabling
//! Ogg adds no useful YouTube startup path.
//! Fast-start M4A is the usual YouTube/Piped audio; MP3 starts from a few
//! frames. Lower-bitrate copies are not preferred — headers, not payload
//! size, gate time-to-first-audio.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioStream {
    pub url: String,
    pub mime: Option<String>,
    pub codec: Option<String>,
    pub format: Option<String>,
    pub bitrate: Option<u32>,
    pub video_only: bool,
    pub quality: Option<String>,
    pub http_headers: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Aac,
    Mp3,
    Wav,
}

pub fn select_audio_stream(streams: &[AudioStream]) -> Option<&AudioStream> {
    let mut best_aac: Option<(u32, usize)> = None;
    let mut best_mp3: Option<(u32, usize)> = None;
    let mut best_wav: Option<(u32, usize)> = None;
    for (index, stream) in streams.iter().enumerate() {
        if !usable(stream) {
            continue;
        }
        let bitrate = stream.bitrate.unwrap_or(0);
        match family(stream) {
            Some(Family::Aac) if best_aac.is_none_or(|(current, _)| bitrate >= current) => {
                best_aac = Some((bitrate, index));
            }
            Some(Family::Mp3) if best_mp3.is_none_or(|(current, _)| bitrate >= current) => {
                best_mp3 = Some((bitrate, index));
            }
            Some(Family::Wav) if best_wav.is_none_or(|(current, _)| bitrate >= current) => {
                best_wav = Some((bitrate, index));
            }
            _ => {}
        }
    }
    best_aac
        .or(best_mp3)
        .or(best_wav)
        .map(|(_, index)| &streams[index])
}

fn usable(stream: &AudioStream) -> bool {
    if stream.url.is_empty() || stream.video_only {
        return false;
    }
    let blob = blob(stream);
    if blob.contains("video/") && !blob.contains("audio/") {
        return false;
    }
    if blob.contains("opus")
        || blob.contains("webm")
        || blob.contains("vorbis")
        || blob.contains("vp9")
        || blob.contains("avc1")
        || blob.contains("av01")
    {
        return false;
    }
    family(stream).is_some()
}

fn family(stream: &AudioStream) -> Option<Family> {
    let blob = blob(stream);
    if blob.contains("mp4a")
        || blob.contains("aac")
        || blob.contains("m4a")
        || blob.contains("audio/mp4")
        || blob.contains("audio/aac")
        || blob.contains("mp4") && blob.contains("audio")
    {
        return Some(Family::Aac);
    }
    if blob.contains("mp3") || blob.contains("mpeg") || blob.contains("mpga") {
        return Some(Family::Mp3);
    }
    if blob.contains("wav") || blob.contains("pcm") {
        return Some(Family::Wav);
    }
    // Bare "m4a" format labels from Piped / yt-dlp.
    if stream.format.as_deref().is_some_and(|format| {
        let format = format.to_ascii_lowercase();
        format == "m4a" || format == "mp4" || format == "aac"
    }) {
        return Some(Family::Aac);
    }
    None
}

fn blob(stream: &AudioStream) -> String {
    format!(
        "{} {} {} {}",
        stream.mime.as_deref().unwrap_or(""),
        stream.codec.as_deref().unwrap_or(""),
        stream.format.as_deref().unwrap_or(""),
        stream.quality.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(
        format: &str,
        mime: &str,
        codec: &str,
        bitrate: u32,
        video_only: bool,
    ) -> AudioStream {
        AudioStream {
            url: format!("https://cdn.example/{format}"),
            mime: Some(mime.into()),
            codec: Some(codec.into()),
            format: Some(format.into()),
            bitrate: Some(bitrate),
            video_only,
            quality: None,
            http_headers: Vec::new(),
        }
    }

    #[test]
    fn prefers_aac_over_higher_bitrate_mp3() {
        let streams = [
            stream("mp3", "audio/mpeg", "mp3", 320_000, false),
            stream("m4a", "audio/mp4", "mp4a.40.2", 128_000, false),
        ];
        let picked = select_audio_stream(&streams).unwrap();
        assert_eq!(picked.format.as_deref(), Some("m4a"));
    }

    #[test]
    fn falls_back_to_mp3() {
        let streams = [stream("mp3", "audio/mpeg", "mp3", 192_000, false)];
        assert_eq!(
            select_audio_stream(&streams).unwrap().format.as_deref(),
            Some("mp3")
        );
    }

    #[test]
    fn accepts_wav_when_nothing_else_is_playable() {
        let streams = [stream("wav", "audio/wav", "pcm", 128_000, false)];
        assert_eq!(
            select_audio_stream(&streams).unwrap().format.as_deref(),
            Some("wav")
        );
    }

    #[test]
    fn rejects_video_only_and_opus_webm() {
        let streams = [
            stream("mp4", "video/mp4", "avc1", 0, true),
            stream("webm", "audio/webm", "opus", 160_000, false),
            stream("ogg", "audio/ogg", "vorbis", 128_000, false),
            AudioStream {
                url: String::new(),
                mime: Some("audio/mp4".into()),
                codec: Some("mp4a.40.2".into()),
                format: Some("m4a".into()),
                bitrate: Some(128_000),
                video_only: false,
                quality: None,
                http_headers: Vec::new(),
            },
        ];
        assert!(select_audio_stream(&streams).is_none());
    }

    #[test]
    fn picks_higher_bitrate_within_aac() {
        let streams = [
            stream("m4a", "audio/mp4", "mp4a.40.2", 64_000, false),
            stream("m4a", "audio/mp4", "mp4a.40.2", 256_000, false),
        ];
        assert_eq!(
            select_audio_stream(&streams).unwrap().bitrate,
            Some(256_000)
        );
    }
}
