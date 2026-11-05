//! A computer whose network goes through a proxy (ADR 0033: "the proxy the
//! environment names is used"): the look for a new version asks the proxy
//! named by `HTTP_PROXY` or `HTTPS_PROXY`, and sends it nothing about the
//! computer either.
//! Its own test binary: the proxy is named in the process's environment.

use ourstick_core::new_version::{HttpSource, NoticeSource};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn the_look_goes_through_the_proxy_the_environment_names() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    // The only test of this binary: no other thread reads the environment.
    std::env::set_var("HTTP_PROXY", &proxy_url);
    std::env::set_var("http_proxy", &proxy_url);
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let body = br#"{"notice":"x","signature":"y"}"#;
    let server = tokio::spawn(async move {
        let (mut stream, _) = proxy.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.ends_with(b"\r\n\r\n") {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let answer = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(answer.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let _ = stream.shutdown().await;
        String::from_utf8(request).unwrap()
    });

    // A name that resolves nowhere: only the proxy can answer for it.
    let source = HttpSource::new("http://version.ourstick.invalid/version.json").unwrap();
    let read = source.read().await.unwrap();
    assert_eq!(read, body);

    let request = server.await.unwrap();
    let first = request.lines().next().unwrap();
    assert_eq!(
        first,
        "GET http://version.ourstick.invalid/version.json HTTP/1.1"
    );
    let lower = request.to_ascii_lowercase();
    for header in ["user-agent", "cookie", "referer", "proxy-authorization"] {
        assert!(!lower.contains(&format!("\n{header}:")), "{header} sent");
    }

    // The real file, over HTTPS: a tunnel through the proxy (`CONNECT`), so
    // the proxy sees only where, never what.
    let tunnel = TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var(
        "HTTPS_PROXY",
        format!("http://{}", tunnel.local_addr().unwrap()),
    );
    std::env::set_var(
        "https_proxy",
        format!("http://{}", tunnel.local_addr().unwrap()),
    );
    let asked = tokio::spawn(async move {
        let (mut stream, _) = tunnel.accept().await.unwrap();
        let mut request = vec![0u8; 1024];
        let read = stream.read(&mut request).await.unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let source = HttpSource::new("https://ourstick.com/version.json").unwrap();
    assert!(
        source.read().await.is_err(),
        "the closed tunnel gives nothing"
    );
    let asked = asked.await.unwrap();
    assert!(
        asked.starts_with("CONNECT ourstick.com:443 HTTP/1.1"),
        "{}",
        asked.lines().next().unwrap_or_default()
    );
}
