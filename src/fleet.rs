//! La sección "Fleet": administra la flota de máquinas propias invocando el
//! toolkit `fleet/orchestrator/fleet.py` (Headscale + SSH/Ansible + RustDesk).
//!
//! No usa el gateway de OpenClaw: corre el orquestador como un proceso aparte,
//! igual que la sección Servicio corre la CLI de `openclaw`.

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// Una máquina del inventario, tal como la devuelve `fleet.py list --json`.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub hostname: String,
    pub mesh_ip: String,
    pub os: String,
    pub online: bool,
}

enum FleetEvent {
    Inventory(Result<Vec<Machine>, String>),
    EnrollCmd(Result<String, String>),
}

pub struct FleetState {
    /// Carpeta donde está fleet.py (fleet/orchestrator). Se persiste en settings.
    pub dir: String,
    /// Intérprete de Python ("python", "py", ruta completa…).
    pub python: String,
    /// Passphrase maestra del vault (en memoria, no se guarda en disco).
    pub passphrase: String,

    pub machines: Vec<Machine>,
    pub loading: bool,
    /// El toolkit dijo que le falta una dependencia de Python.
    pub missing_deps: bool,
    pub loaded_once: bool,
    pub error: Option<String>,

    // Panel "ejecutar comando".
    pub target: String,
    pub command: String,
    pub use_sudo: bool,

    // Panel "enrolar".
    pub enroll_platform: usize, // 0 = Linux, 1 = Windows
    pub enroll_cmd: String,

    // Panel "vault".
    pub vault_host: String,
    pub vault_pw: String,

    tx: Sender<FleetEvent>,
    rx: Receiver<FleetEvent>,
}

impl Default for FleetState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            dir: String::new(),
            python: "python".into(),
            passphrase: String::new(),
            machines: vec![],
            loading: false,
            missing_deps: false,
            loaded_once: false,
            error: None,
            target: String::new(),
            command: String::new(),
            use_sudo: false,
            enroll_platform: 0,
            enroll_cmd: String::new(),
            vault_host: String::new(),
            vault_pw: String::new(),
            tx,
            rx,
        }
    }
}

impl FleetState {
    pub fn is_configured(&self) -> bool {
        !self.dir.trim().is_empty() && Path::new(self.dir.trim()).join("fleet.py").is_file()
    }

    /// Construye el proceso `python fleet.py <args>` dentro de la carpeta del toolkit.
    pub fn command(&self, args: &[&str]) -> Result<Command, String> {
        let dir = self.dir.trim();
        if dir.is_empty() {
            return Err("Configurá la carpeta del toolkit (fleet/orchestrator).".into());
        }
        if !Path::new(dir).join("fleet.py").is_file() {
            return Err(format!("No encuentro fleet.py en «{dir}»."));
        }
        let py = if self.python.trim().is_empty() { "python" } else { self.python.trim() };
        let mut c = Command::new(py);
        c.arg("fleet.py").args(args).current_dir(dir);
        c.env("NO_COLOR", "1").env("FORCE_COLOR", "0").env("PYTHONIOENCODING", "utf-8");
        if !self.passphrase.is_empty() {
            c.env("FLEET_VAULT_PASSPHRASE", &self.passphrase);
        }
        crate::cli::hide_console(&mut c);
        Ok(c)
    }

    /// `python -m pip install -r requirements.txt` en la carpeta del toolkit.
    pub fn install_deps_command(&self) -> Result<Command, String> {
        let dir = self.dir.trim();
        if !Path::new(dir).join("requirements.txt").is_file() {
            return Err(format!("No encuentro requirements.txt en «{dir}»."));
        }
        let py = if self.python.trim().is_empty() { "python" } else { self.python.trim() };
        let mut c = Command::new(py);
        c.args(["-m", "pip", "install", "-r", "requirements.txt"]).current_dir(dir);
        c.env("PYTHONIOENCODING", "utf-8");
        crate::cli::hide_console(&mut c);
        Ok(c)
    }

    fn fail(&mut self, msg: String) {
        let (text, deps) = friendly_error(&msg);
        self.missing_deps = deps;
        self.error = Some(text);
    }

