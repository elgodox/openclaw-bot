//! Runs the `openclaw` command, on this computer or on another one over SSH.
//!
//! Secrets never go on a command line: they travel through stdin or through
//! the Gateway connection, so they can't be seen in the process list.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How to start the OpenClaw command on this computer.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    /// `node openclaw.mjs`, the safest form: no shell is involved.
    Node { node: PathBuf, script: PathBuf },
    /// A real program file.
    Exe(PathBuf),
    /// A `.cmd` launcher, which Windows can only start through `cmd`.
    Script(PathBuf),
}

/// Where a command runs.
#[derive(Clone, Debug)]
pub enum Host {
    Local,
    Ssh { target: String, identity: String },
}

fn path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // A freshly installed OpenClaw isn't on this program's PATH yet.
    if let Some(a) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(a).join("npm"));
    }
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".openclaw").join("bin"));
        dirs.push(h.join(".local").join("bin"));
    }
    if let Some(p) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(p).join("nodejs"));
    }
    dirs
}

fn find_node(near: &Path) -> Option<PathBuf> {
    let local = near.join("node.exe");
    if local.is_file() {
        return Some(local);
    }
    path_dirs().into_iter().map(|d| d.join("node.exe")).find(|p| p.is_file())
}

/// Finds the OpenClaw command, or None when it isn't installed here.
pub fn find_local() -> Option<Launch> {
    for dir in path_dirs() {
        // npm's global layout: openclaw.cmd beside node_modules/openclaw.
        let script = dir.join("node_modules").join("openclaw").join("openclaw.mjs");
        if script.is_file() && dir.join("openclaw.cmd").is_file() {
            if let Some(node) = find_node(&dir) {
                return Some(Launch::Node { node, script });
            }
        }
        let exe = dir.join("openclaw.exe");
        if exe.is_file() {
            return Some(Launch::Exe(exe));
        }
        let cmd = dir.join("openclaw.cmd");
        if cmd.is_file() {
            return Some(Launch::Script(cmd));
        }
    }
    None
}

/// Only plain characters may reach a `.cmd` launcher, because `cmd` would
/// otherwise read them as shell syntax.
fn script_safe(arg: &str) -> bool {
    !arg.chars().any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '%' | '"' | '!' | '\n' | '\r'))
}

fn posix_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Builds the process that runs `openclaw <args>` on the host.
pub fn build(host: &Host, args: &[String]) -> Result<Command, String> {
    match host {
        Host::Local => {
            let launch = find_local().ok_or("OpenClaw no está instalado en este equipo.")?;
            let mut c = match launch {
                Launch::Node { node, script } => {
                    let mut c = Command::new(node);
                    c.arg("--no-warnings").arg(script);
                    c.args(args);
                    c
                }
                Launch::Exe(p) => {
                    let mut c = Command::new(p);
                    c.args(args);
                    c
                }
                Launch::Script(p) => {
                    if let Some(bad) = args.iter().find(|a| !script_safe(a)) {
                        return Err(format!("Ese valor tiene caracteres que no se pueden usar aquí: {bad}"));
                    }
                    let mut c = Command::new("cmd");
                    c.arg("/D").arg("/C").arg(p).args(args);
                    c
                }
            };
            // Plain text, so the output can be read and shown.
            c.env("NO_COLOR", "1").env("FORCE_COLOR", "0");
            hide_console(&mut c);
            Ok(c)
        }
        Host::Ssh { target, identity } => {
            let (host, port) = crate::gateway::split_ssh_target(target);
            let mut c = Command::new("ssh");
            c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"]);
            if let Some(p) = port {
                c.args(["-p", &p]);
            }
            if !identity.trim().is_empty() {
                c.args(["-i", identity.trim()]);
            }
            let mut line = String::from("NO_COLOR=1 openclaw");
            for a in args {
                line.push(' ');
                line.push_str(&posix_quote(a));
            }
            c.arg(host).arg(line);
            hide_console(&mut c);
            Ok(c)
        }
    }
}

/// Removes terminal colour codes from a line of output.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command to the end, with a time limit. For short checks.
pub fn capture(host: &Host, args: &[String], stdin: Option<&str>, limit: Duration) -> Result<Output, String> {
    let mut c = build(host, args)?;
    capture_command(&mut c, stdin, limit)
}

pub fn capture_command(c: &mut Command, stdin: Option<&str>, limit: Duration) -> Result<Output, String> {
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("No se pudo iniciar: {e}"))?;
    if let Some(text) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            let text = text.to_string();
            thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            });
        }
    }
    let read_all = |mut r: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = vec![];
            let _ = r.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = read_all(Box::new(child.stdout.take().ok_or("sin salida")?));
    let err = read_all(Box::new(child.stderr.take().ok_or("sin salida")?));
    let until = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() > until => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Se agotó el tiempo de espera.".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(Output {
        code: status.code().unwrap_or(-1),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// The installed version, such as "2026.11.1", or None.
pub fn local_version() -> Option<String> {
    let out = capture(&Host::Local, &["--version".to_string()], None, Duration::from_secs(40)).ok()?;
    let text = strip_ansi(&out.stdout);
    text.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// The official installer for Windows; `-NoOnboard` leaves the setup to us.
pub fn official_installer(tag: &str) -> Command {
    let mut script = String::from(
        "$ProgressPreference='SilentlyContinue'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; & ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -NoOnboard",
    );
    if !tag.is_empty() && tag != "latest" && tag != "stable" {
        script.push_str(" -Tag ");
        script.push_str(tag);
    }
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    hide_console(&mut c);
    c
}

/// Splits a line typed by the person into arguments, honouring quotes.
pub fn split_args(line: &str) -> Vec<String> {
    let mut args = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args
}

// ---- background jobs -------------------------------------------------------

pub const MAX_LINES: usize = 4000;

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Running,
    Finished(i32),
    Failed(String),
    Stopped,
}

