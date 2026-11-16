//! Real HTTP requests against loopback fixtures; no external service or credential is used.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use futures::{StreamExt, channel::mpsc, executor::block_on};
use rtools::plugins::translate::{
    client::DeepSeekClient,
    core::{
        Completion, Direction, Scenario, TranslateError, TranslateSettings, TranslationRequest,
    },
    credentials::Credential,
};

/// Serves one bounded HTTP request and returns its exact body for assertions.
fn server(
    status: u16,
    response: impl Into<String>,
    extra_headers: &str,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/custom/chat", listener.local_addr().unwrap());
    let extra_headers = extra_headers.to_owned();
    let response = response.into();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
            assert!(bytes.len() < 16384);
        }
        let headers = String::from_utf8(bytes).unwrap();
        assert!(headers.starts_with("POST /custom/chat HTTP/1.1"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{response}", response.len()).unwrap();
        String::from_utf8(body).unwrap()
    });
    (endpoint, handle)
}

/// Drives sender and consumer concurrently, preserving backpressure and all chunks.
fn translate(endpoint: String) -> (Result<Completion, TranslateError>, String) {
    let settings = TranslateSettings {
        endpoint,
        ..Default::default()
    };
    let request = TranslationRequest::new(
        &settings,
        "Hello".into(),
        Direction::EnZh,
        Scenario::General,
    )
    .unwrap();
    let key = Credential::resolve(None, Some("test-only-key".into())).unwrap();
    let client = DeepSeekClient::new().unwrap();
    let (sender, mut receiver) = mpsc::channel(8);
    block_on(async {
        futures::join!(client.translate(request, key, sender), async {
            let mut text = String::new();
            while let Some(chunk) = receiver.next().await {
                text.push_str(&chunk);
            }
            text
        })
    })
}

/// A custom URL receives the exact path and a minimal interoperable payload.
#[test]
fn streams_custom_endpoint() {
    let (endpoint, server) = server(
        200,
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        "",
    );
    let (result, text) = translate(endpoint);
    assert_eq!(result.unwrap(), Completion::Complete);
    assert_eq!(text, "你好");
    let body: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert!(body.get("thinking").is_none());
    assert_eq!(body["messages"][1]["content"], "Hello");
}

/// HTTP failure text cannot expose a reflected API key through an error.
#[test]
fn http_errors_are_safe() {
    for status in [400, 401, 402, 404, 429, 500] {
        let (endpoint, server) = server(status, "test-only-key secret echoed by server", "");
        let (result, _) = translate(endpoint);
        assert_eq!(result.unwrap_err(), TranslateError::Http(status));
        server.join().unwrap();
    }
}

/// A redirect never sends the authorization header to the redirect target.
#[test]
fn refuses_redirects() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let (endpoint, server) = server(
        307,
        "",
        &format!("Location: http://{}/leak\r\n", target.local_addr().unwrap()),
    );
    assert_eq!(
        translate(endpoint).0.unwrap_err(),
        TranslateError::Http(307)
    );
    server.join().unwrap();
    target.set_nonblocking(true).unwrap();
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// A closed socket without a terminal frame preserves output but reports failure.
#[test]
fn interrupted_stream_preserves_partial_text() {
    let (endpoint, server) = server(
        200,
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        "",
    );
    let (result, text) = translate(endpoint);
    assert_eq!(result.unwrap_err(), TranslateError::Interrupted);
    assert_eq!(text, "partial");
    server.join().unwrap();
}