    /// Lanza, en segundo plano, `fleet.py list --json` y guarda el inventario.
    pub fn refresh(&mut self, repaint: impl Fn() + Send + 'static) {
        if self.loading {
            return;
        }
        let cmd = match self.command(&["list", "--json"]) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.loading = true;
        self.error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut cmd = cmd;
            let parsed = match crate::cli::capture_command(&mut cmd, None, Duration::from_secs(30)) {
                Ok(o) if o.code == 0 => parse_machines(&o.stdout),
                Ok(o) => {
                    let msg = crate::cli::strip_ansi(if o.stderr.trim().is_empty() { &o.stdout } else { &o.stderr });
                    Err(if msg.trim().is_empty() { "El comando falló.".into() } else { msg.trim().to_string() })
                }
                Err(e) => Err(e),
            };
            let _ = tx.send(FleetEvent::Inventory(parsed));
            repaint();
        });
    }

    /// Lanza, en segundo plano, `fleet.py enroll-cmd <plataforma>`.
    pub fn refresh_enroll(&mut self, repaint: impl Fn() + Send + 'static) {
        let plat = if self.enroll_platform == 0 { "linux" } else { "win" };
        let cmd = match self.command(&["enroll-cmd", plat]) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut cmd = cmd;
            let out = match crate::cli::capture_command(&mut cmd, None, Duration::from_secs(20)) {
                Ok(o) if o.code == 0 => Ok(crate::cli::strip_ansi(&o.stdout).trim().to_string()),
                Ok(o) => Err(crate::cli::strip_ansi(&o.stderr).trim().to_string()),
                Err(e) => Err(e),
            };
            let _ = tx.send(FleetEvent::EnrollCmd(out));
            repaint();
        });
    }

    /// Recibe los resultados de los hilos de fondo. Se llama en cada frame.
    pub fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                FleetEvent::Inventory(r) => {
                    self.loading = false;
                    match r {
                        Ok(m) => {
                            self.machines = m;
                            self.loaded_once = true;
                            self.missing_deps = false;
                        }
                        Err(e) => self.fail(e),
                    }
                }
                FleetEvent::EnrollCmd(r) => match r {
                    Ok(s) => {
                        self.enroll_cmd = s;
                        self.missing_deps = false;
                    }
                    Err(e) => self.fail(e),
                },
            }
        }
    }
}

/// Turns what the toolkit printed on failure into one readable line, and says
/// whether the problem is a missing Python package.
fn friendly_error(raw: &str) -> (String, bool) {
    let text = crate::cli::strip_ansi(raw);
    let deps = text.contains("ModuleNotFoundError") || text.contains("Falta la dependencia de Python");
    if deps {
        return ("Al toolkit le faltan dependencias de Python. Tocá «Instalar dependencias».".into(), true);
    }
    if text.contains("No such file") && text.contains("python") || text.contains("program not found") {
        return ("No encuentro Python. Revisá el intérprete configurado.".into(), false);
    }
    // De un traceback, la última línea es la que explica el error.
    let last = text.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("El comando falló.");
    (last.to_string(), false)
}

fn parse_machines(stdout: &str) -> Result<Vec<Machine>, String> {
    let clean = crate::cli::strip_ansi(stdout);
    let text = clean.trim();
    // La CLI puede imprimir una línea de aviso antes del JSON; tomamos desde el '['.
    let json = text.find('[').map(|i| &text[i..]).unwrap_or(text);
    let v: Value = serde_json::from_str(json).map_err(|e| format!("No pude leer el inventario: {e}"))?;
    let arr = v.as_array().ok_or("Formato de inventario inesperado.")?;
    Ok(arr
        .iter()
        .map(|m| Machine {
            hostname: m.get("hostname").and_then(Value::as_str).unwrap_or("?").to_string(),
            mesh_ip: m.get("mesh_ip").and_then(Value::as_str).unwrap_or("?").to_string(),
            os: m.get("os").and_then(Value::as_str).unwrap_or("?").to_string(),
            online: m.get("online").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_inventory_after_a_notice() {
        let out = "aviso: algo\n[{\"hostname\":\"pc1\",\"mesh_ip\":\"100.64.0.1\",\"os\":\"linux\",\"online\":true}]";
        let m = parse_machines(out).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].hostname, "pc1");
        assert!(m[0].online);
    }

    #[test]
    fn explains_missing_packages_and_tracebacks() {
        let (msg, deps) = friendly_error("Traceback (most recent call last):\n  File x\nModuleNotFoundError: No module named 'cryptography'\n");
        assert!(deps);
        assert!(msg.contains("Instalar dependencias"));
        let (msg, deps) = friendly_error("Traceback:\n  File y\nValueError: unknown url type\n");
        assert!(!deps);
        assert_eq!(msg, "ValueError: unknown url type");
    }
}
