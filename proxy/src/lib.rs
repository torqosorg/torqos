//! torq-model-proxy (ADR-057): the one way out of an agent's account.
//!
//! It listens on this machine only (127.0.0.1). Two things arrive there, both from a person's agent account
//! (`torq-agent-<person>`), which the kernel names (`who`), never the caller:
//!
//! - **A request to the model** (the engine's `ANTHROPIC_BASE_URL` is this proxy): passed to the model's
//!   service over TLS as it came, the answer passed back as it streams, and one line written to the person's
//!   record when it ends (`relay`, `read`, `record`). Nothing is changed on the way but what HTTP says belongs
//!   to one connection (`HOP`) and the name of the host asked. No credential is kept, logged or looked at.
//! - **A site to reach** (`CONNECT`, the engine's `HTTPS_PROXY` is this proxy too): allowed or refused by the
//!   owner's settings (`policy`), recorded either way, and where allowed, bytes carried both ways unseen.
//!
//! HTTP is hyper's and TLS is rustls's: nothing of either protocol is parsed here.

// No unsafe code and no dead code, ever (docs/DECISIONS.md ADR-020).
#![forbid(unsafe_code)]
#![forbid(dead_code)]

pub mod policy;
pub mod read;
pub mod record;
pub mod time;
pub mod who;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::{HeaderValue, CONTENT_ENCODING, CONTENT_TYPE, HOST};
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::rt::TokioIo;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use torq_wire::activity::{self, Line, Outcome, Why};

/// The model's service: the one place a request to the model is passed to. Never chosen by a caller, and not a
/// setting.
pub const MODEL: &str = "api.anthropic.com";
/// The machine's own list of the authorities whose certificates it accepts (NixOS's).
pub const AUTHORITIES: &str = "/etc/ssl/certs/ca-certificates.crt";
/// How many requests and sites one agent has under way at once. More are refused, and said so.
pub const MAX_AT_ONCE: usize = 64;
/// How long a connection out may take to be made.
pub const CONNECT_WAIT: Duration = Duration::from_secs(30);
/// The headers HTTP says belong to one connection, not to the request: never passed on, either way
/// (RFC 9110, 7.6.1). Everything else is passed as it came.
pub const HOP: [&str; 9] = ["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade"];
/// The headers the engine names its conversation and its helpers by: the only headers' values the record keeps.
const SESSION: &str = "x-claude-code-session-id";
const AGENT_ID: &str = "x-claude-code-agent-id";
const PARENT_AGENT_ID: &str = "x-claude-code-parent-agent-id";

/// An agent's account, and the person it serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub account: String,
    pub person: String,
}

/// What the proxy is given by the machine's configuration (`/etc/torq/model-proxy.json`, written by
/// `modules/model-proxy.nix`; no agent can propose it, and the configuration tree cannot set the file).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Where it listens, on 127.0.0.1.
    pub port: u16,
    /// Each agent account and its person.
    pub agents: BTreeMap<String, String>,
    /// The sites the owner lets the agents reach.
    pub hosts: Vec<String>,
    /// Whether the web is open to them.
    pub web: bool,
}

/// The agents by uid, as `/etc/passwd` (its text) has them. Each must be what an agent's account is: named for
/// its person, a system account, never root. Anything else and the proxy does not start.
pub fn agents_by_uid(passwd: &str, agents: &BTreeMap<String, String>) -> Result<HashMap<u32, Agent>, String> {
    let mut by_uid = HashMap::new();
    for (account, person) in agents {
        let plain = !person.is_empty() && person.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !plain || *account != format!("torq-agent-{person}") {
            return Err(format!("{account} is not the agent account of a person named {person:?}"));
        }
        let uid = passwd.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 2 && f[0] == account).then(|| f[2].parse::<u32>().ok()).flatten()
        });
        match uid {
            Some(uid) if uid != 0 && uid < 1000 => by_uid.insert(uid, Agent { account: account.clone(), person: person.clone() }),
            Some(uid) => return Err(format!("{account} has uid {uid}: not a system account, so not an agent's")),
            None => return Err(format!("{account} is not an account on this machine")),
        };
    }
    Ok(by_uid)
}

