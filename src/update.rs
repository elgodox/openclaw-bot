//! Self-update from the project's GitHub releases.
//!
//! The app asks GitHub for the latest release. When its version is newer than
//! this one, the person can install it from a banner: the new program file is
//! downloaded next to the running one, the running file is renamed aside
//! (Windows allows renaming a program that is open), the new one takes its
//! name, and the app restarts. The old file is removed on the next start.

use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const REPO: &str = "elgodox/openclaw-bot";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// How often a running app looks again.
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The Windows program attached to the release.
    pub fn program(&self) -> Option<&Asset> {
        let exes = || self.assets.iter().filter(|a| a.name.to_lowercase().ends_with(".exe"));
        exes().find(|a| a.name.to_lowercase().contains("setup")).or_else(|| exes().next())
    }
}

pub fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

/// True when `tag` names a newer version than `current`.
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(t), Some(c)) => t > c,
        _ => false,
    }
}

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { version: String, done: u64, total: u64 },
    /// Downloaded and swapped in; the app restarts itself.
    Restarting,
    Failed(String),
}

pub struct Updater {
    pub state: Arc<Mutex<State>>,
    /// The person chose "later" for this version; no banner until a newer one.
    pub dismissed: Option<String>,
    last_check: Option<Instant>,
    enabled: bool,
}

impl Updater {
    pub fn new(enabled: bool) -> Self {
        Self { state: Default::default(), dismissed: None, last_check: None, enabled }
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn busy(&self) -> bool {
        matches!(self.state(), State::Checking | State::Downloading { .. } | State::Restarting)
    }

    /// Looks for a new release now, in the background.
    pub fn check_now(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        self.last_check = Some(Instant::now());
        set(&self.state, State::Checking);
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let next = match latest_release() {
                Ok(r) if is_newer(&r.tag_name, CURRENT) && r.program().is_some() => State::Available(r),
                Ok(_) => State::UpToDate,
                Err(e) => State::Failed(e),
            };
            set(&state, next);
            ctx.request_repaint();
        });
    }

    /// Called every frame: checks at start and then every few hours.
    pub fn tick(&mut self, ctx: &eframe::egui::Context) {
        if !self.enabled {
            return;
        }
        let due = self.last_check.is_none_or(|t| t.elapsed() > RECHECK);
        if due {
            self.check_now(ctx);
        }
    }

    /// Downloads and installs `release`, then restarts the app.
    pub fn install(&mut self, release: Release, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        let Some(asset) = release.program().cloned() else {
            set(&self.state, State::Failed("La versión nueva no trae un programa para Windows.".into()));
            return;
        };
        let version = release.tag_name.clone();
        set(&self.state, State::Downloading { version: version.clone(), done: 0, total: asset.size });
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let exe = replace_self(&asset, |done, total| {
                    set(&state, State::Downloading { version: version.clone(), done, total });
                    ctx.request_repaint();
                })?;
                set(&state, State::Restarting);
                ctx.request_repaint();
                let mut cmd = std::process::Command::new(&exe);
                cmd.arg("--updated");
                cmd.spawn().map_err(|e| format!("No se pudo reiniciar: {e}"))?;
                Ok::<(), String>(())
            })();
            match result {
                Ok(()) => {
                    // Give the new copy a moment to show its window, then leave.
                    std::thread::sleep(Duration::from_millis(400));
                    std::process::exit(0);
                }
                Err(e) => {
                    set(&state, State::Failed(e));
                    ctx.request_repaint();
                }
            }
        });
    }
}

/// Downloads `asset` and puts it in place of the running program file.
/// Returns that path; the running copy keeps working until it exits.
pub fn replace_self(asset: &Asset, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let fresh = sibling(&exe, "new");
    download(&asset.browser_download_url, &fresh, progress)?;
    swap_in(&exe, &fresh)?;
    Ok(exe)
}

/// `--update-now`: the same update without a window. Exit code 0 when up to
/// date or updated.
pub fn run_cli() -> i32 {
    println!("OpenClaw Bot {CURRENT}");
    let release = match latest_release() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if !is_newer(&release.tag_name, CURRENT) {
        println!("Ya tienes la última versión ({}).", release.tag_name);
        return 0;
    }
    let Some(asset) = release.program() else {
        eprintln!("La versión {} no trae un programa para Windows.", release.tag_name);
        return 1;
    };
    println!("Descargando {} ({})…", release.tag_name, asset.name);
    match replace_self(asset, |_, _| {}) {
        Ok(exe) => {
            println!("Actualizado a {} en {}", release.tag_name, exe.display());
            crate::setup::refresh_registered_version();
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn set(state: &Arc<Mutex<State>>, s: State) {
    if let Ok(mut g) = state.lock() {
        *g = s;
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("openclaw-bot-updater/", env!("CARGO_PKG_VERSION")))
        .build()
}

pub fn latest_release() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "Todavía no hay versiones publicadas.".to_string(),
            ureq::Error::Status(code, _) => format!("GitHub respondió {code}."),
            other => format!("Sin conexión con GitHub: {other}"),
        })?;
    let r: Release = resp.into_json().map_err(|e| format!("Respuesta inesperada de GitHub: {e}"))?;
    if r.draft || r.prerelease {
        return Err("La última versión aún no es estable.".into());
    }
    Ok(r)
}

fn download(url: &str, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let resp = agent().get(url).call().map_err(|e| format!("No se pudo descargar: {e}"))?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("No se pudo guardar la descarga: {e}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now();
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            format!("La descarga se cortó: {e}")
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("No se pudo guardar la descarga: {e}"))?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(100) {
            progress(done, total);
            last = Instant::now();
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if total > 0 && done != total {
        let _ = std::fs::remove_file(dest);
        return Err("La descarga quedó incompleta.".into());
    }
    if done < 1024 * 1024 {
        let _ = std::fs::remove_file(dest);
        return Err("La descarga no parece un programa válido.".into());
    }
    progress(done, total.max(done));
    Ok(())
}

fn sibling(exe: &Path, ext: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(ext);
    exe.with_file_name(name)
}

/// Puts `fresh` where `exe` is, keeping the running file aside as `.old`.
fn swap_in(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = sibling(exe, "old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(fresh);
        format!("No se pudo reemplazar el programa (¿carpeta protegida?): {e}")
    })?;
    if let Err(e) = std::fs::rename(fresh, exe) {
        // Put the working copy back.
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(fresh);
        return Err(format!("No se pudo instalar la versión nueva: {e}"));
    }
    Ok(())
}

/// Removes what a previous update left behind.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        for ext in ["old", "new"] {
            let p = sibling(&exe, ext);
            if p.exists() {
                // The previous copy may still be closing.
                for _ in 0..20 {
                    if std::fs::remove_file(&p).is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
    }

    #[test]
    fn picks_the_setup_program() {
        let a = |n: &str| Asset { name: n.into(), browser_download_url: String::new(), size: 0 };
        let r = Release {
            tag_name: "v1.2.0".into(),
            html_url: String::new(),
            draft: false,
            prerelease: false,
            assets: vec![a("notes.txt"), a("OpenClaw-Bot.exe"), a("OpenClaw-Bot-Setup.exe")],
        };
        assert_eq!(r.program().unwrap().name, "OpenClaw-Bot-Setup.exe");
    }

    #[test]
    fn sibling_names() {
        let p = Path::new(r"C:\x\OpenClaw Bot.exe");
        assert_eq!(sibling(p, "old"), PathBuf::from(r"C:\x\OpenClaw Bot.exe.old"));
    }
}
