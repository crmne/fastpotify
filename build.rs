//! Embed the target's pinned yt-dlp asset and Windows executable resources.
//! A missing yt-dlp file keeps the build green; a hash mismatch is fatal.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct Manifest {
    version: String,
    assets: Vec<Asset>,
}

#[derive(Clone)]
struct Asset {
    target: String,
    upstream: String,
    sha256: String,
}

fn main() {
    embed_windows_resources();
    embed_ytdlp();
}

fn embed_windows_resources() {
    #[cfg(windows)]
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/fastpotify.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/fastpotify.ico")
            .set("ProductName", "Fastpotify")
            .set("FileDescription", "Fastpotify");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
    }
}

fn embed_ytdlp() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest_path = manifest_dir.join("third_party/yt-dlp/manifest");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-env-changed=FASTPOTIFY_SKIP_YTDLP_BUNDLE");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let generated = out_dir.join("bundled_ytdlp.rs");

    if skip_bundle() {
        write_none(&generated);
        return;
    }

    let Some(target) = embed_target() else {
        write_none(&generated);
        return;
    };

    let manifest = match read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => panic!("yt-dlp manifest: {error}"),
    };
    let Some(asset) = manifest
        .assets
        .iter()
        .find(|asset| asset.target == target)
        .cloned()
    else {
        write_none(&generated);
        return;
    };

    let file = manifest_dir
        .join("third_party/yt-dlp/bin")
        .join(&asset.target)
        .join(&asset.upstream);
    println!("cargo:rerun-if-changed={}", file.display());

    if !file.is_file() {
        println!(
            "cargo:warning=no vendored yt-dlp for {target}; Piped and an external yt-dlp remain available"
        );
        write_none(&generated);
        return;
    }

    let bytes = fs::read(&file).unwrap_or_else(|error| {
        panic!("unable to read vendored yt-dlp {}: {error}", file.display())
    });
    let got = hex_lower(&Sha256::digest(&bytes));
    if got != asset.sha256 {
        panic!(
            "vendored yt-dlp hash mismatch for {target} ({}): expected {}, got {got}",
            file.display(),
            asset.sha256
        );
    }

    write_some(&generated, &manifest.version, &asset.sha256, &file);
}

fn skip_bundle() -> bool {
    matches!(
        env::var("FASTPOTIFY_SKIP_YTDLP_BUNDLE")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

fn embed_target() -> Option<String> {
    let os = env::var("CARGO_CFG_TARGET_OS").ok()?;
    let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
    match (os.as_str(), arch.as_str()) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc".into()),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc".into()),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu".into()),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu".into()),
        ("macos", "aarch64") => Some("aarch64-apple-darwin".into()),
        ("macos", "x86_64") => Some("x86_64-apple-darwin".into()),
        _ => None,
    }
}

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut version = String::new();
    let mut assets = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "version" => version = value.trim().to_string(),
            "asset" => {
                let mut parts = value.split_whitespace();
                let target = parts.next().ok_or("asset line missing target")?.to_string();
                let upstream = parts
                    .next()
                    .ok_or("asset line missing upstream name")?
                    .to_string();
                let sha256 = parts
                    .next()
                    .ok_or("asset line missing sha256")?
                    .trim()
                    .to_ascii_lowercase();
                if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
                    return Err(format!("invalid sha256 for {target}"));
                }
                assets.push(Asset {
                    target,
                    upstream,
                    sha256,
                });
            }
            _ => {}
        }
    }
    if version.is_empty() {
        return Err("manifest is missing version".into());
    }
    Ok(Manifest { version, assets })
}

fn write_none(path: &Path) {
    fs::write(
        path,
        r#"pub const BUNDLED_YTDLP_VERSION: Option<&str> = None;
pub const BUNDLED_YTDLP_SHA256: Option<&str> = None;
pub const BUNDLED_YTDLP_BYTES: Option<&[u8]> = None;
"#,
    )
    .expect("write bundled_ytdlp.rs");
}

fn write_some(path: &Path, version: &str, sha256: &str, file: &Path) {
    let include_path = file
        .to_str()
        .expect("vendored yt-dlp path must be UTF-8")
        .replace('\\', "/");
    let version = escape_rust_string(version);
    let sha256 = escape_rust_string(sha256);
    let body = format!(
        r#"pub const BUNDLED_YTDLP_VERSION: Option<&str> = Some("{version}");
pub const BUNDLED_YTDLP_SHA256: Option<&str> = Some("{sha256}");
pub const BUNDLED_YTDLP_BYTES: Option<&[u8]> = Some(include_bytes!("{include_path}"));
"#
    );
    fs::write(path, body).expect("write bundled_ytdlp.rs");
}

fn escape_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