/// TLS as the proxy speaks it to the model's service: certificates checked against the authorities in
/// `authorities` (a file of them, PEM), HTTP/1.1.
pub fn tls(authorities: &Path) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(authorities).map_err(io::Error::other)?.filter_map(Result::ok).collect();
    roots.add_parsable_certificates(certs);
    if roots.is_empty() {
        return Err(io::Error::other(format!("no certificate authority in {}", authorities.display())));
    }
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(io::Error::other)?.with_root_certificates(roots).with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// The model's service as the proxy reaches it.
pub struct Upstream {
    /// Its name: what its certificate must be for, and the host it is asked as.
    pub name: String,
    /// Where it is connected to (`api.anthropic.com:443`).
    pub at: String,
    pub tls: Arc<ClientConfig>,
}

/// How a site is connected to for an agent: its name, the port, and whether the owner listed it.
pub type Dial = Arc<dyn Fn(String, u16, bool) -> Pin<Box<dyn Future<Output = io::Result<TcpStream>> + Send>> + Send + Sync>;

/// The real way out: the name looked up, each address judged (`policy::address_ok`), the first that answers
/// taken. An address that may not be reached is "permission denied", which the caller is told in words.
pub fn outside() -> Dial {
    Arc::new(|host, port, listed| {
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await?.collect();
            let allowed: Vec<SocketAddr> = found.iter().copied().filter(|a| policy::address_ok(a.ip(), listed)).collect();
            if allowed.is_empty() && !found.is_empty() {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let mut last = io::Error::from(io::ErrorKind::NotFound);
            for a in allowed {
                match tokio::time::timeout(CONNECT_WAIT, TcpStream::connect(a)).await {
                    Ok(Ok(s)) => return Ok(s),
                    Ok(Err(e)) => last = e,
                    Err(_) => last = io::ErrorKind::TimedOut.into(),
                }
            }
            Err(last)
        })
    })
}

/// What the proxy sends back: an answer's body as it streams, or words of its own.
type Passed = BoxBody<Bytes, hyper::Error>;

/// The proxy.
pub struct Proxy {
    pub agents: HashMap<u32, Agent>,
    pub policy: policy::Policy,
    pub upstream: Upstream,
    pub recorder: record::Recorder,
    pub dial: Dial,
    /// How much each agent has under way.
    pub busy: Mutex<HashMap<u32, Arc<Semaphore>>>,
}

