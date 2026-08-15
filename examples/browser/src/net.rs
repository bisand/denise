//! The wire, on its own thread.
//!
//! Neither Denise backend can be woken by another thread — the winit loop
//! has no proxy and the kiosk loop polls file descriptors it was given — so
//! the fetcher does not try. Requests go down an mpsc channel, responses
//! come back up one, and the app drains the receiver every frame *while a
//! request is in flight*, at a bounded cadence the backends arrange. Idle,
//! nothing polls and nothing wakes; the toolkit's idle-costs-nothing rule
//! survives having a network.
//!
//! One [`ureq::Agent`] lives as long as the thread: its cookie jar is what
//! makes a login form or a redirect-after-POST behave, and rustls underneath
//! is what keeps the kiosk build one static musl binary.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use url::Url;

/// What a response body cap is for: a page is read to be laid out, an image
/// to be decoded, and neither deserves an unbounded allocation.
const PAGE_CAP: u64 = 8 * 1024 * 1024;
const IMAGE_CAP: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FetchKind {
    Page,
    /// The DOM node whose `<img>` this fills.
    Image {
        dom: usize,
    },
    /// A linked stylesheet; text, like a page.
    Style,
}

pub struct FetchReq {
    pub id: u64,
    pub kind: FetchKind,
    pub url: Url,
    /// A urlencoded body makes the request a POST.
    pub post: Option<String>,
}

pub enum Fetched {
    /// A page, already transcoded to UTF-8 from whatever charset the server
    /// declared.
    Text(String),
    /// An image, as delivered. Decoded once images land.
    Bytes(#[allow(dead_code)] Vec<u8>),
}

pub struct FetchDone {
    pub id: u64,
    pub kind: FetchKind,
    /// Where the redirects ended up; relative URLs resolve against this.
    pub final_url: Url,
    pub result: Result<Fetched, String>,
}

pub struct Net {
    tx: Sender<FetchReq>,
    rx: Receiver<FetchDone>,
    next_id: u64,
}

impl Net {
    pub fn start() -> Self {
        let (tx, worker_rx) = channel::<FetchReq>();
        let (worker_tx, rx) = channel::<FetchDone>();
        std::thread::spawn(move || worker(worker_rx, worker_tx));
        Self { tx, rx, next_id: 1 }
    }

    /// Queues a fetch and returns its id, the number that tells a late
    /// response from a stale one.
    pub fn fetch(&mut self, kind: FetchKind, url: Url, post: Option<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.tx.send(FetchReq {
            id,
            kind,
            url,
            post,
        });
        id
    }

    /// Everything that has arrived since last asked. Never blocks.
    pub fn done(&mut self) -> Vec<FetchDone> {
        self.rx.try_iter().collect()
    }
}

fn worker(rx: Receiver<FetchReq>, tx: Sender<FetchDone>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(10)
        .user_agent(concat!("denise-browser/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    for req in rx {
        let done = handle(&agent, &req);
        if tx.send(done).is_err() {
            // The app is gone; so is the reason to fetch.
            return;
        }
    }
}

fn handle(agent: &ureq::Agent, req: &FetchReq) -> FetchDone {
    let result = match req.url.scheme() {
        "http" | "https" => http(agent, req),
        // Local files make the browser testable on a desk with no network
        // and a Pi with no route: fixtures are pages too.
        "file" => file(req),
        other => Err(format!("scheme {other}: not going there")),
    };
    let (final_url, result) = match result {
        Ok((url, fetched)) => (url, Ok(fetched)),
        Err(e) => (req.url.clone(), Err(e)),
    };
    FetchDone {
        id: req.id,
        kind: req.kind,
        final_url,
        result,
    }
}

fn http(agent: &ureq::Agent, req: &FetchReq) -> Result<(Url, Fetched), String> {
    use ureq::ResponseExt as _;

    let response = match &req.post {
        None => agent.get(req.url.as_str()).call(),
        Some(body) => agent
            .post(req.url.as_str())
            .header("content-type", "application/x-www-form-urlencoded")
            .send(body.as_str()),
    };
    let mut response = response.map_err(|e| e.to_string())?;

    let final_url = response
        .get_uri()
        .to_string()
        .parse::<Url>()
        .unwrap_or_else(|_| req.url.clone());

    let fetched = match req.kind {
        FetchKind::Page | FetchKind::Style => {
            // `read_to_string` transcodes from the declared charset; the
            // rest of the pipeline only ever sees UTF-8.
            let text = response
                .body_mut()
                .with_config()
                .limit(PAGE_CAP)
                .read_to_string()
                .map_err(|e| e.to_string())?;
            Fetched::Text(text)
        }
        FetchKind::Image { .. } => {
            let bytes = response
                .body_mut()
                .with_config()
                .limit(IMAGE_CAP)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            Fetched::Bytes(bytes)
        }
    };
    Ok((final_url, fetched))
}

fn file(req: &FetchReq) -> Result<(Url, Fetched), String> {
    let path = req
        .url
        .to_file_path()
        .map_err(|()| "not a usable file path".to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let fetched = match req.kind {
        FetchKind::Page | FetchKind::Style => {
            Fetched::Text(String::from_utf8_lossy(&bytes).into_owned())
        }
        FetchKind::Image { .. } => Fetched::Bytes(bytes),
    };
    Ok((req.url.clone(), fetched))
}
