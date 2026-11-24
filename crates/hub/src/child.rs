//! One app's MCP server, run as a child process and spoken to over its stdin / stdout.
use crate::rpc;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpec {
    /// App id: the tool prefix.
    pub app: String,
    /// Display name for descriptions.
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The flag in `args` that lets the app write (`--allow-writes`), if it was passed.
    pub write_arg: Option<String>,
}

impl ChildSpec {
    /// The same server without its write flag, or None when it wasn't started with one. Only
    /// ever removes write permission.
    pub fn without_writes(&self) -> Option<ChildSpec> {
        let w = self.write_arg.as_ref()?;
        let args: Vec<String> = self.args.iter().filter(|a| *a != w).cloned().collect();
        (args.len() < self.args.len()).then(|| ChildSpec { args, write_arg: None, ..self.clone() })
    }
}

pub struct ChildServer {
    pub spec: ChildSpec,
    proc: Child,
    /// The tail of its stderr (kept to explain a failed start).
    stderr: Arc<Mutex<Vec<u8>>>,
    stdin: Arc<Mutex<ChildStdin>>,
    rx: Receiver<Value>,
    next_id: i64,
    /// `tools/list` as the child gave it.
    pub tools: Vec<Value>,
    pub server_name: Option<String>,
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    Timeout,
    /// The child's stdout closed (it exited or crashed).
    Gone,
    Rpc { code: i64, message: String },
    Io(String),
    /// It exited while starting: its exit code and the last of what it printed on stderr.
    Exited { code: Option<i32>, stderr: String },
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Timeout => write!(f, "the app took too long to answer"),
            CallError::Gone => write!(f, "the app's MCP server stopped"),
            CallError::Rpc { code, message } => write!(f, "{message} ({code})"),
            CallError::Io(e) => write!(f, "{e}"),
            CallError::Exited { code, stderr } => {
                match code {
                    Some(c) => write!(f, "the app's MCP server exited with code {c}")?,
                    None => write!(f, "the app's MCP server exited")?,
                }
                if !stderr.is_empty() { write!(f, ": {stderr}") } else { Ok(()) }
            }
        }
    }
}

fn write_line(stdin: &Mutex<ChildStdin>, v: &Value) -> std::io::Result<()> {
    let mut s = stdin.lock().unwrap_or_else(|e| e.into_inner());
    writeln!(s, "{v}")?;
    s.flush()
}

impl ChildServer {
    /// Starts the child, runs the `initialize` handshake and reads its tools, all within `timeout`.
    pub fn start(spec: &ChildSpec, timeout: Duration) -> Result<ChildServer, CallError> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashing up for each app
            cmd.creation_flags(0x0800_0000);
        }
        let mut proc = cmd.spawn().map_err(|e| CallError::Io(e.to_string()))?;
        let stdin = Arc::new(Mutex::new(proc.stdin.take().ok_or(CallError::Gone)?));
        let stdout = proc.stdout.take().ok_or(CallError::Gone)?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut err) = proc.stderr.take() {
            let tail = stderr.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = std::io::Read::read(&mut err, &mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    t.extend_from_slice(&buf[..n]);
                    let over = t.len().saturating_sub(STDERR_TAIL);
                    t.drain(..over);
                }
            });
        }
        let (tx, rx) = channel();
        let answer = stdin.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let is_response = msg.get("id").is_some() && (msg.get("result").is_some() || msg.get("error").is_some());
                if is_response {
                    if tx.send(msg).is_err() {
                        break;
                    }
                } else if let (Some(id), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
                    // requests from the child (sampling, roots…) aren't supported; ping is
                    let reply = if method == "ping" { rpc::ok(id.clone(), json!({})) } else { rpc::error(id.clone(), rpc::METHOD_NOT_FOUND, "not supported by otherwise-hub") };
                    let _ = write_line(&answer, &reply);
                }
            }
        });
        let mut c = ChildServer { spec: spec.clone(), proc, stderr, stdin, rx, next_id: 1, tools: vec![], server_name: None, instructions: None };
        let deadline = Instant::now() + timeout;
        let init = c.request("initialize", json!({"protocolVersion": rpc::PROTOCOL_VERSIONS[1], "capabilities": {}, "clientInfo": {"name": "otherwise-hub", "version": env!("CARGO_PKG_VERSION")}}), left(deadline));
        let init = match init {
            Ok(v) => v,
            Err(e) => return Err(c.failed(e)),
        };
        c.server_name = init.pointer("/serverInfo/name").and_then(Value::as_str).map(str::to_string);
        c.instructions = init.get("instructions").and_then(Value::as_str).map(str::to_string);
        let _ = write_line(&c.stdin, &rpc::notification("notifications/initialized", json!({})));
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cur) => json!({"cursor": cur}),
                None => json!({}),
            };
            let page = match c.request("tools/list", params, left(deadline)) {
                Ok(p) => p,
                Err(e) => return Err(c.failed(e)),
            };
            c.tools.extend(page.get("tools").and_then(Value::as_array).cloned().unwrap_or_default());
            cursor = page.get("nextCursor").and_then(Value::as_str).map(str::to_string);
            if cursor.is_none() || c.tools.len() > 2000 {
                break;
            }
        }
        Ok(c)
    }

    /// One request; waits for its response (other responses are dropped: the hub asks one
    /// thing at a time).
    pub fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        write_line(&self.stdin, &rpc::request(id, method, params)).map_err(|_| CallError::Gone)?;
        let deadline = Instant::now() + timeout;
        loop {
            match self.rx.recv_timeout(left(deadline)) {
                Ok(msg) if msg.get("id").and_then(Value::as_i64) == Some(id) => {
                    if let Some(e) = msg.get("error") {
                        return Err(CallError::Rpc { code: e.get("code").and_then(Value::as_i64).unwrap_or(rpc::INTERNAL), message: e.get("message").and_then(Value::as_str).unwrap_or("error").to_string() });
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => return Err(CallError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(CallError::Gone),
            }
        }
    }

    /// A failed start: if the process exited, says so with its code and stderr; then kills it.
    fn failed(&mut self, e: CallError) -> CallError {
        let e = if matches!(e, CallError::Gone | CallError::Rpc { .. }) {
            // stdout closed or initialize refused: give it a moment to exit and finish stderr
            let deadline = Instant::now() + Duration::from_millis(500);
            let mut status = None;
            while Instant::now() < deadline {
                if let Ok(Some(s)) = self.proc.try_wait() {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(20));
            let stderr = self.stderr_tail();
            match (status, e) {
                (Some(s), CallError::Gone) => CallError::Exited { code: s.code(), stderr },
                (_, e) => e,
            }
        } else {
            e
        };
        self.kill();
        e
    }

    /// The last line(s) it wrote on stderr, trimmed.
    pub fn stderr_tail(&self) -> String {
        let t = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&t).trim().lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.proc.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.proc.kill();
        let _ = self.proc.wait();
    }
}

impl Drop for ChildServer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// How much of a child's stderr is kept.
const STDERR_TAIL: usize = 4096;

fn left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
