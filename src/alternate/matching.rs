//! Query normalisation and deterministic candidate ranking.

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct TrackQuery {
    pub title: String,
    pub artists: Vec<String>,
    pub duration_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub uploader: String,
    pub duration_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RankedMatch {
    pub candidate: Candidate,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedQuery {
    pub title: String,
    pub artists: Vec<String>,
}

const NOISE_PHRASES: &[&str] = &[
    "official audio",
    "official video",
    "official music video",
    "official visualizer",
    "music video",
    "lyric video",
    "lyrics",
    "visualizer",
    "audio only",
    "album version",
    "radio edit",
    "original mix",
    "bonus track",
    "remastered",
    "remaster",
    "deluxe",
    "explicit",
    "clean version",
    "clean",
    "mono",
    "stereo",
    "hd",
    "4k",
    "vevo",
];

pub fn normalize_query(query: &TrackQuery) -> NormalizedQuery {
    NormalizedQuery {
        title: normalize_text(&query.title),
        artists: query
            .artists
            .iter()
            .map(|artist| normalize_text(artist))
            .filter(|artist| !artist.is_empty())
            .collect(),
    }
}

pub fn normalize_text(input: &str) -> String {
    let lowered = input.to_lowercase();
    let mut stripped = String::with_capacity(lowered.len());
    let mut prev_space = true;
    for ch in lowered.chars() {
        if ch.is_alphanumeric() {
            stripped.push(ch);
            prev_space = false;
        } else if ch == '\'' || ch == '’' {
            continue;
        } else if !prev_space {
            stripped.push(' ');
            prev_space = true;
        }
    }
    let mut text = strip_featuring(stripped.trim());
    for phrase in NOISE_PHRASES {
        text = strip_phrase(&text, phrase);
    }
    collapse_spaces(&text)
}

pub fn rank_candidates(
    query: &TrackQuery,
    candidates: &[Candidate],
    min_score: f32,
) -> Option<RankedMatch> {
    let normalized = normalize_query(query);
    if normalized.title.is_empty() {
        return None;
    }
    let mut best: Option<RankedMatch> = None;
    for candidate in candidates {
        let score = score_candidate(query, &normalized, candidate);
        if score < min_score {
            continue;
        }
        let better = match &best {
            None => true,
            Some(current) => {
                score > current.score + 0.0001
                    || ((score - current.score).abs() <= 0.0001
                        && candidate.id < current.candidate.id)
            }
        };
        if better {
            best = Some(RankedMatch {
                candidate: candidate.clone(),
                score,
            });
        }
    }
    best
}

fn score_candidate(query: &TrackQuery, normalized: &NormalizedQuery, candidate: &Candidate) -> f32 {
    let cand_title = normalize_text(&candidate.title);
    let cand_uploader = normalize_text(&candidate.uploader);
    let title_score = token_jaccard(&normalized.title, &cand_title);
    let artist_score = artist_match(normalized, &cand_title, &cand_uploader);
    let duration_score = duration_match(query.duration_ms, candidate.duration_ms);
    let penalty = mismatch_penalty(&normalized.title, &candidate.title, &candidate.uploader);
    let official = official_hint(&candidate.title, &candidate.uploader);
    let mut score = title_score * 0.46 + artist_score * 0.24 + duration_score * 0.18;
    // Official/topic hints must not lift an unrequested live/remix/cover over the bar.
    if official && penalty == 0.0 {
        score += 0.12;
    }
    score -= penalty;
    score.clamp(0.0, 1.0)
}

fn artist_match(normalized: &NormalizedQuery, title: &str, uploader: &str) -> f32 {
    if normalized.artists.is_empty() {
        return 0.5;
    }
    let mut best = 0.0f32;
    for artist in &normalized.artists {
        if artist.is_empty() {
            continue;
        }
        if title.contains(artist) || uploader.contains(artist) {
            best = best.max(1.0);
            continue;
        }
        let overlap = token_jaccard(artist, uploader).max(token_jaccard(artist, title));
        best = best.max(overlap);
    }
    best
}

fn duration_match(expected: Option<u32>, actual: Option<u32>) -> f32 {
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return 0.55;
    };
    if expected == 0 {
        return 0.55;
    }
    let delta = expected.abs_diff(actual) as f32 / 1000.0;
    if delta <= 2.0 {
        1.0
    } else if delta >= 25.0 {
        0.0
    } else {
        1.0 - (delta - 2.0) / 23.0
    }
}

fn official_hint(title: &str, uploader: &str) -> bool {
    let title = title.to_lowercase();
    let uploader = uploader.to_lowercase();
    uploader.ends_with(" - topic")
        || uploader.contains("vevo")
        || title.contains("official audio")
        || title.contains("official video")
        || title.contains("official visualizer")
        || title.contains("official music video")
}

fn mismatch_penalty(query_title: &str, raw_title: &str, raw_uploader: &str) -> f32 {
    let hay = format!(
        "{} {}",
        raw_title.to_lowercase(),
        raw_uploader.to_lowercase()
    );
    let mut penalty = 0.0;
    penalty += tag_penalty(query_title, &hay, &["karaoke"], 0.45);
    penalty += tag_penalty(query_title, &hay, &["nightcore"], 0.45);
    penalty += tag_penalty(query_title, &hay, &["cover"], 0.38);
    penalty += tag_penalty(query_title, &hay, &["slowed", "reverb"], 0.36);
    penalty += tag_penalty(query_title, &hay, &["live"], 0.28);
    penalty += tag_penalty(query_title, &hay, &["remix"], 0.22);
    penalty
}

fn tag_penalty(query_title: &str, haystack: &str, tags: &[&str], amount: f32) -> f32 {
    let query_has = tags.iter().any(|tag| contains_word(query_title, tag));
    let hay_has = tags.iter().any(|tag| contains_word(haystack, tag));
    if hay_has && !query_has { amount } else { 0.0 }
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|token| token == word)
}

