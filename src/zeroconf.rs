//! Spotify Connect receivers on the local network.
//!
//! Spotify's Web API lists only devices already signed in. Receivers waiting
//! for an account are therefore absent from `/me/player/devices`.
//!
//! Receivers advertise `_spotify-connect._tcp` over mDNS. `getInfo` returns
//! device details and a Diffie-Hellman public key; `addUser` sends encrypted
//! account data. After sign-in, the receiver appears in the normal device list.
//!
//! The transferred librespot credential is encrypted to the receiver's key,
//! never written, and never logged.

use std::collections::HashMap;
use std::time::Duration;

use aes::cipher::{BlockEncrypt, KeyInit, KeyIvInit, StreamCipher};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use librespot_core::diffie_hellman::DhLocalKeys;
use serde::Deserialize;
use sha1::{Digest, Sha1};

const SERVICE: &str = "_spotify-connect._tcp.local.";
/// Spotify's ZeroConf login uses this fixed initialisation vector.
const FIXED_IV: [u8; 16] = [
    253, 81, 222, 19, 70, 203, 45, 89, 141, 68, 210, 240, 93, 20, 76, 30,
];
const HTTP_TIMEOUT: Duration = Duration::from_secs(6);

type HmacSha1 = Hmac<Sha1>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// A receiver found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receiver {
    /// The name it advertises, e.g. "House Spotify".
    pub name: String,
    pub address: std::net::IpAddr,
    pub port: u16,
    /// Path its HTTP interface answers on, from the `CPath` TXT record.
    pub path: String,
}

impl Receiver {
    fn url(&self, query: &str) -> String {
        let host = match self.address {
            std::net::IpAddr::V6(address) => format!("[{address}]"),
            std::net::IpAddr::V4(address) => address.to_string(),
        };
        let path = if self.path.starts_with('/') {
            self.path.clone()
        } else {
            format!("/{}", self.path)
        };
        format!("http://{host}:{}{path}{query}", self.port)
    }
}

/// What a receiver says about itself.
#[derive(Clone, Debug, Deserialize)]
pub struct Info {
    #[serde(rename = "deviceID", default)]
    pub device_id: String,
    #[serde(rename = "remoteName", default)]
    pub remote_name: String,
    #[serde(rename = "deviceType", default)]
    pub device_type: String,
    #[serde(rename = "publicKey", default)]
    pub public_key: String,
    #[serde(rename = "tokenType", default)]
    pub token_type: String,
    #[serde(rename = "activeUser", default)]
    pub active_user: String,
    #[serde(default)]
    pub version: String,
}

impl Info {
    /// Spotify's device kinds are capitalised; the Web API reports lowercase.
    pub fn kind(&self) -> String {
        self.device_type.to_lowercase()
    }
}

#[derive(Deserialize)]
struct AddUserReply {
    #[serde(default)]
    status: i64,
    #[serde(rename = "statusString", default)]
    status_string: String,
}

/// The account credential handed to a receiver.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub auth_type: i64,
    pub auth_data: Vec<u8>,
}

impl Credentials {
    /// Reads the reusable credential librespot stored for local playback.
    pub fn load(credentials_dir: &std::path::Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct Stored {
            username: String,
            auth_type: i64,
            auth_data: String,
        }
        let path = credentials_dir.join("credentials.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("no stored playback credential at {}", path.display()))?;
        let stored: Stored =
            serde_json::from_str(&text).context("stored playback credential is unreadable")?;
        Ok(Self {
            username: stored.username,
            auth_type: stored.auth_type,
            auth_data: BASE64
                .decode(stored.auth_data)
                .context("stored playback credential is malformed")?,
        })
    }
}

/// Browses the network for receivers. Blocking, bounded by `timeout`.
pub fn discover(timeout: Duration) -> Result<Vec<Receiver>> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let daemon = ServiceDaemon::new().context("cannot browse the local network")?;
    let events = daemon
        .browse(SERVICE)
        .context("cannot browse for Spotify receivers")?;
    let deadline = std::time::Instant::now() + timeout;
    let mut found: HashMap<String, Receiver> = HashMap::new();

    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match events.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                // Prefer IPv4: link-local IPv6 needs a scope id to dial.
                let mut addresses: Vec<std::net::IpAddr> = service
                    .get_addresses()
                    .iter()
                    .map(|scoped| scoped.to_ip_addr())
                    .collect();
                addresses.sort_by_key(|address| address.is_ipv6());
                let Some(address) = addresses.into_iter().find(|address| match address {
                    std::net::IpAddr::V6(address) => !address.is_unicast_link_local(),
                    std::net::IpAddr::V4(_) => true,
                }) else {
                    continue;
                };
                let path = service
                    .get_property_val_str("CPath")
                    .filter(|path| !path.is_empty())
                    .unwrap_or("/")
                    .to_string();
                let name =
                    unescape_instance(service.get_fullname().split('.').next().unwrap_or_default());
                found.insert(
                    service.get_fullname().to_string(),
                    Receiver {
                        name,
                        address,
                        port: service.get_port(),
                        path,
                    },
                );
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    let mut receivers: Vec<Receiver> = found.into_values().collect();
    receivers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(receivers)
}

