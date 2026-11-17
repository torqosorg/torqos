//! The proxy against a stand-in for the model's service: a TLS server on this machine with a certificate the
//! tests' own authority signed (tests/fixtures), which answers as scripted and reports the bytes it was sent.
//! The caller is this test's own account, named by the kernel's table as in a real machine: the tests make it
//! alice's agent, or no one's.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use torq_model_proxy::{agents_by_uid, policy::Policy, record, tls, Agent, Dial, Proxy, Upstream};
use torq_wire::activity::{Entry, Item, Line, Outcome, Tokens, Why};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// What the stand-in was sent: the request's head as it arrived, and its body.
struct Got {
    head: String,
    body: Vec<u8>,
}

/// How the stand-in answers one request.
enum Script {
    /// A whole answer: its head's extra lines and its body.
    Whole(&'static str, Vec<u8>),
    /// A stream: the first piece at once, the rest when the gate opens (or the connection dropped if it closes).
    Stream { head: &'static str, first: Vec<u8>, gate: oneshot::Receiver<()>, rest: Vec<Vec<u8>> },
}

/// The stand-in for the model's service, with the certificate `cert` and its key. Each connection takes the
/// next script.
async fn model(cert: &str, key: &str, scripts: Vec<Script>) -> (SocketAddr, mpsc::UnboundedReceiver<Got>) {
    let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(fixture(cert)).unwrap().map(Result::unwrap).collect();
    let key = PrivateKeyDer::from_pem_file(fixture(key)).unwrap();
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(certs, key).unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let scripts = Arc::new(Mutex::new(VecDeque::from(scripts)));
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let (acceptor, tx, scripts) = (acceptor.clone(), tx.clone(), scripts.clone());
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(tcp).await else { return };
                let mut buf = Vec::new();
                let head_end = loop {
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                    let mut piece = [0u8; 4096];
                    let n = s.read(&mut piece).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&piece[..n]);
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let length: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                let mut body = buf[head_end..].to_vec();
                while body.len() < length {
                    let mut piece = [0u8; 4096];
                    let n = s.read(&mut piece).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&piece[..n]);
                }
                let _ = tx.send(Got { head, body });
                let script = scripts.lock().unwrap().pop_front();
                match script {
                    Some(Script::Whole(head, body)) => {
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\n{head}content-length: {}\r\n\r\n", body.len()).as_bytes()).await;
                        let _ = s.write_all(&body).await;
                    }
                    Some(Script::Stream { head, first, gate, rest }) => {
                        let chunk = |b: &[u8]| [format!("{:x}\r\n", b.len()).as_bytes(), b, b"\r\n"].concat();
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\n{head}transfer-encoding: chunked\r\n\r\n").as_bytes()).await;
                        let _ = s.write_all(&chunk(&first)).await;
                        let _ = s.flush().await;
                        if gate.await.is_err() {
                            return;
                        }
                        for piece in rest {
                            let _ = s.write_all(&chunk(&piece)).await;
                        }
                        let _ = s.write_all(b"0\r\n\r\n").await;
                    }
                    None => {}
                }
                let _ = s.shutdown().await;
            });
        }
    });
    (addr, rx)
}

/// This test's own uid, as the kernel's table will name its connections.
fn me() -> u32 {
    std::os::unix::fs::MetadataExt::uid(&std::fs::metadata("/proc/self").unwrap())
}