fn token_jaccard(a: &str, b: &str) -> f32 {
    let left = tokens(a);
    let right = tokens(b);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let inter = left.intersection(&right).count() as f32;
    let union = left.union(&right).count() as f32;
    if union == 0.0 { 0.0 } else { inter / union }
}

fn tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .filter(|token| token.len() > 1)
        .map(|token| token.to_string())
        .collect()
}

fn strip_featuring(text: &str) -> String {
    let markers = [" feat ", " ft ", " featuring "];
    let mut cut = text.len();
    for marker in markers {
        if let Some(index) = text.find(marker) {
            cut = cut.min(index);
        }
    }
    text[..cut].trim().to_string()
}

fn strip_phrase(text: &str, phrase: &str) -> String {
    if phrase.is_empty() {
        return text.to_string();
    }
    collapse_spaces(&text.replace(phrase, " "))
}

fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(title: &str, artists: &[&str], duration_ms: u32) -> TrackQuery {
        TrackQuery {
            title: title.into(),
            artists: artists.iter().map(|name| (*name).to_string()).collect(),
            duration_ms: Some(duration_ms),
        }
    }

    fn cand(id: &str, title: &str, uploader: &str, duration_ms: u32) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            uploader: uploader.into(),
            duration_ms: Some(duration_ms),
        }
    }

    #[test]
    fn normalize_strips_feat_and_remaster_noise() {
        let text =
            normalize_text("Blinding Lights (feat. Someone) [Remastered 2020] - Official Audio");
        assert_eq!(text, "blinding lights");
    }

    #[test]
    fn good_official_topic_match_ranks_high() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let candidates = [cand(
            "aaaaaaaaaaa",
            "Blinding Lights (Official Audio)",
            "The Weeknd - Topic",
            200_000,
        )];
        let ranked = rank_candidates(&q, &candidates, 0.55).unwrap();
        assert!(ranked.score > 0.75, "score was {}", ranked.score);
        assert_eq!(ranked.candidate.id, "aaaaaaaaaaa");
    }

    #[test]
    fn unrelated_title_is_a_miss() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let candidates = [cand("bbbbbbbbbbb", "Shape of You", "Ed Sheeran", 233_000)];
        assert!(rank_candidates(&q, &candidates, 0.55).is_none());
    }

    #[test]
    fn live_mismatch_is_penalized_below_threshold() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let candidates = [cand(
            "ccccccccccc",
            "Blinding Lights (Live at the Bowl)",
            "Some Channel",
            260_000,
        )];
        let ranked = rank_candidates(&q, &candidates, 0.0).unwrap();
        assert!(ranked.score < 0.55, "live score was {}", ranked.score);
        assert!(rank_candidates(&q, &candidates, 0.55).is_none());
    }

    #[test]
    fn official_hint_cannot_save_unrequested_live_or_remix() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let live = [cand(
            "ccccccccccc",
            "Blinding Lights (Live Official Audio)",
            "The Weeknd - Topic",
            200_000,
        )];
        let remix = [cand(
            "hhhhhhhhhhh",
            "Blinding Lights (Remix)",
            "The Weeknd - Topic",
            200_000,
        )];
        assert!(rank_candidates(&q, &live, 0.55).is_none());
        assert!(rank_candidates(&q, &remix, 0.55).is_none());
    }

    #[test]
    fn cover_mismatch_is_rejected() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let candidates = [cand(
            "ddddddddddd",
            "Blinding Lights (Cover)",
            "Bedroom Covers",
            205_000,
        )];
        assert!(rank_candidates(&q, &candidates, 0.55).is_none());
    }

    #[test]
    fn karaoke_and_nightcore_are_rejected() {
        let q = query("Blinding Lights", &["The Weeknd"], 200_000);
        let karaoke = [cand(
            "eeeeeeeeeee",
            "Blinding Lights Karaoke Version",
            "Karaoke Hits",
            200_000,
        )];
        let nightcore = [cand(
            "fffffffffff",
            "Nightcore - Blinding Lights",
            "Nightcore World",
            150_000,
        )];
        assert!(rank_candidates(&q, &karaoke, 0.55).is_none());
        assert!(rank_candidates(&q, &nightcore, 0.55).is_none());
    }

    #[test]
    fn requested_live_keeps_live_candidate() {
        let q = query("Blinding Lights Live", &["The Weeknd"], 240_000);
        let candidates = [cand(
            "ggggggggggg",
            "Blinding Lights (Live)",
            "The Weeknd",
            240_000,
        )];
        let ranked = rank_candidates(&q, &candidates, 0.5).unwrap();
        assert_eq!(ranked.candidate.id, "ggggggggggg");
    }

    #[test]
    fn ranking_is_deterministic_on_tied_scores() {
        let q = query("Same Song", &["Same Artist"], 180_000);
        let a = cand("bbbbbbbbbbb", "Same Song", "Same Artist - Topic", 180_000);
        let b = cand("aaaaaaaaaaa", "Same Song", "Same Artist - Topic", 180_000);
        let ranked = rank_candidates(&q, &[a.clone(), b.clone()], 0.5).unwrap();
        assert_eq!(ranked.candidate.id, "aaaaaaaaaaa");
        let ranked_rev = rank_candidates(&q, &[b, a], 0.5).unwrap();
        assert_eq!(ranked_rev.candidate.id, "aaaaaaaaaaa");
    }
}