/// Asks a receiver to describe itself.
pub fn get_info(http: &reqwest::blocking::Client, receiver: &Receiver) -> Result<Info> {
    let response = http
        .get(receiver.url("?action=getInfo"))
        .timeout(HTTP_TIMEOUT)
        .send()
        .context("receiver did not answer")?;
    if !response.status().is_success() {
        bail!("receiver answered {}", response.status());
    }
    response.json().context("receiver sent an unreadable reply")
}

/// Hands an account to a receiver so it logs in and joins Spotify Connect.
pub fn add_user(
    http: &reqwest::blocking::Client,
    receiver: &Receiver,
    info: &Info,
    credentials: &Credentials,
    our_name: &str,
) -> Result<()> {
    let (blob, client_key) = login_blob(credentials, &info.device_id, &info.public_key)?;
    let our_device_id = hex(&Sha1::digest(our_name.as_bytes()));
    let form = [
        ("action", "addUser".to_string()),
        (
            "version",
            if info.version.is_empty() {
                "2.9.0".into()
            } else {
                info.version.clone()
            },
        ),
        ("tokenType", "default".to_string()),
        ("clientKey", client_key),
        ("loginId", credentials.username.clone()),
        ("userName", credentials.username.clone()),
        ("blob", blob),
        ("deviceName", our_name.to_string()),
        ("deviceId", our_device_id),
    ];
    let response = http
        .post(receiver.url(""))
        .timeout(HTTP_TIMEOUT)
        .form(&form)
        .send()
        .context("receiver did not accept the connection")?;
    let reply: AddUserReply = response
        .json()
        .context("receiver sent an unreadable reply")?;
    // 101 is this interface's "OK".
    if reply.status == 101 {
        return Ok(());
    }
    let detail = if reply.status_string.is_empty() {
        format!("status {}", reply.status)
    } else {
        reply.status_string.clone()
    };
    Err(anyhow!("receiver refused the connection ({detail})"))
}

/// Builds the encrypted login blob and our Diffie-Hellman public key.
///
/// The inner blob carries the account and its reusable credential, wrapped in
/// a key derived from the receiver's own device id, so a blob captured from
/// the network is useless to anything but that receiver. That is then
/// encrypted again to the key both sides derive from the exchange.
fn login_blob(
    credentials: &Credentials,
    receiver_device_id: &str,
    receiver_public_key: &str,
) -> Result<(String, String)> {
    let remote_key = BASE64
        .decode(receiver_public_key)
        .context("receiver public key is malformed")?;
    if remote_key.is_empty() {
        bail!("receiver offered no public key");
    }

    let mut blob: Vec<u8> = Vec::new();
    write_int(&mut blob, 0x49);
    write_bytes(&mut blob, credentials.username.as_bytes());
    write_int(&mut blob, 0x50);
    write_int(&mut blob, credentials.auth_type as u32);
    write_int(&mut blob, 0x51);
    write_bytes(&mut blob, &credentials.auth_data);
    let padding = 16 - (blob.len() % 16) - 1;
    blob.extend(std::iter::repeat_n(0u8, padding));
    blob.push(padding as u8 + 1);
    // Spotify's own obfuscation pass over the padded blob.
    let length = blob.len();
    for index in (0..=length.saturating_sub(0x11)).rev() {
        blob[length - index - 1] ^= blob[length - index - 0x11];
    }

    let secret = Sha1::digest(receiver_device_id.as_bytes());
    let mut derived = [0u8; 20];
    pbkdf2::pbkdf2_hmac::<Sha1>(
        &secret,
        credentials.username.as_bytes(),
        0x100,
        &mut derived,
    );
    let mut blob_key = Sha1::digest(derived).to_vec();
    blob_key.extend_from_slice(&[0, 0, 0, 20]);

    // AES-192-ECB, because the key is the 20-byte digest plus its length.
    let cipher = aes::Aes192::new_from_slice(&blob_key)
        .map_err(|_| anyhow!("could not prepare the credential cipher"))?;
    let mut encrypted_blob = blob.clone();
    let (blocks, _) = encrypted_blob.as_chunks_mut::<16>();
    for block in blocks {
        cipher.encrypt_block(block.into());
    }
    let encoded_blob = BASE64.encode(&encrypted_blob);

    let keys = DhLocalKeys::random(&mut rand::rng());
    let shared = keys.shared_secret(&remote_key);
    let base_key = &Sha1::digest(&shared)[..16];
    let encryption_key = {
        let mut mac =
            <HmacSha1 as Mac>::new_from_slice(base_key).expect("HMAC accepts any key length");
        mac.update(b"encryption");
        mac.finalize().into_bytes()
    };
    let mut encrypted = encoded_blob.into_bytes();
    let mut stream = Aes128Ctr::new_from_slices(&encryption_key[..16], &FIXED_IV)
        .map_err(|_| anyhow!("could not prepare the transport cipher"))?;
    stream.apply_keystream(&mut encrypted);

    let checksum = {
        let checksum_key = {
            let mut mac =
                <HmacSha1 as Mac>::new_from_slice(base_key).expect("HMAC accepts any key length");
            mac.update(b"checksum");
            mac.finalize().into_bytes()
        };
        let mut mac =
            <HmacSha1 as Mac>::new_from_slice(&checksum_key).expect("HMAC accepts any key length");
        mac.update(&encrypted);
        mac.finalize().into_bytes()
    };

    let mut signed = FIXED_IV.to_vec();
    signed.extend_from_slice(&encrypted);
    signed.extend_from_slice(&checksum);
    Ok((BASE64.encode(signed), BASE64.encode(keys.public_key())))
}

