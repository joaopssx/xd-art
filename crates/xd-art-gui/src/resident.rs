//! The optional resident instance (off by default).
//!
//! Started with `--resident`, or with the "stay resident" setting on, xd-art listens on
//! a local TCP port (127.0.0.1 only) and writes the port and a random token to
//! `resident.txt` next to the config file. A later launch in resident mode reads that
//! file, sends its paths over the port with the token, and exits before creating any
//! window: the running instance opens the images and comes to the front.
//!
//! Closing the window of the resident instance minimizes it and drops the image, the
//! results and the caches instead of quitting, so the next launch only costs a
//! connection. Ctrl+Q quits for real and removes `resident.txt`.
//!
//! Plain TCP instead of a Unix socket or a named pipe keeps one code path on every
//! system with only the standard library. Anyone on the machine can connect to a
//! local port; the token in a file only this user can read is what keeps them from
//! sending paths.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use gpui::{Context, Window};
use xd_art_core::pipeline::Pipeline;

use crate::app::{Doc, Root};

/// Largest message accepted: plenty for a few hundred paths.
const MAX_MESSAGE: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_millis(500);

/// The running instance's side.
pub struct Server {
    /// Paths sent by later launches; an empty list only asks to show the window.
    pub rx: Receiver<Vec<PathBuf>>,
    info: PathBuf,
}

impl Server {
    /// Removes `resident.txt`, so later launches start their own instance.
    pub fn stop(&self) {
        let _ = std::fs::remove_file(&self.info);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where the port and token are written: next to the config file.
pub fn info_path() -> Option<PathBuf> {
    crate::config::path().map(|p| p.with_file_name("resident.txt"))
}

fn token() -> String {
    // RandomState is seeded from the system's random source once per process.
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    let a = h.finish();
    let b = RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

/// The paths of a message, if its first line is `token`.
pub fn parse(message: &str, token: &str) -> Option<Vec<PathBuf>> {
    let mut lines = message.lines();
    if lines.next()? != token {
        return None;
    }
    Some(lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

/// Hands `paths` to a running instance. True when one took them; false when there is
/// none (no file, nothing listening, wrong token), and the caller should start one.
pub fn send(info: &Path, paths: &[PathBuf]) -> bool {
    let Ok(text) = std::fs::read_to_string(info) else {
        return false;
    };
    let mut parts = text.split_whitespace();
    let (Some(Ok(port)), Some(token)) = (parts.next().map(str::parse::<u16>), parts.next()) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, TIMEOUT) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let mut msg = format!("{token}\n");
    for p in paths {
        // The other process may have another working directory.
        let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        msg.push_str(&p.to_string_lossy());
        msg.push('\n');
    }
    if s.write_all(msg.as_bytes()).is_err() || s.shutdown(std::net::Shutdown::Write).is_err() {
        return false;
    }
    let mut reply = String::new();
    s.read_to_string(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Writes `text` to a file only the current user can read. On Windows the config
/// folder under `%APPDATA%` already is; on Unix the file gets mode 0600.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(text.as_bytes())
}

/// Starts listening and writes `info`. `None` when the port or the file cannot be
/// set up; xd-art then runs as a normal instance.
pub fn listen(info: &Path) -> Option<Server> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = token();
    if let Some(dir) = info.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    write_private(info, &format!("{port} {token}\n")).ok()?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("xd-art resident".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                let _ = conn.set_read_timeout(Some(TIMEOUT));
                let mut msg = String::new();
                let read = (&conn).take(MAX_MESSAGE).read_to_string(&mut msg);
                let Some(paths) = read.ok().and_then(|_| parse(&msg, &token)) else {
                    continue;
                };
                let _ = conn.write_all(b"ok\n");
                if tx.send(paths).is_err() {
                    return;
                }
            }
        })
        .ok()?;
    Some(Server {
        rx,
        info: info.to_path_buf(),
    })
}

/// How often the window checks for paths from later launches.
const POLL: Duration = Duration::from_millis(100);

impl Root {
    /// Makes this window the resident instance: closing only minimizes it, and paths
    /// from later launches open here.
    pub(crate) fn become_resident(
        &mut self,
        server: Server,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(root) = this.upgrade() else {
                return true;
            };
            root.update(cx, |root, cx| root.unload(cx));
            window.minimize_window();
            false
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update_in(cx, |root, window, cx| {
                    let Some(server) = &root.resident else {
                        return;
                    };
                    let mut asked = false;
                    let mut paths = Vec::new();
                    while let Ok(p) = server.rx.try_recv() {
                        asked = true;
                        paths.extend(p);
                    }
                    if asked {
                        window.activate_window();
                        if !paths.is_empty() {
                            root.open_many(paths, window, cx);
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        self.resident = Some(server);
    }

    /// Drops the image and everything computed from it, keeping the settings.
    pub(crate) fn unload(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        self.generation += 1;
        self.doc = Doc::Empty;
        self.output = None;
        self.proxy = None;
        self.last_run = None;
        self.frames = crate::frames::Frames::default();
        self.retouch.reset(None);
        self.gallery = crate::gallery::Gallery::default();
        self.variations = crate::variations::Variations::default();
        self.history.start(&self.settings);
        self.strip.colors.clear();
        self.strip.counts.clear();
        *self
            .pipeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Pipeline::default();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_checks_the_token() {
        let msg = "abc\nC:\\a.png\n/b c.png\n\n";
        assert_eq!(
            parse(msg, "abc"),
            Some(vec![PathBuf::from("C:\\a.png"), PathBuf::from("/b c.png")])
        );
        assert_eq!(parse(msg, "abd"), None);
        assert_eq!(parse("abc", "abc"), Some(Vec::new()));
        assert_eq!(parse("", "abc"), None);
    }

    #[test]
    fn second_launch_hands_paths_over() {
        let dir = std::env::temp_dir().join(format!("xd-art-resident-{}", std::process::id()));
        let info = dir.join("resident.txt");
        assert!(!send(&info, &[]), "no instance yet");
        let server = listen(&info).unwrap();
        let p = dir.join("cat.png");
        assert!(send(&info, std::slice::from_ref(&p)));
        assert_eq!(server.rx.recv_timeout(Duration::from_secs(2)), Ok(vec![p]));
        // A wrong token is ignored and gets no reply.
        let text = std::fs::read_to_string(&info).unwrap();
        let port = text.split_whitespace().next().unwrap();
        std::fs::write(&info, format!("{port} 0000\n")).unwrap();
        assert!(!send(&info, &[]));
        assert!(server.rx.recv_timeout(Duration::from_millis(200)).is_err());
        drop(server);
        assert!(!info.exists(), "stopping removes the file");
        std::fs::remove_dir_all(&dir).ok();
    }
}
