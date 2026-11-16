//! Reading what passes, for the record (ADR-057). Nothing here touches what is passed on: these read copies.
//!
//! A request to the model carries the whole conversation each time. What is new in it is what comes after
//! the model's last message: the person's question, or what each tool returned. The answer is read as it streams: its text, each tool
//! the model calls and with what, the tokens the service counted. Every bound is `torq_wire::activity`'s, and
//! where one cuts, the line says so.

use flate2::write::GzDecoder;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use torq_wire::activity::{cut, name, Item, Request, Tokens, MAX_ITEMS, MAX_READ_ANSWER, MAX_READ_REQUEST, MAX_TEXT};

/// A copy of a request's body as it passes, up to `MAX_READ_REQUEST`; past that, only its length.
#[derive(Debug, Default)]
pub struct Asked {
    /// Whether the body is kept at all (a request to the model's messages; nothing else is read).
    keep: bool,
    body: Vec<u8>,
    /// Every byte that passed, kept or not.
    pub sent: u64,
}

impl Asked {
    pub fn new(keep: bool) -> Self {
        Asked { keep, ..Default::default() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.sent += bytes.len() as u64;
        if self.keep && self.body.len() < MAX_READ_REQUEST {
            let room = MAX_READ_REQUEST - self.body.len();
            self.body.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
    }

    /// What was new in the request, written into `line`: the model named, and the parts of each message after
    /// the model's last one.
    pub fn read_into(&self, line: &mut Request) {
        if !self.keep {
            return;
        }
        if self.sent > MAX_READ_REQUEST as u64 {
            line.unread.push(format!("the request was {} bytes, more than the {} read for the record", self.sent, MAX_READ_REQUEST));
            return;
        }
        // Only the model's name and the messages are looked at; the messages are kept as text until the last
        // one or two are needed, so a long conversation is not built up in memory to be thrown away.
        #[derive(Deserialize)]
        struct Body<'a> {
            model: Option<String>,
            #[serde(borrow, default)]
            messages: Vec<&'a RawValue>,
        }
        let Ok(body) = serde_json::from_slice::<Body>(&self.body) else {
            line.unread.push("the request was not read: it is not the JSON a request to the model is".to_string());
            return;
        };
        line.model = body.model.as_deref().and_then(name);
        // What is new: every message after the model's last one. (Claude Code 2.1.316 ends a request with a
        // message of its own after the person's, role `system`: the last message alone would miss the question.)
        let mut fresh: Vec<Value> = Vec::new();
        let mut before: Option<Value> = None;
        for m in body.messages.iter().rev().take(MAX_ITEMS) {
            let Ok(v) = serde_json::from_str::<Value>(m.get()) else { break };
            if v["role"] == "assistant" {
                before = Some(v);
                break;
            }
            fresh.push(v);
        }
        // The tools the model called in its last message, by the id each result answers.
        let mut calls: HashMap<String, String> = HashMap::new();
        for block in before.iter().flat_map(|b| b["content"].as_array()).flatten() {
            if let (Some("tool_use"), Some(id), Some(tool)) = (block["type"].as_str(), block["id"].as_str(), block["name"].as_str()) {
                calls.insert(id.to_string(), tool.to_string());
            }
        }
        let mut items = Kept::default();
        for message in fresh.iter().rev() {
            // A message that is not the person's side of the conversation (the engine's own instructions): that
            // it was there, not its words.
            if message["role"] != "user" {
                items.push(Item::Other { what: format!("{}-message", message["role"].as_str().and_then(name).unwrap_or_else(|| "unknown".to_string())) });
                continue;
            }
            match &message["content"] {
                Value::String(s) => items.push(text(s)),
                Value::Array(blocks) => {
                    for b in blocks {
                        items.push(match b["type"].as_str() {
                            Some("text") => text(b["text"].as_str().unwrap_or_default()),
                            Some("tool_result") => {
                                let (text, cut_from) = cut(&said(&b["content"]));
                                let tool = b["tool_use_id"].as_str().and_then(|id| calls.get(id)).and_then(|t| name(t));
                                Item::ToolResult { tool, text, error: b["is_error"] == true, cut_from }
                            }
                            other => Item::Other { what: other.and_then(name).unwrap_or_else(|| "unknown".to_string()) },
                        });
                    }
                }
                _ => {}
            }
        }
        line.new = items.items;
        line.left_out.new = items.over;
    }
}

fn text(s: &str) -> Item {
    let (text, cut_from) = cut(s);
    Item::Text { text, cut_from }
}

/// What a tool returned, as words: its text, or the texts of its parts, a part that is not text by its kind.
fn said(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(|p| match p["type"].as_str() {
            Some("text") => p["text"].as_str().unwrap_or_default().to_string(),
            other => format!("[{}]", other.and_then(name).unwrap_or_else(|| "unknown".to_string())),
        }).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Up to `MAX_ITEMS` things, and how many more there were.
#[derive(Debug, Default)]
struct Kept {
    items: Vec<Item>,
    over: u64,
}

impl Kept {
    fn push(&mut self, item: Item) {
        if self.items.len() < MAX_ITEMS {
            self.items.push(item);
        } else {
            self.over += 1;
        }
    }
}

/// A text gathered piece by piece, kept up to a little past `MAX_TEXT` (it is cut to it at the end), its full
/// length counted.
#[derive(Debug, Default)]
struct Growing {
    text: String,
    full: u64,
}

impl Growing {
    fn push(&mut self, s: &str) {
        self.full += s.len() as u64;
        if self.text.len() <= MAX_TEXT {
            self.text.push_str(s);
        }
    }

    fn done(self) -> (String, Option<u64>) {
        let (text, was_cut) = cut(&self.text);
        (text, (was_cut.is_some() || self.full > self.text.len() as u64).then_some(self.full))
    }
}

/// One part of an answer while it streams.
#[derive(Debug)]
enum Block {
    Text(Growing),
    Call { tool: String, input: Growing },
    Other(String),
}

/// How the answer's body is written.
enum Form {
    /// A stream of events (`text/event-stream`).
    Events,
    /// One JSON value.
    Json,
    /// Neither: only counted.
    Unread,
}

/// How the body is compressed, as its `content-encoding` says.
enum Packed {
    No,
    Gzip(Box<GzDecoder<Vec<u8>>>),
}

/// An answer read as it passes.
pub struct Answer {
    form: Form,
    packed: Packed,
    /// What is not yet a whole event (a stream), or the body so far (JSON).
    pending: Vec<u8>,
    /// The uncompressed bytes read so far, against `MAX_READ_ANSWER`.
    read: usize,
    /// Every byte that passed, read or not.
    pub received: u64,
    blocks: Vec<(u64, Block)>,
    over: u64,
    tokens: Option<Tokens>,
    stop: Option<String>,
    error: Option<String>,
    unread: Vec<String>,
}

impl Answer {
    /// An answer whose headers say `content_type` and `content_encoding`. `wanted`: whether it is read at all
    /// (an answer to a request for the model's messages).
    pub fn new(wanted: bool, content_type: Option<&str>, content_encoding: Option<&str>) -> Self {
        let kind = content_type.unwrap_or_default().split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        let mut unread = Vec::new();
        let mut form = match kind.as_str() {
            _ if !wanted => Form::Unread,
            "text/event-stream" => Form::Events,
            "application/json" => Form::Json,
            _ => Form::Unread,
        };
        let packed = match content_encoding.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("identity") => Packed::No,
            Some("gzip") => Packed::Gzip(Box::new(GzDecoder::new(Vec::new()))),
            Some(other) => {
                if !matches!(form, Form::Unread) {
                    unread.push(format!("the answer was not read: it was compressed in a way the record does not read ({})", name(other).unwrap_or_else(|| "unnamed".to_string())));
                }
                form = Form::Unread;
                Packed::No
            }
        };
        Answer { form, packed, pending: Vec::new(), read: 0, received: 0, blocks: Vec::new(), over: 0, tokens: None, stop: None, error: None, unread }
    }

    /// The next bytes of the body, as they passed.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.received += bytes.len() as u64;
        if matches!(self.form, Form::Unread) {
            return;
        }
        let plain = match &mut self.packed {
            Packed::No => bytes.to_vec(),
            Packed::Gzip(z) => {
                if z.write_all(bytes).and_then(|()| z.flush()).is_err() {
                    self.stop_reading("the answer was not read to its end: its compression could not be undone");
                    return;
                }
                std::mem::take(z.get_mut())
            }
        };
        if self.read + plain.len() > MAX_READ_ANSWER {
            self.stop_reading(&format!("the answer was longer than the {MAX_READ_ANSWER} bytes read for the record: the rest was passed on unread"));
            return;
        }
        self.read += plain.len();
        self.pending.extend_from_slice(&plain);
        if matches!(self.form, Form::Events) {
            self.events();
        }
    }