/// mDNS escapes bytes in an instance name as `\\ddd`, so "House Spotify"
/// arrives as "House\\032Spotify".
fn unescape_instance(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut characters = name.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        let digits: String = characters.clone().take(3).collect();
        match digits.parse::<u8>() {
            Ok(byte) if digits.len() == 3 => {
                out.push(byte as char);
                characters.nth(2);
            }
            _ => out.push(character),
        }
    }
    out
}

/// Spotify's variable-length integer.
fn write_int(out: &mut Vec<u8>, value: u32) {
    if value < 0x80 {
        out.push(value as u8);
    } else {
        out.push(0x80 | (value & 0x7F) as u8);
        out.push((value >> 7) as u8);
    }
}

fn write_bytes(out: &mut Vec<u8>, value: &[u8]) {
    write_int(out, value.len() as u32);
    out.extend_from_slice(value);
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_integers_match_the_wire_format() {
        let mut out = Vec::new();
        write_int(&mut out, 0x49);
        assert_eq!(out, vec![0x49]);
        out.clear();
        write_int(&mut out, 0x100);
        assert_eq!(out, vec![0x80, 0x02]);
        out.clear();
        write_bytes(&mut out, b"abc");
        assert_eq!(out, vec![3, b'a', b'b', b'c']);
    }

    /// The blob is deterministic apart from the ephemeral key, so the same
    /// account and receiver must always produce the same length and a
    /// well-formed envelope: fixed IV, ciphertext, then a 20-byte checksum.
    #[test]
    fn login_blob_has_the_expected_envelope() {
        let credentials = Credentials {
            username: "someone".into(),
            auth_type: 1,
            auth_data: vec![7u8; 80],
        };
        let receiver_key = BASE64.encode(vec![9u8; 96]);
        let (blob, client_key) =
            login_blob(&credentials, "abcdef0123456789", &receiver_key).unwrap();
        let decoded = BASE64.decode(blob).unwrap();
        assert_eq!(&decoded[..16], &FIXED_IV);
        assert!(decoded.len() > 16 + 20);
        assert!(!client_key.is_empty());
        assert!(BASE64.decode(client_key).is_ok());
    }

    #[test]
    fn instance_names_are_unescaped() {
        assert_eq!(unescape_instance("House\\032Spotify"), "House Spotify");
        assert_eq!(unescape_instance("Kitchen"), "Kitchen");
        assert_eq!(unescape_instance("odd\\x"), "odd\\x");
    }

    #[test]
    fn receiver_urls_bracket_ipv6() {
        let receiver = Receiver {
            name: "House".into(),
            address: "192.168.8.166".parse().unwrap(),
            port: 5907,
            path: "/".into(),
        };
        assert_eq!(
            receiver.url("?action=getInfo"),
            "http://192.168.8.166:5907/?action=getInfo"
        );
        let receiver = Receiver {
            address: "fe80::1".parse().unwrap(),
            path: "zc".into(),
            ..receiver
        };
        assert_eq!(receiver.url(""), "http://[fe80::1]:5907/zc");
    }
}
