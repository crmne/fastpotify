//! Translation and romanization for the lyrics panel, from Google Translate.
//!
//! [Google Translate](https://translate.google.com) needs no account and no
//! key on the endpoint its own web page uses, so the panel can put a line in
//! the reader's language under every lyric line and, when the words are
//! written in a script one cannot sing from, spell the same line in Latin
//! letters.
//!
//! The two aids travel differently. Translation batches well: lines joined
//! with newlines come back with the newlines in place, so a few requests
//! cover a whole song and the answers still line up afterwards. Romanization
//! loses the newlines, so it is asked for one line at a time, a few requests
//! in flight. And when Google hears the song already in the reader's
//! language, there is nothing to add, and nothing more is asked.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const API: &str = "https://clients5.google.com/translate_a/single";
/// Lyrics do not change; a cached answer is good for this long.
const CACHE_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The room one request's query may take, counted so carefully that no song,
/// however dense with wide letters, pushes the URL past what servers accept.
const MAX_QUERY: usize = 6000;
/// Romanization is asked per line; this many lines may ask at once.
const ROMANIZE_AT_ONCE: usize = 5;
/// A refused request is tried again this many times, waiting a little
/// longer between tries, since an answer a moment late still spares the
/// next play of the song every request at all.
const ATTEMPTS: u32 = 3;
/// The wait before the first retry, and the ceiling it doubles towards.
const FIRST_RETRY: Duration = Duration::from_millis(500);
const MAX_RETRY: Duration = Duration::from_secs(4);

/// Per-line aids for the lyrics panel, aligned with the track's lyric lines.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Translation {
    /// The lines rewritten in Latin letters; `None` keeps the original line.
    pub romanized: Vec<Option<String>>,
    /// The lines in the reader's language; `None` for no translation of that line.
    pub translated: Vec<Option<String>>,
}

/// The translation and romanization of `lines` into `target` (a Google
/// language code like "en" or "zh-CN"), or `None` when there is nothing to
/// add: the song is already in the target language, or the lines are empty.
/// Answers are kept on disk, so a song heard again costs no request.
pub async fn fetch(
    http: &reqwest::Client,
    cache_dir: &Path,
    lines: &[&str],
    target: &str,
) -> Result<Option<Translation>> {
    // Nothing to say: an empty song asks for nothing.
    if lines.iter().all(|line| line.trim().is_empty()) {
        return Ok(None);
    }
    let cache_path = cache_dir.join(format!("{}.json", cache_key(target, lines)));
    if let Some(cached) = read_cache(&cache_path) {
        return Ok(cached);
    }
    let found = ask(http, lines, target).await?;
    write_cache(&cache_path, &found);
    Ok(found)
}

/// What Google has for `lines`, fresh from the endpoint.
async fn ask(http: &reqwest::Client, lines: &[&str], target: &str) -> Result<Option<Translation>> {
    let chunks = chunk_lines(lines);
    let mut text = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let (piece, source) = translated(&request(http, target, chunk).await?)?;
        // The first answer tells the tongue the song was heard in; when that
        // is the reader's own, there is nothing to add, and no spelling to
        // ask for either.
        if index == 0 && same_language(&source, target) {
            return Ok(None);
        }
        // An answer keeps the newlines inside a chunk but not the one at
        // its end, so the chunks are stitched back with the newline each
        // lost.
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&piece);
    }
    Ok(Some(Translation {
        romanized: romanize(http, lines, target).await,
        translated: aligned(&text, lines.len()),
    }))
}

/// One call to the endpoint the Google Translate page itself uses, tried
/// again on a refusal: a little later each time, with a dash of randomness
/// so concurrent lines do not knock together, and for as long as Google's
/// own `Retry-After` asks when it sends one. `text` may hold several lines
/// joined with newlines.
async fn request(http: &reqwest::Client, target: &str, text: &str) -> Result<serde_json::Value> {
    let url = format!(
        "{API}?client=dict-chrome-ex&dt=t&dt=rm&sl=auto&tl={}&q={}",
        urlencoding::encode(target),
        urlencoding::encode(text),
    );
    let mut wait = FIRST_RETRY;
    for attempt in 0..ATTEMPTS {
        let response = http
            .get(&url)
            .send()
            .await
            .context("cannot reach Google Translate")?;
        let status = response.status();
        if status.is_success() {
            return response
                .json()
                .await
                .context("unexpected answer from Google Translate");
        }
        if attempt + 1 == ATTEMPTS {
            anyhow::bail!("Google Translate answered {status}");
        }
        let asked = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(wait);
        let pause = asked + Duration::from_millis(rand::random::<u64>() % 250);
        log::debug!("Google Translate answered {status}; trying again in {pause:?}");
        tokio::time::sleep(pause).await;
        wait = (wait * 2).min(MAX_RETRY);
    }
    unreachable!("the loop returns or bails on its last attempt")
}