/// A way out for the tests: every site is the one stand-in at `to`; `forbidden` is a name whose address may
/// not be reached.
fn dial_to(to: Option<SocketAddr>) -> Dial {
    Arc::new(move |host, _port, _listed| {
        Box::pin(async move {
            if host == "forbidden.example" {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
            match to {
                Some(a) => TcpStream::connect(a).await,
                None => Err(std::io::ErrorKind::ConnectionRefused.into()),
            }
        })
    })
}

struct Running {
    at: SocketAddr,
    record: tempfile::TempDir,
    proxy: Arc<Proxy>,
}

/// The proxy, with this test's account as alice's agent (or as no one's), the model at `model`.
async fn proxy(model: SocketAddr, an_agent: bool, policy: Policy, dial: Dial) -> Running {
    let record = tempfile::tempdir().unwrap();
    for person in ["alice", "bob"] {
        std::fs::create_dir(record.path().join(person)).unwrap();
    }
    let agents: HashMap<u32, Agent> = if an_agent { [(me(), Agent { account: "torq-agent-alice".into(), person: "alice".into() })].into() } else { HashMap::new() };
    let proxy = Arc::new(Proxy {
        agents,
        policy,
        upstream: Upstream { name: "model.test".into(), at: model.to_string(), tls: tls(&fixture("ca.pem")).unwrap() },
        recorder: record::Recorder::new(record.path().into(), record::Bounds::default()),
        dial,
        busy: Mutex::default(),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(proxy.clone().serve(listener));
    Running { at, record, proxy }
}

fn closed() -> Policy {
    Policy { hosts: vec![], web: false, model: "model.test".into() }
}

/// A chunked body, as its bytes.
fn unchunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
        let n = usize::from_str_radix(std::str::from_utf8(&b[..i]).unwrap().trim(), 16).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[i + 2 + n + 2..];
    }
    out
}

/// An answer split into its head and its body (unchunked where it was chunked).
fn parts(raw: &[u8]) -> (String, Vec<u8>) {
    let i = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("an answer's head");
    let head = String::from_utf8_lossy(&raw[..i + 4]).into_owned();
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { unchunk(&raw[i + 4..]) } else { raw[i + 4..].to_vec() };
    (head, body)
}

/// One request sent to the proxy as these bytes, and everything that came back.
async fn ask(at: SocketAddr, raw: &[u8]) -> (String, Vec<u8>) {
    let mut c = TcpStream::connect(at).await.unwrap();
    c.write_all(raw).await.unwrap();
    let mut back = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), c.read_to_end(&mut back)).await.expect("an answer in time").unwrap();
    parts(&back)
}

/// A person's record: every line of every file, oldest first.
fn lines(record: &Path, person: &str) -> Vec<Entry> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(record.join(person)).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    names.iter().flat_map(|n| std::fs::read_to_string(n).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect::<Vec<Entry>>()).collect()
}

/// Everything the record holds, as text: to look for what must never be in it.
fn everything(record: &Path) -> String {
    let mut all = String::new();
    for person in std::fs::read_dir(record).unwrap() {
        for file in std::fs::read_dir(person.unwrap().path()).unwrap() {
            all.push_str(&std::fs::read_to_string(file.unwrap().path()).unwrap());
        }
    }
    all
}

const QUESTION: &str = r#"{"model":"claude-test-1","stream":true,"messages":[{"role":"user","content":"an older question"},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"hostname"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"torq5"}]}]}"#;

/// A request as Claude Code makes one, with every kind of credential a request can carry, and a caller's own
/// claims about who it is.
fn request(body: &str) -> Vec<u8> {
    format!("POST /v1/messages?beta=true&key=SECRET-IN-QUERY HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer SECRET-TOKEN\r\nx-api-key: SECRET-KEY\r\nCookie: session=SECRET-COOKIE\r\nProxy-Authorization: Basic SECRET-PROXY\r\nanthropic-version: 2023-06-01\r\nanthropic-beta: oauth-2025-04-20,another\r\nX-Mixed-Case: Kept As It Came\r\nx-claude-code-session-id: sess-0fa3\r\nx-claude-code-agent-id: helper-2\r\nx-claude-code-parent-agent-id: helper-1\r\nX-Torq-Person: bob\r\nUser: torq-agent-bob\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

const FIRST: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9,\"cache_read_input_tokens\":100,\"output_tokens\":1}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"The machine \"}}\n\n";
const REST: [&str; 2] = [
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"is torq5.\"}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t2\",\"name\":\"Read\",\"input\":{}}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"file_path\\\":\\\"/etc/os-release\\\"}\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":21}}\n\n",
];