/// Words of the proxy's own, as an answer.
fn words(status: StatusCode, text: String) -> Response<Passed> {
    let mut r = Response::new(Full::new(Bytes::from(text)).map_err(|never| match never {}).boxed());
    *r.status_mut() = status;
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

impl Proxy {
    /// Serves `listener`, for ever. Only on this machine's own address: anything else is refused before a
    /// connection is taken, since the kernel's table names a caller only there.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> io::Result<()> {
        let port = listener.local_addr()?;
        if !port.ip().is_loopback() {
            return Err(io::Error::other(format!("{port} is not this machine's own address (127.0.0.1): the model proxy listens nowhere else")));
        }
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("torq-model-proxy: a connection could not be taken: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let proxy = self.clone();
            tokio::spawn(async move {
                // Who is connected, read once, while the connection is held: every request on it is theirs.
                let who = stream.local_addr().ok().and_then(|local| who::caller(peer, local)).and_then(|uid| proxy.agents.get(&uid).map(|a| (uid, a.clone())));
                let service = hyper::service::service_fn(move |req| {
                    let (proxy, who) = (proxy.clone(), who.clone());
                    async move { Ok::<_, std::convert::Infallible>(proxy.answer(who, port.port(), req).await) }
                });
                // A connection that ends badly is the caller's to notice; nothing of it is logged.
                let _ = hyper::server::conn::http1::Builder::new().preserve_header_case(true).serve_connection(TokioIo::new(stream), service).with_upgrades().await;
            });
        }
    }

    /// One more thing under way for the agent with `uid`, or none: too many already.
    fn slot(&self, uid: u32) -> Option<OwnedSemaphorePermit> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        busy.entry(uid).or_insert_with(|| Arc::new(Semaphore::new(MAX_AT_ONCE))).clone().try_acquire_owned().ok()
    }

    async fn answer(self: Arc<Self>, who: Option<(u32, Agent)>, port: u16, req: Request<Incoming>) -> Response<Passed> {
        let Some((uid, agent)) = who else {
            return words(StatusCode::FORBIDDEN, "TorqOS: this is the way out for the agents' accounts (torq-agent-<person>), and you are not one. Your own programs reach the network directly.\n".to_string());
        };
        if !self.recorder.ready(&agent.person) {
            return words(StatusCode::SERVICE_UNAVAILABLE, format!("TorqOS: the record of what {}'s agent does cannot be written (is the disk full?), and nothing is passed on for it until it can be. An administrator can look: journalctl -u torq-model-proxy.\n", agent.person));
        }
        let Some(permit) = self.slot(uid) else {
            return words(StatusCode::SERVICE_UNAVAILABLE, format!("TorqOS: your agent has {MAX_AT_ONCE} requests under way through the model proxy already. Try again when some have ended.\n"));
        };
        if req.method() == Method::CONNECT {
            self.connect(agent, port, permit, req).await
        } else if req.uri().scheme().is_some() {
            words(StatusCode::FORBIDDEN, "TorqOS: your agent reaches sites as HTTPS only. A plain HTTP address is refused.\n".to_string())
        } else {
            self.relay(agent, permit, req).await
        }
    }

    /// A site asked for (`CONNECT host:port`): judged, recorded, and where allowed, carried.
    async fn connect(self: Arc<Self>, agent: Agent, proxy_port: u16, permit: OwnedSemaphorePermit, req: Request<Incoming>) -> Response<Passed> {
        let (host, port) = match req.uri().authority() {
            Some(a) => (a.host().to_string(), a.port_u16().unwrap_or(0)),
            None => (String::new(), 0),
        };
        let (allowed, why) = self.policy.decide(&host, port);
        let shown: String = host.chars().filter(|c| c.is_ascii_graphic()).take(activity::MAX_NAME).collect();
        let mut line = activity::Connect { time: time::rfc3339(time::now()), agent: agent.account.clone(), person: agent.person.clone(), host: shown.clone(), port, allowed, why, reached: None };
        if !allowed {
            self.recorder.write(&agent.person, Line::Connect(line));
            return words(StatusCode::FORBIDDEN, policy::refusal(why, &shown, port, proxy_port));
        }
        let site = policy::site(&host).unwrap_or_default();
        match (self.dial)(site, port, why == Why::Listed).await {
            Ok(mut out) => {
                line.reached = Some(true);
                self.recorder.write(&agent.person, Line::Connect(line));
                tokio::spawn(async move {
                    // What passes from here is between the agent and the site, encrypted: carried, not seen.
                    if let Ok(upgraded) = hyper::upgrade::on(req).await {
                        let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(upgraded), &mut out).await;
                    }
                    drop(permit);
                });
                Response::new(Empty::new().map_err(|never| match never {}).boxed())
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                line.allowed = false;
                line.why = Why::Address;
                self.recorder.write(&agent.person, Line::Connect(line));
                words(StatusCode::FORBIDDEN, policy::refusal(Why::Address, &shown, port, proxy_port))
            }
            Err(e) => {
                line.reached = Some(false);
                self.recorder.write(&agent.person, Line::Connect(line));
                words(StatusCode::BAD_GATEWAY, format!("TorqOS: {shown} was not reached ({e}).\n"))
            }
        }
    }

    /// The request sent to the model's service, and its answer's beginning.
    async fn send(&self, req: Request<Asking>) -> Result<Response<Incoming>, String> {
        let tcp = tokio::time::timeout(CONNECT_WAIT, TcpStream::connect(self.upstream.at.as_str())).await.map_err(|_| "it did not answer in time".to_string())?.map_err(|e| e.to_string())?;
        let name = ServerName::try_from(self.upstream.name.clone()).map_err(|e| e.to_string())?;
        let tls = TlsConnector::from(self.upstream.tls.clone()).connect(name, tcp).await.map_err(|e| format!("its certificate or its encryption was not accepted: {e}"))?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new().preserve_header_case(true).handshake(TokioIo::new(tls)).await.map_err(|e| e.to_string())?;
        // The connection is driven until the answer has passed, then ends: one request, one connection.
        tokio::spawn(connection);
        sender.send_request(req).await.map_err(|e| e.to_string())
    }

    /// A request to the model: passed on as it came, its answer passed back as it streams, one line recorded
    /// when it has ended, however it ends.
    async fn relay(self: Arc<Self>, agent: Agent, permit: OwnedSemaphorePermit, req: Request<Incoming>) -> Response<Passed> {
        let (mut parts, body) = req.into_parts();
        let path = parts.uri.path().to_string();
        // What is read for the record: a request for the model's messages. Anything else asked of the service is
        // passed on and recorded as a line (what, when, how much), not read.
        let messages = parts.method == Method::POST && (path == "/v1/messages" || path.starts_with("/v1/messages/"));
        let packed = parts.headers.contains_key(CONTENT_ENCODING);
        let named = |h: &str| parts.headers.get(h).and_then(|v| v.to_str().ok()).and_then(activity::name);
        let mut line = activity::Request {
            time: time::rfc3339(time::now()),
            agent: agent.account,
            person: agent.person,
            session: named(SESSION),
            agent_id: named(AGENT_ID),
            parent_agent_id: named(PARENT_AGENT_ID),
            method: parts.method.as_str().chars().take(16).collect(),
            path: path.chars().filter(|c| c.is_ascii_graphic()).take(activity::MAX_NAME).collect(),
            ..Default::default()
        };
        if messages && packed {
            line.unread.push("the request was not read: its body was compressed".to_string());
        }
        let asked = Arc::new(Mutex::new(read::Asked::new(messages && !packed)));
        for h in HOP {
            parts.headers.remove(h);
        }
        // The one header rewritten: the service is asked by its own name, not by this proxy's address.
        if let Ok(host) = HeaderValue::from_str(&self.upstream.name) {
            parts.headers.insert(HOST, host);
        }
        parts.version = Version::HTTP_11;
        let mut job = Job { proxy: self.clone(), line, asked: asked.clone(), started: Instant::now(), _permit: permit };
        match self.send(Request::from_parts(parts, Asking { inner: body, asked })).await {
            Ok(answer) => {
                let (mut parts, body) = answer.into_parts();
                job.line.status = Some(parts.status.as_u16());
                let said = |h| parts.headers.get(h).and_then(|v: &HeaderValue| v.to_str().ok());
                let reading = read::Answer::new(messages, said(CONTENT_TYPE), said(CONTENT_ENCODING));
                for h in HOP {
                    parts.headers.remove(h);
                }
                Response::from_parts(parts, Answering { inner: body, reading: Some((job, reading)) }.boxed())
            }
            Err(e) => {
                let name = self.upstream.name.clone();
                job.finish(Outcome::NotReached, None);
                words(StatusCode::BAD_GATEWAY, format!("TorqOS: the model's service ({name}) was not reached through the model proxy: {e}\n"))
            }
        }
    }
}