/// The translated text of an answer — every segment's first words, joined —
/// and the tongue Google heard the text in.
fn translated(response: &serde_json::Value) -> Result<(String, String)> {
    let segments = response
        .get(0)
        .and_then(|part| part.as_array())
        .context("unexpected answer from Google Translate")?;
    let mut text = String::new();
    for segment in segments {
        // A romanization segment opens with null and says nothing here.
        if let Some(piece) = segment.get(0).and_then(|part| part.as_str()) {
            text.push_str(piece);
        }
    }
    let source = response
        .get(2)
        .and_then(|part| part.as_str())
        .context("unexpected answer from Google Translate")?
        .to_string();
    Ok((text, source))
}

/// The romanization an answer carries, if Google offered one.
fn romanization(response: &serde_json::Value) -> Option<String> {
    response
        .get(0)?
        .as_array()?
        .iter()
        .find_map(|segment| segment.get(3).and_then(|part| part.as_str()))
        .map(str::to_string)
}

/// Puts a batched answer back on its lines: a line that came back empty
/// keeps `None`, and a trailing line Google dropped stays gone, so the count
/// still matches the song.
fn aligned(text: &str, count: usize) -> Vec<Option<String>> {
    let mut lines: Vec<Option<String>> = text
        .split('\n')
        .map(|line| (!line.is_empty()).then(|| line.to_string()))
        .collect();
    lines.resize(count, None);
    lines
}

/// Whether `source` and `target` name the same tongue, ignoring the region
/// Google appends ("zh-CN") and the case it uses.
fn same_language(source: &str, target: &str) -> bool {
    fn base(code: &str) -> &str {
        code.split_once('-').map_or(code, |(root, _)| root)
    }
    base(source).eq_ignore_ascii_case(base(target))
}

// ---- romanization ------------------------------------------------------------

/// Romanizations for every line, in order. Lines already written in plain
/// ASCII letters have nothing to spell, and a chorus sings its lines again,
/// so each distinct line is asked for once, a few at a time. A line whose
/// request fails keeps `None`, for one stubborn line must not cost the
/// whole song its spelling.
async fn romanize(http: &reqwest::Client, lines: &[&str], target: &str) -> Vec<Option<String>> {
    let mut distinct: Vec<&str> = Vec::new();
    for line in lines {
        if line.chars().all(|c| c.is_ascii()) || distinct.contains(line) {
            continue;
        }
        distinct.push(line);
    }
    let permits = Arc::new(Semaphore::new(ROMANIZE_AT_ONCE));
    let mut tasks = JoinSet::new();
    for line in &distinct {
        let permits = Arc::clone(&permits);
        let http = http.clone();
        let target = target.to_string();
        let line = (*line).to_string();
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            let spelled = match romanize_line(&http, &target, &line).await {
                Ok(spelled) => Some(spelled),
                Err(error) => {
                    log::debug!("no romanization for a line: {error:#}");
                    None
                }
            };
            (line, spelled)
        });
    }
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    while let Some(finished) = tasks.join_next().await {
        match finished {
            Ok((line, spelled)) => {
                let spelling = kept(&line, spelled);
                found.push((line, spelling));
            }
            Err(error) => log::debug!("a romanization request died: {error}"),
        }
    }
    lines
        .iter()
        .map(|line| {
            found
                .iter()
                .find(|(text, _)| text == *line)
                .and_then(|(_, spelling)| spelling.clone())
        })
        .collect()
}

/// The spelling to keep for a line: `None` when Google had nothing, or
/// spelled out exactly what was written already.
fn kept(line: &str, found: Option<String>) -> Option<String> {
    found.filter(|spelled| spelled != line)
}

/// One line's romanization. Romanization loses newlines, so it is asked for
/// a single line, never for a batch.
async fn romanize_line(http: &reqwest::Client, target: &str, line: &str) -> Result<String> {
    let response = request(http, target, line).await?;
    romanization(&response).context("Google Translate offered no romanization")
}

// ---- batching ----------------------------------------------------------------