pub struct Job {
    pub id: u64,
    pub title: String,
    pub shown: String,
    pub lines: VecDeque<String>,
    pub state: JobState,
    child: Option<Arc<Mutex<Child>>>,
}

impl Job {
    pub fn text(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

enum JobEvent {
    Line(u64, String),
    End(u64, JobState),
}

/// Long-running commands, shown live.
pub struct Jobs {
    pub list: Vec<Job>,
    tx: Sender<JobEvent>,
    rx: Receiver<JobEvent>,
    next: u64,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { list: vec![], tx, rx, next: 1 }
    }
}

impl Jobs {
    /// Starts `openclaw <args>`. `shown` is what the person sees as the command.
    pub fn run(&mut self, title: &str, host: &Host, args: Vec<String>, stdin: Option<String>, repaint: impl Fn() + Send + Clone + 'static) -> u64 {
        let shown = format!("openclaw {}", args.join(" "));
        match build(host, &args) {
            Ok(c) => self.start(title, shown, c, stdin, repaint),
            Err(e) => {
                let id = self.next;
                self.next += 1;
                self.list.push(Job {
                    id,
                    title: title.to_string(),
                    shown,
                    lines: VecDeque::new(),
                    state: JobState::Failed(e),
                    child: None,
                });
                id
            }
        }
    }

    pub fn start(&mut self, title: &str, shown: String, mut cmd: Command, stdin: Option<String>, repaint: impl Fn() + Send + Clone + 'static) -> u64 {
        let id = self.next;
        self.next += 1;
        cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut job = Job {
            id,
            title: title.to_string(),
            shown,
            lines: VecDeque::new(),
            state: JobState::Running,
            child: None,
        };
        match cmd.spawn() {
            Err(e) => job.state = JobState::Failed(format!("No se pudo iniciar: {e}")),
            Ok(mut child) => {
                if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
                    thread::spawn(move || {
                        let _ = pipe.write_all(text.as_bytes());
                    });
                }
                let pump = |stream: Box<dyn Read + Send>, tx: Sender<JobEvent>, repaint: Box<dyn Fn() + Send>| {
                    thread::spawn(move || {
                        let reader = BufReader::new(stream);
                        for line in reader.split(b'\n').map_while(Result::ok) {
                            let _ = tx.send(JobEvent::Line(id, strip_ansi(&String::from_utf8_lossy(&line))));
                            repaint();
                        }
                    })
                };
                let out = child.stdout.take().map(|s| pump(Box::new(s), self.tx.clone(), Box::new(repaint.clone())));
                let err = child.stderr.take().map(|s| pump(Box::new(s), self.tx.clone(), Box::new(repaint.clone())));
                let child = Arc::new(Mutex::new(child));
                job.child = Some(child.clone());
                let tx = self.tx.clone();
                let ping = repaint.clone();
                thread::spawn(move || {
                    let state = loop {
                        if let Ok(mut c) = child.lock() {
                            match c.try_wait() {
                                Ok(Some(s)) => break JobState::Finished(s.code().unwrap_or(-1)),
                                Ok(None) => {}
                                Err(e) => break JobState::Failed(e.to_string()),
                            }
                        }
                        thread::sleep(Duration::from_millis(100));
                    };
                    for h in [out, err].into_iter().flatten() {
                        let _ = h.join();
                    }
                    let _ = tx.send(JobEvent::End(id, state));
                    ping();
                });
            }
        }
        self.list.push(job);
        if self.list.len() > 30 {
            if let Some(i) = self.list.iter().position(|j| j.state != JobState::Running) {
                self.list.remove(i);
            }
        }
        id
    }

    pub fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                JobEvent::Line(id, line) => {
                    if let Some(j) = self.list.iter_mut().find(|j| j.id == id) {
                        j.lines.push_back(line);
                        while j.lines.len() > MAX_LINES {
                            j.lines.pop_front();
                        }
                    }
                }
                JobEvent::End(id, state) => {
                    if let Some(j) = self.list.iter_mut().find(|j| j.id == id) {
                        if j.state != JobState::Stopped {
                            j.state = state;
                        }
                        j.child = None;
                    }
                }
            }
        }
    }

    pub fn stop(&mut self, id: u64) {
        if let Some(j) = self.list.iter_mut().find(|j| j.id == id) {
            if let Some(child) = &j.child {
                if let Ok(mut c) = child.lock() {
                    let _ = c.kill();
                }
            }
            if j.state == JobState::Running {
                j.state = JobState::Stopped;
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<&Job> {
        self.list.iter().find(|j| j.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_arguments() {
        assert_eq!(split_args(r#"config get "a b" 'c d' e"#), vec!["config", "get", "a b", "c d", "e"]);
        assert_eq!(split_args(r#"x """#), vec!["x", ""]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn strips_colour_codes() {
        assert_eq!(strip_ansi("\u{1b}[31mhola\u{1b}[0m\r"), "hola");
    }

    #[test]
    fn quotes_for_a_remote_shell() {
        assert_eq!(posix_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn refuses_shell_syntax_for_cmd_launchers() {
        assert!(script_safe("gateway"));
        assert!(!script_safe("x&del"));
        assert!(!script_safe("%PATH%"));
    }
}