fn stream(gate: oneshot::Receiver<()>) -> Script {
    Script::Stream { head: "content-type: text/event-stream; charset=utf-8\r\nRequest-Id: req_1\r\nset-cookie: SECRET-SET-COOKIE\r\n", first: FIRST.into(), gate, rest: REST.iter().map(|r| r.as_bytes().to_vec()).collect() }
}

#[tokio::test]
async fn a_request_passes_as_it_came_the_answer_streams_back_and_one_line_records_it() {
    let (open, gate) = oneshot::channel();
    let (model, mut got) = model("model.pem", "model.key", vec![stream(gate)]).await;
    let p = proxy(model, true, closed(), dial_to(None)).await;

    let mut c = TcpStream::connect(p.at).await.unwrap();
    c.write_all(&request(QUESTION)).await.unwrap();

    // What the service was sent: the same request, under its own name.
    let sent = tokio::time::timeout(Duration::from_secs(20), got.recv()).await.unwrap().unwrap();
    assert!(sent.head.starts_with("POST /v1/messages?beta=true&key=SECRET-IN-QUERY HTTP/1.1\r\n"), "the path and its query as they came: {}", sent.head);
    for passed in ["Authorization: Bearer SECRET-TOKEN\r\n", "x-api-key: SECRET-KEY\r\n", "Cookie: session=SECRET-COOKIE\r\n", "anthropic-version: 2023-06-01\r\n", "anthropic-beta: oauth-2025-04-20,another\r\n", "X-Mixed-Case: Kept As It Came\r\n", "x-claude-code-session-id: sess-0fa3\r\n", "Content-Type: application/json\r\n"] {
        assert!(sent.head.contains(passed), "passed unchanged, its name as it was written: {passed:?} in {}", sent.head);
    }
    assert!(sent.head.to_ascii_lowercase().contains("\r\nhost: model.test\r\n"), "asked by the service's own name: {}", sent.head);
    assert!(!sent.head.contains("127.0.0.1"));
    for dropped in ["Proxy-Authorization", "SECRET-PROXY", "Connection:"] {
        assert!(!sent.head.contains(dropped), "what belongs to the one connection is not passed on: {dropped}");
    }
    assert_eq!(sent.body, QUESTION.as_bytes(), "the body, byte for byte");

    // The answer's first piece arrives while the service has not yet written the rest: nothing waits for the end.
    let mut back = Vec::new();
    let streamed = tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&back).contains("The machine ") {
            let mut piece = [0u8; 4096];
            let n = c.read(&mut piece).await.unwrap();
            assert!(n > 0, "the connection ended before the first piece");
            back.extend_from_slice(&piece[..n]);
        }
    }).await;
    assert!(streamed.is_ok(), "the first piece of the answer did not arrive while the rest was still to come: the answer is held back");
    assert!(lines(p.record.path(), "alice").is_empty(), "the line is written when the answer has ended");
    open.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), c.read_to_end(&mut back)).await.unwrap().unwrap();
    let (head, body) = parts(&back);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Request-Id: req_1\r\n") && head.contains("content-type: text/event-stream; charset=utf-8\r\n"), "the answer's headers as they came: {head}");
    assert_eq!(String::from_utf8(body).unwrap(), format!("{FIRST}{}{}", REST[0], REST[1]), "the answer, byte for byte");

    // The record: one line, alice's, whatever the request said of itself.
    let record = lines(p.record.path(), "alice");
    assert_eq!(record.len(), 1);
    let Line::Request(r) = &record[0].line else { panic!("a request's line: {:?}", record[0]) };
    assert_eq!((r.agent.as_str(), r.person.as_str()), ("torq-agent-alice", "alice"), "named by the kernel");
    assert!(lines(p.record.path(), "bob").is_empty(), "nothing is bob's because a header said bob");
    assert_eq!((r.session.as_deref(), r.agent_id.as_deref(), r.parent_agent_id.as_deref()), (Some("sess-0fa3"), Some("helper-2"), Some("helper-1")));
    assert_eq!((r.method.as_str(), r.path.as_str(), r.model.as_deref()), ("POST", "/v1/messages", Some("claude-test-1")));
    assert_eq!((r.outcome, r.status, r.sent), (Outcome::Complete, Some(200), QUESTION.len() as u64));
    assert_eq!(r.received as usize, FIRST.len() + REST[0].len() + REST[1].len());
    assert_eq!(r.tokens, Some(Tokens { input: 9, cache_read: 100, cache_write: 0, output: 21 }));
    assert_eq!(r.new, [Item::ToolResult { tool: Some("Bash".into()), text: "torq5".into(), error: false, cut_from: None }], "what was new: the tool's result, with its name");
    assert_eq!(r.answer, [Item::Text { text: "The machine is torq5.".into(), cut_from: None }, Item::ToolCall { tool: "Read".into(), input: "{\"file_path\":\"/etc/os-release\"}".into(), cut_from: None }]);
    assert_eq!(r.stop.as_deref(), Some("tool_use"));
    assert!(r.unread.is_empty());

    // And never a credential, in any form a request or an answer carried one.
    let all = everything(p.record.path());
    assert!(!all.contains("SECRET"), "a credential is in the record: {all}");
    assert!(!all.to_ascii_lowercase().contains("authorization") && !all.to_ascii_lowercase().contains("cookie") && !all.contains("api-key"));
}

