# hyper-proxy2 (patched)

Vendored from [hyper-proxy2 0.1.0](https://github.com/siketyan/hyper-proxy2)
with TLS dependencies moved to rustls 0.23 / hyper-rustls 0.27 so the
CONNECT-then-TLS path does not use rustls-webpki 0.102.

Drop this directory when a hyper-proxy2 release, or a librespot that no
longer depends on it, uses rustls 0.23.
