//! The HTTP source against a server of one answer on this machine (plain
//! HTTP, which only a debug build allows).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::*;

/// Serves `answer` once; gives back the request it read.
async fn serve(answer: Vec<u8>) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/version.json", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.ends_with(b"\r\n\r\n") {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let _ = stream.write_all(&answer).await;
        let _ = stream.shutdown().await;
        String::from_utf8(request).unwrap()
    });
    (url, server)
}

fn answer(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut answer = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    answer.extend_from_slice(body);
    answer
}

#[tokio::test]
async fn the_file_is_read_and_nothing_about_the_computer_is_sent() {
    let (url, server) = serve(answer("200 OK", "", b"{\"notice\":1}")).await;
    let bytes = HttpSource::new(&url).unwrap().read().await.unwrap();
    assert_eq!(bytes, b"{\"notice\":1}");
    let request = server.await.unwrap().to_ascii_lowercase();
    assert!(
        request.starts_with("get /version.json http/1.1\r\n"),
        "{request}"
    );
    for header in [
        "user-agent",
        "referer",
        "cookie",
        "if-none-match",
        "if-modified-since",
    ] {
        assert!(!request.contains(header), "{header} in {request}");
    }
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let (url, server) = serve(answer(
        "302 Found",
        "Location: http://127.0.0.1:1/elsewhere\r\n",
        b"",
    ))
    .await;
    assert_eq!(
        HttpSource::new(&url).unwrap().read().await,
        Err(ReadError::Refused(302))
    );
    server.await.unwrap();
}

#[tokio::test]
async fn not_found_and_server_errors_are_refusals() {
    for status in ["404 Not Found", "500 Internal Server Error"] {
        let (url, server) = serve(answer(status, "", b"oops")).await;
        let code: u16 = status[..3].parse().unwrap();
        assert_eq!(
            HttpSource::new(&url).unwrap().read().await,
            Err(ReadError::Refused(code))
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn more_than_64_kib_is_not_read() {
    let (url, server) = serve(answer("200 OK", "", &vec![b'x'; MOST_BYTES + 1])).await;
    assert_eq!(
        HttpSource::new(&url).unwrap().read().await,
        Err(ReadError::TooBig)
    );
    server.await.unwrap();
    // Without a length, counted as it comes.
    let mut chunked = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    chunked.extend(vec![b'x'; MOST_BYTES + 1]);
    let (url, server) = serve(chunked).await;
    assert_eq!(
        HttpSource::new(&url).unwrap().read().await,
        Err(ReadError::TooBig)
    );
    server.await.unwrap();
}

#[tokio::test]
async fn no_server_is_no_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/version.json", listener.local_addr().unwrap());
    drop(listener);
    assert!(matches!(
        HttpSource::new(&url).unwrap().read().await,
        Err(ReadError::NoConnection(_))
    ));
}

/// Over the network: TLS to the real site works (until `version.json` is
/// up, a 404 says so as well as the file would).
#[tokio::test]
#[ignore = "needs the network"]
async fn the_real_site_answers_over_https() {
    let read = HttpSource::new(NOTICE_URL).unwrap().read().await;
    assert!(
        matches!(read, Ok(_) | Err(ReadError::Refused(404))),
        "{read:?}"
    );
}