/// One request under way: what its line will say, and what is needed to write it.
struct Job {
    proxy: Arc<Proxy>,
    line: activity::Request,
    asked: Arc<Mutex<read::Asked>>,
    started: Instant,
    _permit: OwnedSemaphorePermit,
}

impl Job {
    /// The line written: what was asked, what was answered as far as it was read, and how it ended.
    fn finish(mut self, outcome: Outcome, answer: Option<read::Answer>) {
        self.line.outcome = outcome;
        self.line.ms = self.started.elapsed().as_millis() as u64;
        {
            let asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
            self.line.sent = asked.sent;
            asked.read_into(&mut self.line);
        }
        if let Some(a) = answer {
            a.read_into(&mut self.line);
        }
        let person = self.line.person.clone();
        self.proxy.recorder.write(&person, Line::Request(self.line));
    }
}

/// A request's body on its way to the service: each piece passed as it came, and a copy read for the record.
struct Asking {
    inner: Incoming,
    asked: Arc<Mutex<read::Asked>>,
}

impl Body for Asking {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let next = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &next {
            if let Some(data) = frame.data_ref() {
                self.asked.lock().unwrap_or_else(|e| e.into_inner()).feed(data);
            }
        }
        next
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// An answer's body on its way back: each piece passed the moment it comes, a copy read for the record, and
/// the line written when it ends, or when the engine goes away before the end.
struct Answering {
    inner: Incoming,
    reading: Option<(Job, read::Answer)>,
}

impl Answering {
    fn end(&mut self, outcome: Outcome) {
        if let Some((job, answer)) = self.reading.take() {
            job.finish(outcome, Some(answer));
        }
    }
}

impl Body for Answering {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let next = Pin::new(&mut self.inner).poll_frame(cx);
        match &next {
            Poll::Ready(Some(Ok(frame))) => {
                if let (Some(data), Some((_, answer))) = (frame.data_ref(), self.reading.as_mut()) {
                    answer.feed(data);
                }
            }
            Poll::Ready(Some(Err(_))) => self.end(Outcome::Interrupted),
            Poll::Ready(None) => self.end(Outcome::Complete),
            Poll::Pending => {}
        }
        next
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.end(Outcome::Interrupted);
    }
}