/// The room a piece of text takes in the query string, counted the careful
/// way: an ASCII letter may grow to three (`%20`), any other letter to nine
/// (`%E3%81%82`), so a batch that fits this count fits whatever the encoder
/// really does.
fn encoded_len(text: &str) -> usize {
    text.chars().map(|c| if c.is_ascii() { 3 } else { 9 }).sum()
}

/// Splits `lines` into batches whose query stays inside the budget. Lines
/// keep their order and their newlines, so the answers can be realigned
/// afterwards; a single line longer than the budget gets a request of its
/// own, for no lyric line is that long.
fn chunk_lines(lines: &[&str]) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut budget = 0;
    let mut held = 0;
    for line in lines {
        let cost = encoded_len(line) + if held == 0 { 0 } else { 3 };
        if held > 0 && budget + cost > MAX_QUERY {
            chunks.push(std::mem::take(&mut current));
            budget = 0;
            held = 0;
        }
        if held > 0 {
            current.push('\n');
            budget += 3;
        }
        current.push_str(line);
        budget += encoded_len(line);
        held += 1;
    }
    if held > 0 {
        chunks.push(current);
    }
    chunks
}

// ---- cache ------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Cached {
    found: Option<Translation>,
}

fn cache_key(target: &str, lines: &[&str]) -> String {
    let digest = Sha1::digest(format!("{target}\n{}", lines.join("\n")).as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_cache(path: &PathBuf) -> Option<Option<Translation>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    if modified.elapsed().unwrap_or(CACHE_LIFETIME) >= CACHE_LIFETIME {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Cached>(&text)
        .ok()
        .map(|cached| cached.found)
}

fn write_cache(path: &Path, found: &Option<Translation>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(&Cached {
        found: found.clone(),
    }) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_give_their_text_and_their_spelling() {
        let answer: serde_json::Value = serde_json::from_str(
            r#"[
                [["Hello","こんにちは",null,null,10],["world","世界",null,null,10],
                 [null,null,null,"konnichiwa sekai"]],
                null,"ja"
            ]"#,
        )
        .unwrap();
        let (text, source) = translated(&answer).unwrap();
        assert_eq!(text, "Helloworld");
        assert_eq!(source, "ja");
        assert_eq!(romanization(&answer).as_deref(), Some("konnichiwa sekai"));
        let plain: serde_json::Value =
            serde_json::from_str(r#"[["Hello","hola",null,null,10],null,"es"]"#).unwrap();
        assert_eq!(romanization(&plain), None);
    }

    #[test]
    fn batched_answers_land_back_on_their_lines() {
        // Google may drop the trailing empty line of a batch.
        assert_eq!(
            aligned("one\ntwo\n\nthree", 5),
            vec![
                Some("one".to_string()),
                Some("two".to_string()),
                None,
                Some("three".to_string()),
                None,
            ]
        );
        assert_eq!(
            aligned("one\ntwo\nthree", 2),
            vec![Some("one".to_string()), Some("two".to_string())]
        );
    }

    #[test]
    fn a_song_already_in_the_readers_language_is_left_alone() {
        assert!(same_language("en", "en"));
        assert!(same_language("zh-CN", "zh-TW"));
        assert!(same_language("EN", "en"));
        assert!(!same_language("ja", "en"));
    }

    #[test]
    fn a_spelling_that_repeats_the_line_is_no_help() {
        assert_eq!(kept("café", Some("café".into())), None);
        assert_eq!(kept("café", Some("cafe".into())).as_deref(), Some("cafe"));
        assert_eq!(kept("café", None), None);
    }

    #[test]
    fn chunks_keep_every_line_and_fit_a_url() {
        let line = "あ".repeat(100);
        let lines: Vec<String> = (0..20).map(|_| line.clone()).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let chunks = chunk_lines(&refs);
        assert!(chunks.len() > 1, "a song of CJK lines must split");
        for chunk in &chunks {
            assert!(encoded_len(chunk) <= MAX_QUERY);
        }
        assert_eq!(chunks.join("\n"), lines.join("\n"));
    }

    #[test]
    fn the_cache_key_depends_on_the_words_and_the_tongue() {
        let lines = ["こんにちは", "世界"];
        let key = cache_key("en", &lines);
        assert_eq!(key, cache_key("en", &lines));
        assert_ne!(key, cache_key("fr", &lines));
        assert_ne!(key, cache_key("en", &["こんにちは", "世界", "！"]));
        assert_eq!(key.len(), 40);
    }
}