#[tokio::test]
async fn a_caller_that_is_not_an_agents_account_is_refused_and_nothing_is_sent_or_recorded() {
    let (model, mut got) = model("model.pem", "model.key", vec![Script::Whole("content-type: application/json\r\n", b"{}".to_vec())]).await;
    let p = proxy(model, false, closed(), dial_to(Some(model))).await;
    let (head, body) = ask(p.at, &request(QUESTION)).await;
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("the agents' accounts"));
    let (head, _) = ask(p.at, b"CONNECT example.org:443 HTTP/1.1\r\nHost: example.org:443\r\nConnection: close\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 403 "), "nor a site: {head}");
    assert!(tokio::time::timeout(Duration::from_millis(300), got.recv()).await.is_err(), "the service was sent nothing");
    assert_eq!(everything(p.record.path()), "");
}

#[tokio::test]
async fn a_service_whose_certificate_no_authority_signed_is_not_spoken_to() {
    // A stranger in the middle: a certificate for the right name, from an authority the machine does not trust.
    let (stranger, mut got) = model("stranger.pem", "stranger.key", vec![Script::Whole("content-type: application/json\r\n", b"{}".to_vec())]).await;
    let p = proxy(stranger, true, closed(), dial_to(None)).await;
    let (head, body) = ask(p.at, &request(QUESTION)).await;
    let said = String::from_utf8_lossy(&body).into_owned();
    assert!(head.starts_with("HTTP/1.1 502 "), "{head}");
    assert!(said.contains("certificate") && said.contains("model.test"), "{said}");
    assert!(!said.contains("SECRET"), "the refusal carries no credential: {said}");
    assert!(tokio::time::timeout(Duration::from_millis(300), got.recv()).await.is_err(), "not one byte of the request reached it");
    let record = lines(p.record.path(), "alice");
    let Line::Request(r) = &record[0].line else { panic!() };
    assert_eq!((r.outcome, r.status), (Outcome::NotReached, None));
    assert!(!everything(p.record.path()).contains("SECRET"));

    // The right authority, the wrong name: a certificate for `model.test` shown by what is asked as another.
    let (model, mut got) = model("model.pem", "model.key", vec![]).await;
    let mut p = proxy(model, true, closed(), dial_to(None)).await;
    let other = Arc::new(Proxy {
        agents: p.proxy.agents.clone(),
        policy: closed(),
        upstream: Upstream { name: "another.test".into(), at: model.to_string(), tls: tls(&fixture("ca.pem")).unwrap() },
        recorder: record::Recorder::new(p.record.path().into(), record::Bounds::default()),
        dial: dial_to(None),
        busy: Mutex::default(),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    p.at = listener.local_addr().unwrap();
    tokio::spawn(other.serve(listener));
    let (head, _) = ask(p.at, &request(QUESTION)).await;
    assert!(head.starts_with("HTTP/1.1 502 "), "{head}");
    assert!(tokio::time::timeout(Duration::from_millis(300), got.recv()).await.is_err());
}

#[tokio::test]
async fn an_engine_that_goes_away_mid_answer_leaves_a_line_that_says_so() {
    let (_open, gate) = oneshot::channel::<()>();
    let (model, _got) = model("model.pem", "model.key", vec![stream(gate)]).await;
    let p = proxy(model, true, closed(), dial_to(None)).await;
    let mut c = TcpStream::connect(p.at).await.unwrap();
    c.write_all(&request(QUESTION)).await.unwrap();
    let mut back = Vec::new();
    while !String::from_utf8_lossy(&back).contains("The machine ") {
        let mut piece = [0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(10), c.read(&mut piece)).await.unwrap().unwrap();
        back.extend_from_slice(&piece[..n]);
    }
    drop(c);
    let record = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let l = lines(p.record.path(), "alice");
            if !l.is_empty() {
                return l;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("a line, though the answer never ended");
    let Line::Request(r) = &record[0].line else { panic!() };
    assert_eq!(r.outcome, Outcome::Interrupted);
    assert_eq!(r.answer, [Item::Text { text: "The machine ".into(), cut_from: None }], "what had passed is kept");
    assert_eq!(r.new.len(), 1);
}

#[tokio::test]
async fn anything_else_asked_of_the_service_is_passed_and_recorded_but_not_read() {
    let answer = br#"{"content":[{"type":"text","text":"not a message"}],"usage":{"input_tokens":5}}"#.to_vec();
    let (model, mut got) = model("model.pem", "model.key", vec![Script::Whole("content-type: application/json\r\n", answer.clone()), Script::Whole("content-type: application/json\r\ncontent-encoding: br\r\n", b"\x0b\x02\x80{}\x03".to_vec())]).await;
    let p = proxy(model, true, closed(), dial_to(None)).await;
    let (head, body) = ask(p.at, b"POST /v1/files HTTP/1.1\r\nHost: x\r\nx-api-key: SECRET-KEY\r\nConnection: close\r\nContent-Length: 11\r\n\r\nhello world").await;
    assert!(head.starts_with("HTTP/1.1 200 "));
    assert_eq!(body, answer);
    assert_eq!(got.recv().await.unwrap().body, b"hello world");
    // And an answer the record cannot read (compressed another way) is passed all the same, and said to be unread.
    let (_, body) = ask(p.at, &request(QUESTION)).await;
    assert_eq!(body, b"\x0b\x02\x80{}\x03");
    let record = lines(p.record.path(), "alice");
    let Line::Request(r) = &record[0].line else { panic!() };
    assert_eq!((r.method.as_str(), r.path.as_str(), r.sent, r.received), ("POST", "/v1/files", 11, answer.len() as u64));
    assert!(r.new.is_empty() && r.answer.is_empty() && r.tokens.is_none() && r.model.is_none(), "that it happened and how much, not what: {r:?}");
    let Line::Request(r) = &record[1].line else { panic!() };
    assert!(r.unread[0].contains("(br)") && r.answer.is_empty() && r.new.len() == 1, "{r:?}");
    assert!(!everything(p.record.path()).contains("SECRET"));
}

/// A site that says one line and repeats what it is sent.
async fn site() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let _ = s.write_all(b"hello from the site\n").await;
                let mut piece = [0u8; 1024];
                while let Ok(n) = s.read(&mut piece).await {
                    if n == 0 || s.write_all(&piece[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

fn connects(record: &Path) -> Vec<(String, u16, bool, Why, Option<bool>)> {
    lines(record, "alice").into_iter().filter_map(|e| match e.line {
        Line::Connect(c) => {
            assert_eq!((c.agent.as_str(), c.person.as_str()), ("torq-agent-alice", "alice"));
            Some((c.host, c.port, c.allowed, c.why, c.reached))
        }
        _ => None,
    }).collect()
}

#[tokio::test]
async fn a_site_is_refused_or_carried_and_recorded_either_way() {
    let (model, _got) = model("model.pem", "model.key", vec![]).await;
    let site = site().await;
    let policy = Policy { hosts: vec!["listed.example".into(), "forbidden.example".into(), "down.example".into()], web: false, model: "model.test".into() };
    let p = proxy(model, true, policy, dial_to(Some(site))).await;
    let connect = |host: &str| format!("CONNECT {host} HTTP/1.1\r\nHost: {host}\r\nProxy-Authorization: Basic SECRET-PROXY\r\nConnection: close\r\n\r\n").into_bytes();

    // Not on the list: refused in words that name the setting.
    let (head, body) = ask(p.at, &connect("other.example:443")).await;
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("set torq agent network hosts other.example"), "{}", String::from_utf8_lossy(&body));
    // The model's own service, past the record: refused, and told where the record's way is.
    let (head, body) = ask(p.at, &connect("model.test:443")).await;
    assert!(head.starts_with("HTTP/1.1 403 ") && String::from_utf8_lossy(&body).contains(&format!("http://127.0.0.1:{}", p.at.port())), "{head}{}", String::from_utf8_lossy(&body));
    // Not HTTPS.
    let (head, _) = ask(p.at, &connect("listed.example:22")).await;
    assert!(head.starts_with("HTTP/1.1 403 "));
    // Listed, but its address is one the proxy never connects to.
    let (head, body) = ask(p.at, &connect("forbidden.example:443")).await;
    assert!(head.starts_with("HTTP/1.1 403 ") && String::from_utf8_lossy(&body).contains("this machine itself"), "{head}");
    // A plain HTTP address, as a proxy would be asked for one.
    let (head, _) = ask(p.at, b"GET http://listed.example/ HTTP/1.1\r\nHost: listed.example\r\nConnection: close\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");

    // Listed: carried both ways, unseen.
    let mut c = TcpStream::connect(p.at).await.unwrap();
    c.write_all(&connect("Listed.Example:443")).await.unwrap();
    let mut back = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&back).contains("hello from the site\n") {
            let mut piece = [0u8; 1024];
            let n = c.read(&mut piece).await.unwrap();
            assert!(n > 0);
            back.extend_from_slice(&piece[..n]);
        }
    }).await.expect("the site's first words through the proxy");
    assert!(back.starts_with(b"HTTP/1.1 200 "), "{}", String::from_utf8_lossy(&back));
    c.write_all(b"SECRET-INSIDE-THE-TUNNEL\n").await.unwrap();
    let mut echoed = [0u8; 25];
    tokio::time::timeout(Duration::from_secs(10), c.read_exact(&mut echoed)).await.unwrap().unwrap();
    assert_eq!(&echoed, b"SECRET-INSIDE-THE-TUNNEL\n");
    drop(c);

    assert_eq!(connects(p.record.path()), [
        ("other.example".to_string(), 443, false, Why::NotListed, None),
        ("model.test".to_string(), 443, false, Why::TheModel, None),
        ("listed.example".to_string(), 22, false, Why::NotHttps, None),
        ("forbidden.example".to_string(), 443, false, Why::Address, None),
        ("Listed.Example".to_string(), 443, true, Why::Listed, Some(true)),
    ]);
    assert!(!everything(p.record.path()).contains("SECRET"), "nothing of what passed inside, and no credential");

    // A listed site that does not answer: allowed, and recorded as not reached.
    let p = proxy(model, true, Policy { hosts: vec!["down.example".into()], web: false, model: "model.test".into() }, dial_to(None)).await;
    let (head, _) = ask(p.at, &connect("down.example:443")).await;
    assert!(head.starts_with("HTTP/1.1 502 "), "{head}");
    assert_eq!(connects(p.record.path()), [("down.example".to_string(), 443, true, Why::Listed, Some(false))]);
}

#[tokio::test]
async fn the_web_setting_opens_every_site_and_each_is_still_recorded() {
    let (model, _got) = model("model.pem", "model.key", vec![]).await;
    let site = site().await;
    let p = proxy(model, true, Policy { hosts: vec![], web: true, model: "model.test".into() }, dial_to(Some(site))).await;
    let mut c = TcpStream::connect(p.at).await.unwrap();
    c.write_all(b"CONNECT anywhere.example:443 HTTP/1.1\r\nHost: anywhere.example:443\r\n\r\n").await.unwrap();
    let mut back = [0u8; 12];
    tokio::time::timeout(Duration::from_secs(10), c.read_exact(&mut back)).await.unwrap().unwrap();
    assert_eq!(&back, b"HTTP/1.1 200");
    let (head, _) = ask(p.at, b"CONNECT model.test:443 HTTP/1.1\r\nHost: model.test:443\r\nConnection: close\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 403 "), "the model past the record stays refused: {head}");
    assert_eq!(connects(p.record.path()), [("anywhere.example".to_string(), 443, true, Why::Web, Some(true)), ("model.test".to_string(), 443, false, Why::TheModel, None)]);
}

#[tokio::test]
async fn nothing_is_passed_on_while_the_record_cannot_be_written() {
    let (model, mut got) = model("model.pem", "model.key", vec![Script::Whole("content-type: application/json\r\n", b"{}".to_vec())]).await;
    let site = site().await;
    let p = proxy(model, true, Policy { hosts: vec![], web: true, model: "model.test".into() }, dial_to(Some(site))).await;
    std::fs::remove_dir(p.record.path().join("alice")).unwrap();
    let (head, body) = ask(p.at, &request(QUESTION)).await;
    assert!(head.starts_with("HTTP/1.1 503 "), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("cannot be written"));
    let (head, _) = ask(p.at, b"CONNECT anywhere.example:443 HTTP/1.1\r\nHost: anywhere.example:443\r\nConnection: close\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 503 "), "nor a site: {head}");
    assert!(tokio::time::timeout(Duration::from_millis(300), got.recv()).await.is_err(), "the service was sent nothing");
    // Put right, it passes again.
    std::fs::create_dir(p.record.path().join("alice")).unwrap();
    let (head, _) = ask(p.at, &request(QUESTION)).await;
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    assert_eq!(lines(p.record.path(), "alice").len(), 1);
}