/// The program: `torq-model-proxy serve <settings.json>`.
pub fn run() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [verb, settings] = args.as_slice() else {
        eprintln!("torq-model-proxy serve <settings.json>: TorqOS's model proxy, started by its service (torq-model-proxy.service).");
        return 2.into();
    };
    if verb != "serve" {
        eprintln!("torq-model-proxy: {verb} is not something it does. It is started by its service: torq-model-proxy serve <settings.json>.");
        return 2.into();
    }
    match start(Path::new(settings)) {
        Ok(()) => 0.into(),
        Err(e) => {
            eprintln!("torq-model-proxy: {e}");
            1.into()
        }
    }
}

/// The listening socket systemd made and handed over (torq-model-proxy.socket), checked to be 127.0.0.1:`port`.
/// The proxy never binds the port itself: an unprivileged port is anyone's to take while no one holds it, and
/// whoever held it would be sent the agents' requests, sign-in and all (ADR-057 4). systemd holds it from early
/// boot, across the proxy's restarts, so no account but root ever can. Without it the proxy does not start.
pub fn inherited(given: std::io::Result<Option<std::net::TcpListener>>, port: u16) -> Result<std::net::TcpListener, String> {
    let listener = given
        .map_err(|e| format!("the socket systemd handed over is not a listening TCP socket: {e}"))?
        .ok_or_else(|| "no listening socket from systemd: the proxy is started by its socket (torq-model-proxy.socket), and never listens by itself".to_string())?;
    let at = listener.local_addr().map_err(|e| e.to_string())?;
    if at != SocketAddr::from(([127, 0, 0, 1], port)) {
        return Err(format!("the socket systemd handed over listens on {at}, not 127.0.0.1:{port}"));
    }
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(listener)
}

fn start(settings: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(settings).map_err(|e| format!("reading {}: {e}", settings.display()))?;
    let settings: Settings = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", settings.display()))?;
    let port = settings.port;
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| format!("reading /etc/passwd: {e}"))?;
    let proxy = Arc::new(Proxy {
        agents: agents_by_uid(&passwd, &settings.agents)?,
        policy: policy::Policy { hosts: settings.hosts, web: settings.web, model: MODEL.to_string() },
        upstream: Upstream { name: MODEL.to_string(), at: format!("{MODEL}:{}", policy::HTTPS), tls: tls(Path::new(AUTHORITIES)).map_err(|e| e.to_string())? },
        recorder: record::Recorder::new(activity::DIR.into(), record::Bounds::default()),
        dial: outside(),
        busy: Mutex::default(),
    });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let listener = inherited(listenfd::ListenFd::from_env().take_tcp_listener(0), port)?;
    runtime.block_on(async {
        let listener = TcpListener::from_std(listener).map_err(|e| format!("the listening socket from systemd: {e}"))?;
        eprintln!("torq-model-proxy: listening on 127.0.0.1:{port} for {} agent account(s); the web is {} to them, {} site(s) listed", proxy.agents.len(), if proxy.policy.web { "open" } else { "closed" }, proxy.policy.hosts.len());
        proxy.serve(listener).await.map_err(|e| e.to_string())
    })
}
