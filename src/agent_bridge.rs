//! Local JSONL transport between an agent process and the desktop UI thread.
//!
//! The transport owns stdin/stdout on one worker thread. Requests are handed
//! to the UI through a bounded queue; only the UI thread should execute them.
use eframe::egui;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

/// Maximum UTF-8 JSON request frame size, excluding its terminating LF.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum serialized JSON response size, excluding its terminating LF.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const QUEUE_CAPACITY: usize = 8;

/// A request waiting for UI-thread dispatch and its one-shot response route.
pub struct Envelope {
    pub request: Value,
    pub response: SyncSender<Value>,
}

/// Handle retained by the desktop application while the bridge is enabled.
pub struct Bridge {
    requests: Receiver<Envelope>,
    _worker: JoinHandle<()>,
}

impl Bridge {
    /// Starts the stdio worker. stdout is reserved exclusively for JSONL replies.
    pub fn spawn(ctx: egui::Context) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<Envelope>(QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name("leviathan-agent-stdio".into())
            .spawn(move || {
                let stdin = io::stdin();
                let stdout = io::stdout();
                let mut input = stdin.lock();
                let mut output = stdout.lock();
                let _ = serve(&mut input, &mut output, |request| {
                    let (response, receive) = mpsc::sync_channel(1);
                    let envelope = Envelope { request, response };
                    match sender.try_send(envelope) {
                        Ok(()) => {
                            ctx.request_repaint();
                            receive.recv().unwrap_or_else(|_| {
                                error("ui_disconnected", "The desktop UI stopped responding")
                            })
                        }
                        Err(TrySendError::Full(_)) => {
                            error("busy", "The desktop request queue is full")
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            error("ui_disconnected", "The desktop UI is no longer available")
                        }
                    }
                });
            })
            .map_err(|e| format!("Could not start agent stdio worker: {e}"))?;

        Ok(Self {
            requests,
            _worker: worker,
        })
    }

    /// Retrieves one pending request on the UI thread.
    pub fn try_recv(&self) -> Result<Envelope, TryRecvError> {
        self.requests.try_recv()
    }
}

/// Serve JSONL requests from any buffered input and write one JSONL reply each.
/// The callback is invoked synchronously for each valid request, making this
/// helper usable with deterministic fixtures without touching process stdio.
pub fn serve<R, W, F>(input: &mut R, output: &mut W, mut dispatch: F) -> io::Result<()>
where
    R: BufRead,
    W: Write,
    F: FnMut(Value) -> Value,
{
    while let Some(frame) = read_frame(input)? {
        let reply = match frame {
            Err(()) => error(
                "request_too_large",
                "Request exceeds the 1 MiB JSONL frame limit",
            ),
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(request) => dispatch(request),
                Err(_) => error("invalid_json", "Request is not valid JSON"),
            },
        };
        write_reply(output, &reply)?;
    }
    Ok(())
}

/// Reads one frame while retaining at most MAX_REQUEST_BYTES. Oversized input
/// is drained through LF (or EOF) so the following frame remains synchronized.
fn read_frame<R: BufRead>(input: &mut R) -> io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut frame = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;

    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if !saw_bytes {
                return Ok(None);
            }
            return Ok(Some(if oversized { Err(()) } else { Ok(frame) }));
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        saw_bytes |= content_len != 0 || newline.is_some();

        if !oversized {
            if frame.len().saturating_add(content_len) > MAX_REQUEST_BYTES {
                oversized = true;
                frame.clear();
            } else {
                frame.extend_from_slice(&available[..content_len]);
            }
        }

        let consumed = content_len + usize::from(newline.is_some());
        input.consume(consumed);
        if newline.is_some() {
            return Ok(Some(if oversized { Err(()) } else { Ok(frame) }));
        }
    }
}

fn write_reply<W: Write>(output: &mut W, reply: &Value) -> io::Result<()> {
    let mut bounded = CappedBuffer(Vec::new());
    let encoded = serde_json::to_writer(&mut bounded, reply);
    if encoded.is_err() {
        bounded.0 = serde_json::to_vec(&error(
            "response_too_large",
            "Serialized response exceeds the 16 MiB limit",
        ))
        .expect("static JSON error is serializable");
    }
    output.write_all(&bounded.0)?;
    output.write_all(b"\n")?;
    output.flush()
}

struct CappedBuffer(Vec<u8>);

impl Write for CappedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_RESPONSE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response exceeds configured JSONL limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn error(code: &str, message: &str) -> Value {
    json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(input: &[u8]) -> String {
        let mut input = Cursor::new(input);
        let mut output = Vec::new();
        serve(
            &mut input,
            &mut output,
            |request| json!({ "echo": request }),
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn dispatches_multiple_requests_and_accepts_final_frame_without_lf() {
        let output = run(b"{\"n\":1}\n{\"n\":2}");
        assert_eq!(output, "{\"echo\":{\"n\":1}}\n{\"echo\":{\"n\":2}}\n");
    }

    #[test]
    fn malformed_json_gets_a_structured_error_and_service_continues() {
        let output = run(b"{bad}\n{\"ok\":true}\n");
        let mut lines = output.lines();
        let failure: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(failure["error"]["code"], "invalid_json");
        assert_eq!(lines.next().unwrap(), "{\"echo\":{\"ok\":true}}");
        assert!(lines.next().is_none());
    }

    #[test]
    fn oversized_frame_is_drained_before_the_next_request() {
        let mut input = vec![b'x'; MAX_REQUEST_BYTES + 1];
        input.extend_from_slice(b"\n{}\n");
        let output = run(&input);
        let mut lines = output.lines();
        let failure: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(failure["error"]["code"], "request_too_large");
        assert_eq!(lines.next().unwrap(), "{\"echo\":{}}");
        assert!(lines.next().is_none());
    }

    #[test]
    fn eof_without_input_is_clean() {
        assert_eq!(run(b""), "");
    }

    #[test]
    fn oversized_response_is_replaced_with_a_bounded_error() {
        let mut input = Cursor::new(b"{}\n".as_slice());
        let mut output = Vec::new();
        serve(&mut input, &mut output, |_| {
            json!("x".repeat(MAX_RESPONSE_BYTES + 1))
        })
        .unwrap();
        let reply: Value = serde_json::from_slice(&output[..output.len() - 1]).unwrap();
        assert_eq!(reply["error"]["code"], "response_too_large");
    }
}