/// A dropped request releases the socket instead of leaving a background translation running.
#[test]
fn dropping_stream_cancels_network_request() {
    use futures::{
        FutureExt as _,
        future::{AbortHandle, Abortable},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/custom/chat", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let headers = String::from_utf8(request).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        socket.read_exact(&mut vec![0; length]).unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n").unwrap();
        let result = socket.read(&mut [0; 1]);
        assert!(
            matches!(result, Ok(0))
                || result.is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionReset)
        );
    });
    let client = DeepSeekClient::new().unwrap();
    let request = TranslationRequest::new(
        &TranslateSettings {
            endpoint,
            ..Default::default()
        },
        "Hello".into(),
        Direction::EnZh,
        Scenario::General,
    )
    .unwrap();
    let (sender, mut receiver) = mpsc::channel(8);
    let key = Credential::resolve(None, Some("test-only-key".into())).unwrap();
    let (cancel, registration) = AbortHandle::new_pair();
    block_on(async {
        let call = Abortable::new(client.translate(request, key, sender), registration).fuse();
        let receive = async {
            assert_eq!(receiver.next().await.as_deref(), Some("partial"));
            cancel.abort();
        };
        let (result, ()) = futures::join!(call, receive);
        assert!(result.is_err());
    });
    server.join().unwrap();
}

/// Typed Rig completion reasons retain output truncation and reject malformed/empty replies.
#[test]
fn response_boundaries() {
    for (wire, expected) in [
        (
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
            Ok(Completion::Truncated),
        ),
        (
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            Err(TranslateError::EmptyResponse),
        ),
        ("data: {invalid}\n\n", Err(TranslateError::Protocol)),
        (
            "data: {\"error\":{\"message\":\"sensitive server text\"}}\n\n",
            Err(TranslateError::Protocol),
        ),
        (
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"unexpected\"}]}\n\ndata: [DONE]\n\n",
            Err(TranslateError::Protocol),
        ),
    ] {
        let (endpoint, handle) = server(200, wire, "");
        assert_eq!(translate(endpoint).0, expected);
        handle.join().unwrap();
    }
}

/// The app bounds translated output even when Rig can decode a larger valid event.
#[test]
fn rejects_oversized_output() {
    let content = "a".repeat(rtools::plugins::translate::core::OUTPUT_LIMIT + 1);
    let event =
        serde_json::json!({"choices": [{"delta": {"content": content}, "finish_reason": "stop"}]});
    let (endpoint, handle) = server(200, format!("data: {event}\n\ndata: [DONE]\n\n"), "");
    assert_eq!(translate(endpoint).0, Err(TranslateError::OutputLimit));
    handle.join().unwrap();
}

/// Rig handles multi-line SSE, CRLF, heartbeat comments and usage-only frames in this integration.
#[test]
fn framed_unicode_and_empty_choices() {
    let wire = ": heartbeat\r\n\r\ndata: {\r\ndata: \"choices\":[{\"delta\":{\"content\":\"你好，世界\"}}]}\r\n\r\ndata: {\"choices\":[]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let (endpoint, handle) = server(200, wire, "");
    let (result, text) = translate(endpoint);
    assert_eq!(result, Ok(Completion::Complete));
    assert_eq!(text, "你好，世界");
    handle.join().unwrap();
}

/// SDK warnings cannot bypass the app's safe error mapping and expose provider response bodies.
#[test]
fn provider_error_body_never_enters_logs() {
    use std::sync::{Arc, Mutex};
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);
    impl Write for LogBuffer {
        /// Captures log bytes in memory without writing provider data to disk.
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        /// In-memory writes are immediately available to assertions.
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let capture = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = Arc::clone(&capture);
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || LogBuffer(Arc::clone(&writer)))
        .finish();
    let (endpoint, handle) = server(
        200,
        "data: {\"error\":{\"message\":\"private-provider-body-test-only-key\"}}\n\n",
        "",
    );
    let (result, _) = tracing::subscriber::with_default(subscriber, || translate(endpoint));
    assert_eq!(result, Err(TranslateError::Protocol));
    handle.join().unwrap();
    let logs = String::from_utf8(capture.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("translation request"));
    assert!(logs.contains("invalid streaming response"));
    assert!(!logs.contains("private-provider-body"));
    assert!(!logs.contains("test-only-key"));
}
