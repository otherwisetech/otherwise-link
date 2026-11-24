//! Server-sent events (`text/event-stream`): the parser the client reads `…/events` with, and
//! the writer servers use, so both ends agree on framing.
use std::io::BufRead;

/// One dispatched SSE message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    /// `event:` (default `message`).
    pub event: String,
    /// `id:`, the last one seen when this message had none.
    pub id: Option<String>,
    /// `data:` lines joined with `\n`.
    pub data: String,
}

/// Reads messages from a stream; comments (`: keepalive`) and `retry:` are skipped.
pub struct Reader<R> {
    inner: R,
    last_id: Option<String>,
}

impl<R: BufRead> Reader<R> {
    pub fn new(inner: R) -> Reader<R> {
        Reader { inner, last_id: None }
    }

    pub fn last_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// The next message; `Ok(None)` at the end of the stream.
    pub fn next_message(&mut self) -> std::io::Result<Option<Message>> {
        let (mut event, mut data, mut has_data) = (String::new(), String::new(), false);
        let mut line = String::new();
        loop {
            line.clear();
            if self.inner.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            let l = line.trim_end_matches(['\n', '\r']);
            if l.is_empty() {
                if has_data {
                    let event = if event.is_empty() { "message".to_string() } else { event };
                    return Ok(Some(Message { event, id: self.last_id.clone(), data }));
                }
                event.clear();
                continue;
            }
            if l.starts_with(':') {
                continue;
            }
            let (field, value) = match l.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (l, ""),
            };
            match field {
                "event" => event = value.to_string(),
                "data" => {
                    if has_data {
                        data.push('\n');
                    }
                    data.push_str(value);
                    has_data = true;
                }
                "id" if !value.contains('\0') => self.last_id = Some(value.to_string()),
                _ => {}
            }
        }
    }
}

/// One message as a server writes it.
pub fn format(event: &str, id: Option<&str>, data: &str) -> String {
    let mut s = String::new();
    s.push_str("event: ");
    s.push_str(event);
    s.push('\n');
    if let Some(id) = id {
        s.push_str("id: ");
        s.push_str(id);
        s.push('\n');
    }
    for l in data.split('\n') {
        s.push_str("data: ");
        s.push_str(l);
        s.push('\n');
    }
    s.push('\n');
    s
}

/// A comment line that keeps proxies and idle timers from closing the stream.
pub const KEEPALIVE: &str = ": keepalive\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_format_writes_and_skips_comments() {
        let text = format!("{KEEPALIVE}retry: 3000\n\n{}{}data: plain\n\n", format("changed", Some("7"), "{\"a\":1}"), format("bye", None, "one\ntwo"));
        let mut r = Reader::new(std::io::Cursor::new(text));
        let m = r.next_message().unwrap().unwrap();
        assert_eq!((m.event.as_str(), m.id.as_deref(), m.data.as_str()), ("changed", Some("7"), "{\"a\":1}"));
        let m = r.next_message().unwrap().unwrap();
        assert_eq!((m.event.as_str(), m.id.as_deref(), m.data.as_str()), ("bye", Some("7"), "one\ntwo"));
        let m = r.next_message().unwrap().unwrap();
        assert_eq!(m.event, "message");
        assert!(r.next_message().unwrap().is_none());
        assert_eq!(r.last_id(), Some("7"));
    }

    #[test]
    fn crlf_and_a_truncated_last_message() {
        let mut r = Reader::new(std::io::Cursor::new("event: x\r\ndata: 1\r\n\r\nevent: y\ndata: 2\n"));
        assert_eq!(r.next_message().unwrap().unwrap().data, "1");
        assert!(r.next_message().unwrap().is_none(), "a message without its blank line isn't dispatched");
    }
}
