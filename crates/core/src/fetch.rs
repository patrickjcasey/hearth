//! Downloads for recipe pages and photos, with a timeout and a size cap so a
//! slow or hostile link can't hang an import or exhaust memory.

use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("larger than {} MB", .0 / 1_000_000)]
    TooLarge(u64),
}

/// Downloads `url`, giving up once the body passes `max_bytes`.
pub async fn get(url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
    let mut res = reqwest::Client::builder()
        // Some sites refuse requests without a browser-like user agent.
        .user_agent("Mozilla/5.0 (compatible; Hestia/0.1; +https://github.com/patrickjcasey/hearth)")
        .timeout(TIMEOUT)
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    if res.content_length().is_some_and(|n| n > max_bytes) {
        return Err(FetchError::TooLarge(max_bytes));
    }
    // The length header may be missing or wrong, so count as it arrives.
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if (body.len() + chunk.len()) as u64 > max_bytes {
            return Err(FetchError::TooLarge(max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Serves one canned response on a local port.
    fn serve(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let _ = conn.read(&mut [0; 4096]);
            let _ = conn.write_all(&response);
        });
        format!("http://{addr}/")
    }

    fn response(body: &[u8], content_length: bool) -> Vec<u8> {
        let mut head = "HTTP/1.1 200 OK\r\nconnection: close\r\n".to_string();
        if content_length {
            head += &format!("content-length: {}\r\n", body.len());
        }
        [head.as_bytes(), b"\r\n", body].concat()
    }

    #[tokio::test]
    async fn caps_body_size() {
        let ok = get(&serve(response(b"hello", true)), 10).await.unwrap();
        assert_eq!(ok, b"hello");
        let declared = get(&serve(response(&[b'x'; 11], true)), 10).await;
        assert!(matches!(declared, Err(FetchError::TooLarge(10))), "{declared:?}");
        let undeclared = get(&serve(response(&[b'x'; 100_000], false)), 10).await;
        assert!(matches!(undeclared, Err(FetchError::TooLarge(10))), "{undeclared:?}");
    }
}