    fn stop_reading(&mut self, why: &str) {
        self.unread.push(why.to_string());
        self.form = Form::Unread;
        self.pending = Vec::new();
    }

    /// Each whole event in `pending` taken (events end at an empty line).
    fn events(&mut self) {
        loop {
            let end = self.pending.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2))
                .into_iter().chain(self.pending.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4))).min();
            let Some((at, gap)) = end else { return };
            let event: Vec<u8> = self.pending.drain(..at + gap).collect();
            let data: String = String::from_utf8_lossy(&event[..at]).lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect::<Vec<_>>().join("\n");
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                self.event(&v);
            }
        }
    }

    fn usage(&mut self, u: &Value) {
        if !u.is_object() {
            return;
        }
        let t = self.tokens.get_or_insert_with(Tokens::default);
        // A later count replaces an earlier one where it is given (the output's grows as the answer does).
        for (field, slot) in [("input_tokens", &mut t.input), ("cache_read_input_tokens", &mut t.cache_read), ("cache_creation_input_tokens", &mut t.cache_write), ("output_tokens", &mut t.output)] {
            if let Some(n) = u[field].as_u64() {
                *slot = n;
            }
        }
    }

    fn failed(&mut self, e: &Value) {
        let kind = e["type"].as_str().unwrap_or("error");
        let (words, _) = cut(&format!("{kind}: {}", e["message"].as_str().unwrap_or_default()));
        self.error = Some(words.chars().filter(|c| !c.is_control()).take(500).collect());
    }

    fn begin(&mut self, index: u64, block: &Value) {
        if self.blocks.len() >= MAX_ITEMS {
            self.over += 1;
            return;
        }
        let b = match block["type"].as_str() {
            Some("text") => {
                let mut g = Growing::default();
                g.push(block["text"].as_str().unwrap_or_default());
                Block::Text(g)
            }
            Some("tool_use") => {
                let mut input = Growing::default();
                // In a stream the input comes after, piece by piece; in a whole answer it is here.
                if block["input"].as_object().is_some_and(|o| !o.is_empty()) {
                    input.push(&block["input"].to_string());
                }
                Block::Call { tool: block["name"].as_str().and_then(name).unwrap_or_else(|| "unnamed".to_string()), input }
            }
            other => Block::Other(other.and_then(name).unwrap_or_else(|| "unknown".to_string())),
        };
        self.blocks.push((index, b));
    }

    /// One event of a stream.
    fn event(&mut self, v: &Value) {
        match v["type"].as_str() {
            Some("message_start") => self.usage(&v["message"]["usage"]),
            Some("content_block_start") => self.begin(v["index"].as_u64().unwrap_or(0), &v["content_block"]),
            Some("content_block_delta") => {
                let index = v["index"].as_u64().unwrap_or(0);
                if let Some((_, block)) = self.blocks.iter_mut().rev().find(|(i, _)| *i == index) {
                    match (block, v["delta"]["type"].as_str()) {
                        (Block::Text(g), Some("text_delta")) => g.push(v["delta"]["text"].as_str().unwrap_or_default()),
                        (Block::Call { input, .. }, Some("input_json_delta")) => input.push(v["delta"]["partial_json"].as_str().unwrap_or_default()),
                        _ => {}
                    }
                }
            }
            Some("message_delta") => {
                self.usage(&v["usage"]);
                if let Some(s) = v["delta"]["stop_reason"].as_str() {
                    self.stop = name(s);
                }
            }
            Some("error") => self.failed(&v["error"]),
            _ => {}
        }
    }

    /// The answer as far as it was read, written into `line`.
    pub fn read_into(mut self, line: &mut Request) {
        if matches!(self.form, Form::Json) {
            match serde_json::from_slice::<Value>(&std::mem::take(&mut self.pending)) {
                Ok(v) => {
                    self.usage(&v["usage"]);
                    self.stop = v["stop_reason"].as_str().and_then(name);
                    if v["type"] == "error" {
                        self.failed(&v["error"]);
                    }
                    for (i, block) in v["content"].as_array().into_iter().flatten().enumerate() {
                        self.begin(i as u64, block);
                    }
                }
                Err(_) if self.read > 0 => self.unread.push("the answer was not read: it is not whole JSON".to_string()),
                Err(_) => {}
            }
        }
        line.received = self.received;
        line.tokens = self.tokens;
        line.stop = self.stop;
        line.error = self.error;
        line.left_out.answer = self.over;
        line.unread.extend(self.unread);
        line.answer = self.blocks.into_iter().map(|(_, b)| match b {
            Block::Text(g) => {
                let (text, cut_from) = g.done();
                Item::Text { text, cut_from }
            }
            Block::Call { tool, input } => {
                let (input, cut_from) = input.done();
                Item::ToolCall { tool, input, cut_from }
            }
            Block::Other(what) => Item::Other { what },
        }).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use serde_json::json;

    fn asked(body: &Value) -> Request {
        let mut a = Asked::new(true);
        // In pieces, as it arrives.
        for piece in body.to_string().as_bytes().chunks(7) {
            a.feed(piece);
        }
        let mut line = Request::default();
        a.read_into(&mut line);
        line
    }

    #[test]
    fn what_is_new_in_a_request_is_its_last_message() {
        let line = asked(&json!({
            "model": "claude-test-1",
            "system": "a very long system prompt",
            "tools": [{"name": "Bash"}],
            "messages": [
                {"role": "user", "content": "an old question"},
                {"role": "assistant", "content": [{"type": "text", "text": "an old answer"}]},
                {"role": "user", "content": [{"type": "text", "text": "what is in /etc/hostname?"}]},
            ]
        }));
        assert_eq!(line.model.as_deref(), Some("claude-test-1"));
        assert_eq!(line.new, [Item::Text { text: "what is in /etc/hostname?".into(), cut_from: None }], "the question, not the conversation before it");
        assert!(line.unread.is_empty());
        assert_eq!(asked(&json!({"model": "m", "messages": [{"role": "user", "content": "plain words"}]})).new, [Item::Text { text: "plain words".into(), cut_from: None }]);
    }

    /// The shape Claude Code 2.1.316 sends (seen 2026-11-16): the person's message, its question last among
    /// the engine's own notes, and after it a message of the engine's with role `system`.
    #[test]
    fn the_question_is_found_though_the_engine_ends_the_request_with_a_message_of_its_own() {
        let line = asked(&json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "earlier"},
                {"role": "assistant", "content": [{"type": "text", "text": "earlier answer"}]},
                {"role": "user", "content": [{"type": "text", "text": "<system-reminder>a note</system-reminder>"}, {"type": "text", "text": "Reply with only the word OK."}]},
                {"role": "system", "content": [{"type": "text", "text": "# Environment, thirty thousand bytes of it"}]},
            ]
        }));
        assert_eq!(line.new, [
            Item::Text { text: "<system-reminder>a note</system-reminder>".into(), cut_from: None },
            Item::Text { text: "Reply with only the word OK.".into(), cut_from: None },
            Item::Other { what: "system-message".into() },
        ], "everything after the model's last message, in order; the engine's own message by its kind only");
        // A conversation's first request: nothing of the model's yet, so all of it is new.
        let first = asked(&json!({"model": "m", "messages": [{"role": "user", "content": "one"}, {"role": "system", "content": "x"}]}));
        assert_eq!(first.new, [Item::Text { text: "one".into(), cut_from: None }, Item::Other { what: "system-message".into() }]);
    }

    #[test]
    fn a_tools_result_is_kept_with_the_tools_name() {
        let line = asked(&json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cat /etc/hostname"}},
                    {"type": "tool_use", "id": "t2", "name": "Read", "input": {"file_path": "/x"}}]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "torq5\n"},
                    {"type": "tool_result", "tool_use_id": "t2", "is_error": true, "content": [{"type": "text", "text": "no such file"}, {"type": "image", "source": {"data": "AAAA"}}]},
                    {"type": "tool_result", "tool_use_id": "gone", "content": "x"},
                    {"type": "image", "source": {"data": "AAAA"}}]},
            ]
        }));
        assert_eq!(line.new, [
            Item::ToolResult { tool: Some("Bash".into()), text: "torq5\n".into(), error: false, cut_from: None },
            Item::ToolResult { tool: Some("Read".into()), text: "no such file\n[image]".into(), error: true, cut_from: None },
            Item::ToolResult { tool: None, text: "x".into(), error: false, cut_from: None },
            Item::Other { what: "image".into() },
        ]);
    }

    #[test]
    fn a_long_text_is_cut_and_many_things_are_counted_and_the_record_says_so() {
        let long = "x".repeat(MAX_TEXT + 100);
        let mut content: Vec<Value> = (0..MAX_ITEMS + 5).map(|i| json!({"type": "text", "text": format!("{i}")})).collect();
        content[0] = json!({"type": "text", "text": long});
        let line = asked(&json!({"model": "m", "messages": [{"role": "user", "content": content}]}));
        assert_eq!(line.new.len(), MAX_ITEMS);
        assert_eq!(line.left_out.new, 5);
        assert!(matches!(&line.new[0], Item::Text { text, cut_from: Some(n) } if text.len() == MAX_TEXT && *n == (MAX_TEXT + 100) as u64));
    }

    #[test]
    fn a_request_too_large_or_not_json_is_said_to_be_unread() {
        let mut a = Asked::new(true);
        let piece = vec![b' '; 1024 * 1024];
        for _ in 0..=MAX_READ_REQUEST / piece.len() {
            a.feed(&piece);
        }
        assert!(a.body.len() <= MAX_READ_REQUEST, "never more kept than the bound");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.unread[0].contains("more than"), "{:?}", line.unread);
        let mut a = Asked::new(true);
        a.feed(b"not json");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.unread[0].contains("not the JSON"));
        // A request that is not to the model's messages: counted, never kept.
        let mut a = Asked::new(false);
        a.feed(b"{\"model\":\"m\"}");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(a.body.is_empty() && a.sent == 13 && line.model.is_none() && line.unread.is_empty());
    }

    /// A stream as the service writes one: a text, then a tool call whose input comes in pieces.
    fn stream() -> String {
        let events = [
            json!({"type": "message_start", "message": {"model": "m", "usage": {"input_tokens": 12, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 40, "output_tokens": 1}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "private"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "ping"}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "I will "}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "look."}}),
            json!({"type": "content_block_stop", "index": 1}),
            json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"command\":"}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "\"ls\"}"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 57}}),
            json!({"type": "message_stop"}),
        ];
        events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect()
    }

    fn expect_the_stream(line: &Request) {
        assert_eq!(line.tokens, Some(Tokens { input: 12, cache_read: 3000, cache_write: 40, output: 57 }));
        assert_eq!(line.stop.as_deref(), Some("tool_use"));
        assert_eq!(line.answer, [
            Item::Other { what: "thinking".into() },
            Item::Text { text: "I will look.".into(), cut_from: None },
            Item::ToolCall { tool: "Bash".into(), input: "{\"command\":\"ls\"}".into(), cut_from: None },
        ], "the model's private reasoning by its kind only");
        assert!(line.unread.is_empty() && line.error.is_none());
    }

    #[test]
    fn a_streamed_answer_is_read_as_it_passes_however_it_is_cut_into_pieces() {
        let s = stream();
        for size in [1, 5, 64, s.len()] {
            let mut a = Answer::new(true, Some("text/event-stream; charset=utf-8"), None);
            for piece in s.as_bytes().chunks(size) {
                a.feed(piece);
            }
            let mut line = Request::default();
            a.read_into(&mut line);
            expect_the_stream(&line);
            assert_eq!(line.received, s.len() as u64);
        }
        // With the line ends some servers write.
        let mut a = Answer::new(true, Some("text/event-stream"), Some("identity"));
        a.feed(s.replace('\n', "\r\n").as_bytes());
        let mut line = Request::default();
        a.read_into(&mut line);
        expect_the_stream(&line);
    }

    #[test]
    fn a_compressed_stream_is_read_as_it_passes() {
        // As the service sends it (seen 2026-11-16): gzip, each event flushed as it is written.
        let mut z = GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut pieces: Vec<Vec<u8>> = Vec::new();
        for event in stream().split_inclusive("\n\n") {
            z.write_all(event.as_bytes()).unwrap();
            z.flush().unwrap();
            pieces.push(std::mem::take(z.get_mut()));
        }
        pieces.push(z.finish().unwrap());
        let mut a = Answer::new(true, Some("text/event-stream"), Some("gzip"));
        // Half-way, what has passed has been read already: nothing waits for the end.
        for p in &pieces[..7] {
            a.feed(p);
        }
        assert!(matches!(&a.blocks[1].1, Block::Text(g) if g.text == "I will look."), "read while it streams");
        for p in &pieces[7..] {
            for byte in p.chunks(3) {
                a.feed(byte);
            }
        }
        let mut line = Request::default();
        a.read_into(&mut line);
        expect_the_stream(&line);
    }

    #[test]
    fn an_answer_in_one_piece_is_read_at_its_end() {
        let body = json!({"type": "message", "model": "m", "stop_reason": "end_turn", "usage": {"input_tokens": 5, "output_tokens": 2},
            "content": [{"type": "text", "text": "OK"}, {"type": "tool_use", "id": "t", "name": "Read", "input": {"file_path": "/etc/hostname"}}]}).to_string();
        let mut a = Answer::new(true, Some("application/json"), None);
        for piece in body.as_bytes().chunks(9) {
            a.feed(piece);
        }
        let mut line = Request::default();
        a.read_into(&mut line);
        assert_eq!(line.tokens, Some(Tokens { input: 5, cache_read: 0, cache_write: 0, output: 2 }));
        assert_eq!(line.answer, [Item::Text { text: "OK".into(), cut_from: None }, Item::ToolCall { tool: "Read".into(), input: "{\"file_path\":\"/etc/hostname\"}".into(), cut_from: None }]);
        assert_eq!(line.stop.as_deref(), Some("end_turn"));
    }

    #[test]
    fn an_error_is_kept_in_the_services_words() {
        let mut a = Answer::new(true, Some("application/json"), None);
        a.feed(br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#);
        let mut line = Request::default();
        a.read_into(&mut line);
        assert_eq!(line.error.as_deref(), Some("rate_limit_error: slow down"));
        let mut a = Answer::new(true, Some("text/event-stream"), None);
        a.feed(b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\\nnow\"}}\n\n");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert_eq!(line.error.as_deref(), Some("overloaded_error: busynow"), "one line");
    }

    #[test]
    fn what_cannot_be_read_is_counted_and_said() {
        let mut a = Answer::new(true, Some("text/event-stream"), Some("br"));
        a.feed(b"\x1b\x00\x00");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.received == 3 && line.answer.is_empty());
        assert!(line.unread[0].contains("compressed in a way the record does not read (br)"), "{:?}", line.unread);

        let mut a = Answer::new(true, Some("text/event-stream"), Some("gzip"));
        a.feed(b"this is not gzip at all, whatever its header says");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.unread[0].contains("compression could not be undone"));

        // Longer than the bound: read up to it, the rest counted.
        let mut a = Answer::new(true, Some("text/event-stream"), None);
        let event = format!("data: {}\n\n", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": "x".repeat(1000)}}));
        let mut fed = 0;
        while fed <= MAX_READ_ANSWER {
            a.feed(event.as_bytes());
            fed += event.len();
        }
        assert!(a.pending.is_empty() && a.read <= MAX_READ_ANSWER);
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.unread[0].contains("longer than"));
        assert_eq!(line.received, fed as u64);
        assert_eq!(line.answer.len(), MAX_ITEMS);
        assert!(line.left_out.answer > 0);

        // An answer to something that is not the model's messages: counted only.
        let mut a = Answer::new(false, Some("application/json"), None);
        a.feed(b"{\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}");
        let mut line = Request::default();
        a.read_into(&mut line);
        assert!(line.answer.is_empty() && line.unread.is_empty() && line.received > 0);
    }

    #[test]
    fn a_long_answer_is_cut_and_says_how_long_it_was() {
        let mut a = Answer::new(true, Some("text/event-stream"), None);
        a.feed(format!("data: {}\n\n", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})).as_bytes());
        let pieces = MAX_TEXT / 100 + 50;
        for _ in 0..pieces {
            a.feed(format!("data: {}\n\n", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "y".repeat(100)}})).as_bytes());
        }
        let mut line = Request::default();
        a.read_into(&mut line);
        let Item::Text { text, cut_from } = &line.answer[0] else { panic!("a text") };
        assert_eq!((text.len(), *cut_from), (MAX_TEXT, Some(pieces as u64 * 100)), "cut to the bound, its whole length said");
    }
}