#[tokio::test]
async fn an_agent_has_only_so_much_under_way_at_once() {
    let (model, mut got) = model("model.pem", "model.key", vec![]).await;
    let p = proxy(model, true, closed(), dial_to(None)).await;
    p.proxy.busy.lock().unwrap().insert(me(), Arc::new(tokio::sync::Semaphore::new(0)));
    let (head, body) = ask(p.at, &request(QUESTION)).await;
    assert!(head.starts_with("HTTP/1.1 503 ") && String::from_utf8_lossy(&body).contains("under way"), "{head}");
    assert!(tokio::time::timeout(Duration::from_millis(300), got.recv()).await.is_err());
}

#[tokio::test]
async fn it_listens_on_this_machine_only() {
    let (model, _got) = model("model.pem", "model.key", vec![]).await;
    let p = proxy(model, true, closed(), dial_to(None)).await;
    let everywhere = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), p.proxy.clone().serve(everywhere)).await.expect("refused at once");
    assert!(refused.unwrap_err().to_string().contains("not this machine's own address"));
}

#[test]
fn the_agents_are_the_accounts_the_machine_says_and_nothing_else() {
    let passwd = "root:x:0:0::/root:/bin/sh\ntorq-agent-alice:x:994:991:alice's agent:/var/lib/torq-agent-alice:/run/current-system/sw/bin/nologin\nalice:x:1000:100::/home/alice:/bin/zsh\ntorq-agent-root:x:0:0::/:/bin/sh\ntorq-agent-carol:x:1001:100::/home/c:/bin/sh\n";
    let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(a, p)| (a.to_string(), p.to_string())).collect::<BTreeMap<_, _>>();
    let ok = agents_by_uid(passwd, &map(&[("torq-agent-alice", "alice")])).unwrap();
    assert_eq!(ok.get(&994), Some(&Agent { account: "torq-agent-alice".into(), person: "alice".into() }));
    assert_eq!(ok.len(), 1);
    for (account, person, why) in [
        ("torq-agent-bob", "bob", "not an account on this machine"),
        ("alice", "alice", "not the agent account"),
        ("torq-agent-alice", "bob", "not the agent account"),
        ("torq-agent-root", "root", "uid 0"),
        ("torq-agent-carol", "carol", "uid 1001"),
        ("torq-agent-../x", "../x", "not the agent account"),
    ] {
        let e = agents_by_uid(passwd, &map(&[(account, person)])).unwrap_err();
        assert!(e.contains(why), "{account}: {e}");
    }
}

