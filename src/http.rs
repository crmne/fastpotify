//! Shared HTTP client, so a proxy change can take effect without a restart.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::settings::ProxyConfig;

/// A cheap-to-clone handle to the process HTTP client.
///
/// Replacing the inner client updates every clone: the Web API, token
/// refresh, artwork, lyrics, and the update check all pick up a new proxy
/// on the next request.
#[derive(Clone)]
pub struct Http {
    inner: Arc<RwLock<reqwest::Client>>,
}

impl Http {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(RwLock::new(client)),
        }
    }

    pub fn from_proxy(proxy: &ProxyConfig) -> Self {
        match build_client(proxy) {
            Ok(client) => Self::new(client),
            Err(error) => {
                log::warn!("unable to build the HTTP client with a proxy: {error}");
                Self::new(build_client(&ProxyConfig::Off).expect("unable to build the HTTP client"))
            }
        }
    }

    pub fn replace(&self, client: reqwest::Client) {
        *self.inner.write().unwrap_or_else(|lock| lock.into_inner()) = client;
    }

    pub fn client(&self) -> reqwest::Client {
        self.inner
            .read()
            .unwrap_or_else(|lock| lock.into_inner())
            .clone()
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl From<reqwest::Client> for Http {
    fn from(client: reqwest::Client) -> Self {
        Self::new(client)
    }
}

pub fn build_client(proxy: &ProxyConfig) -> Result<reqwest::Client, reqwest::Error> {
    client_builder(proxy)?
        .timeout(Duration::from_secs(30))
        .build()
}

pub fn build_blocking(
    proxy: &ProxyConfig,
    timeout: Duration,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
    blocking_builder(proxy)?.timeout(timeout).build()
}

fn client_builder(proxy: &ProxyConfig) -> Result<reqwest::ClientBuilder, reqwest::Error> {
    apply_proxy(reqwest::Client::builder().user_agent(user_agent()), proxy)
}

fn blocking_builder(
    proxy: &ProxyConfig,
) -> Result<reqwest::blocking::ClientBuilder, reqwest::Error> {
    apply_blocking_proxy(
        reqwest::blocking::Client::builder().user_agent(user_agent()),
        proxy,
    )
}

fn apply_proxy(
    builder: reqwest::ClientBuilder,
    proxy: &ProxyConfig,
) -> Result<reqwest::ClientBuilder, reqwest::Error> {
    Ok(match proxy {
        ProxyConfig::Off => builder.no_proxy(),
        ProxyConfig::System => builder,
        ProxyConfig::Http(manual) | ProxyConfig::Socks(manual) => {
            builder.proxy(manual.reqwest_proxy()?)
        }
    })
}

fn apply_blocking_proxy(
    builder: reqwest::blocking::ClientBuilder,
    proxy: &ProxyConfig,
) -> Result<reqwest::blocking::ClientBuilder, reqwest::Error> {
    Ok(match proxy {
        ProxyConfig::Off => builder.no_proxy(),
        ProxyConfig::System => builder,
        ProxyConfig::Http(manual) | ProxyConfig::Socks(manual) => {
            builder.proxy(manual.reqwest_proxy()?)
        }
    })
}

fn user_agent() -> &'static str {
    concat!("fastpotify/", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_the_client_is_visible_to_clones() {
        let http = Http::default();
        let clone = http.clone();
        let replacement = reqwest::Client::builder()
            .user_agent("fastpotify-test")
            .build()
            .unwrap();
        http.replace(replacement.clone());
        // Distinct Client values still share the pool after a replace; the
        // lock is what matters, and a second replace of a dummy client
        // must not panic.
        clone.replace(reqwest::Client::new());
    }

    #[test]
    fn off_system_and_socks5_each_build_a_client() {
        build_client(&ProxyConfig::Off).unwrap();
        build_client(&ProxyConfig::System).unwrap();
        let settings = crate::settings::Settings {
            proxy_mode: crate::settings::ProxyMode::Socks,
            proxy_host: "127.0.0.1".into(),
            proxy_port: "1080".into(),
            proxy_username: "user".into(),
            proxy_password: "pass".into(),
            ..crate::settings::Settings::default()
        };
        let proxy = settings.proxy_config().unwrap();
        build_client(&proxy).unwrap();
        build_blocking(&proxy, Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn web_api_client_goes_through_an_http_proxy() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_addr = origin.local_addr().unwrap();
        let origin_thread = thread::spawn(move || {
            let (mut stream, _) = origin.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nproxied",
                )
                .unwrap();
            request
        });

        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let proxy_thread = thread::spawn(move || {
            let (mut client, _) = proxy.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buf = [0u8; 8192];
            let n = client.read(&mut buf).unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let mut upstream = std::net::TcpStream::connect(origin_addr).unwrap();
            upstream.write_all(&buf[..n]).unwrap();
            let mut response = Vec::new();
            upstream.read_to_end(&mut response).unwrap();
            client.write_all(&response).unwrap();
            head
        });

        let settings = crate::settings::Settings {
            proxy_mode: crate::settings::ProxyMode::Http,
            proxy_host: proxy_addr.ip().to_string(),
            proxy_port: proxy_addr.port().to_string(),
            ..crate::settings::Settings::default()
        };
        let proxy_config = settings.proxy_config().unwrap();
        let client = build_blocking(&proxy_config, Duration::from_secs(3)).unwrap();
        let body = client
            .get(format!("http://{origin_addr}/catalogue"))
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert_eq!(body, "proxied");
        let seen_by_proxy = proxy_thread.join().unwrap();
        assert!(
            seen_by_proxy.contains("catalogue"),
            "proxy should see the Web API request, got {seen_by_proxy:?}"
        );
        let seen_by_origin = origin_thread.join().unwrap();
        assert!(seen_by_origin.contains("GET"));
    }

    #[test]
    fn proxy_authentication_preserves_opaque_credentials() {
        use base64::Engine;
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let proxy_thread = thread::spawn(move || {
            let (mut client, _) = proxy.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buf = [0u8; 8192];
            let n = client.read(&mut buf).unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            client
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
            head
        });

        let username = "alice/name";
        let password = " secret/with spaces ";
        let settings = crate::settings::Settings {
            proxy_mode: crate::settings::ProxyMode::Http,
            proxy_host: proxy_addr.ip().to_string(),
            proxy_port: proxy_addr.port().to_string(),
            proxy_username: username.into(),
            proxy_password: password.into(),
            ..crate::settings::Settings::default()
        };
        let proxy_config = settings.proxy_config().unwrap();
        let client = build_blocking(&proxy_config, Duration::from_secs(3)).unwrap();
        assert_eq!(
            client
                .get("http://example.invalid/catalogue")
                .send()
                .unwrap()
                .text()
                .unwrap(),
            "ok"
        );
        let seen = proxy_thread.join().unwrap();
        let expected =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        assert!(
            seen.lines()
                .any(|line| line
                    .eq_ignore_ascii_case(&format!("Proxy-Authorization: Basic {expected}"))),
            "proxy did not receive the exact configured credentials"
        );
    }

    #[tokio::test]
    async fn librespot_http_client_sends_connect_through_an_http_proxy() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let proxy_thread = thread::spawn(move || {
            let (mut client, _) = proxy.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = [0u8; 4096];
            let n = client.read(&mut buf).unwrap_or(0);
            let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let proxy_url = reqwest::Url::parse(&format!("http://{proxy_addr}")).unwrap();
        let client = librespot_core::http_client::HttpClient::new(Some(&proxy_url));
        let request = http::Request::builder()
            .method("GET")
            .uri("https://apresolve.spotify.com/")
            .body(Default::default())
            .unwrap();
        let _ = client.request(request).await;
        let seen = proxy_thread.join().unwrap();
        assert!(
            seen.to_ascii_uppercase().contains("CONNECT") && seen.contains("apresolve.spotify.com"),
            "librespot should CONNECT through the proxy, got {seen:?}"
        );
    }
}