#[test]
fn without_an_authority_there_is_no_tls() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("none.pem"), "").unwrap();
    assert!(tls(&d.path().join("none.pem")).is_err(), "an empty list of authorities trusts no one: refused, not taken as everyone");
    assert!(tls(&d.path().join("missing.pem")).is_err());
    assert!(tls(&fixture("ca.pem")).is_ok());
}

/// By hand, never in a build: the proxy as it runs on a machine (the real model's service, the machine's own
/// authorities, the real way out), but with whoever runs this as alice's agent, the record in the folder
/// `TORQ_BY_HAND` names, on the port `TORQ_BY_HAND_PORT` names, and the sites `TORQ_BY_HAND_HOSTS` lists
/// (commas). How the real Claude Code was put through it once (docs/HARNESS-AS-A-SYSTEM.md).
#[tokio::test]
#[ignore = "by hand: asks the real model's service, with the sign-in of whoever runs it"]
async fn by_hand() {
    let record = PathBuf::from(std::env::var("TORQ_BY_HAND").expect("TORQ_BY_HAND: a folder for the record"));
    std::fs::create_dir_all(record.join("alice")).unwrap();
    let port: u16 = std::env::var("TORQ_BY_HAND_PORT").expect("TORQ_BY_HAND_PORT").parse().unwrap();
    let hosts = std::env::var("TORQ_BY_HAND_HOSTS").unwrap_or_default().split(',').filter(|h| !h.is_empty()).map(String::from).collect();
    let proxy = Arc::new(Proxy {
        agents: [(me(), Agent { account: "torq-agent-alice".into(), person: "alice".into() })].into(),
        policy: Policy { hosts, web: false, model: torq_model_proxy::MODEL.into() },
        upstream: Upstream { name: torq_model_proxy::MODEL.into(), at: format!("{}:443", torq_model_proxy::MODEL), tls: tls(Path::new(torq_model_proxy::AUTHORITIES)).unwrap() },
        recorder: record::Recorder::new(record, record::Bounds::default()),
        dial: torq_model_proxy::outside(),
        busy: Mutex::default(),
    });
    proxy.serve(TcpListener::bind(("127.0.0.1", port)).await.unwrap()).await.unwrap();
}

#[test]
fn the_settings_are_read_exactly() {
    let s: torq_model_proxy::Settings = serde_json::from_str(r#"{"port":7801,"agents":{"torq-agent-alice":"alice"},"hosts":["github.com"],"web":false}"#).unwrap();
    assert_eq!((s.port, s.agents.len(), s.hosts.as_slice(), s.web), (7801, 1, ["github.com".to_string()].as_slice(), false));
    // A word the proxy does not know is a refusal, not something passed over: a setting it would not honour.
    assert!(serde_json::from_str::<torq_model_proxy::Settings>(r#"{"port":7801,"agents":{},"hosts":[],"web":false,"upstream":"https://elsewhere.example"}"#).is_err());
    // And nothing has a default: a file that does not say whether the web is open is refused.
    assert!(serde_json::from_str::<torq_model_proxy::Settings>(r#"{"port":7801,"agents":{},"hosts":[]}"#).is_err());
}

#[test]
fn it_listens_only_on_the_socket_systemd_hands_over() {
    use torq_model_proxy::inherited;
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert!(inherited(Ok(Some(l)), port).is_ok());
    let none = inherited(Ok(None), 7801).unwrap_err();
    assert!(none.contains("never listens by itself"), "{none}");
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let e = inherited(Ok(Some(other)), port).unwrap_err();
    assert!(e.contains("not 127.0.0.1:"), "a socket on another port is refused: {e}");
    assert!(inherited(Err(std::io::Error::other("a datagram socket")), 7801).is_err());
}
