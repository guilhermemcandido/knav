//! A shell inside knav: `kubectl exec -it` in a pseudo-terminal, its screen emulated
//! (vt100) and drawn in the interface.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub struct ShellSession {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: (u16, u16),
}

/// Bash if the image has it, else sh.
const SHELL_PICKER: &str = "command -v bash >/dev/null 2>&1 && exec bash || exec sh";

impl ShellSession {
    /// A shell in `container` of `pod`, drawn in a `rows` x `cols` screen.
    pub fn exec(context: &str, namespace: &str, pod: &str, container: &str, rows: u16, cols: u16) -> Result<Self> {
        let mut command = CommandBuilder::new("kubectl");
        command.args(["--context", context, "exec", "-it", "-n", namespace, pod, "-c", container, "--", "sh", "-c", SHELL_PICKER]);
        command.env("TERM", "xterm-256color");
        Self::spawn(command, rows, cols).context("couldn't run kubectl (is it on the PATH?)")
    }

    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> Result<Self> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));
        let exited = Arc::new(AtomicBool::new(false));
        {
            let (parser, exited) = (Arc::clone(&parser), Arc::clone(&exited));
            std::thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut parser) = parser.lock() {
                                parser.process(&buffer[..n]);
                            }
                        }
                    }
                }
                exited.store(true, Ordering::SeqCst);
            });
        }
        Ok(ShellSession { parser, writer, master: pair.master, child, exited, size: (rows, cols) })
    }

    pub fn send(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Whether the program has ended (the shell exited, or the connection dropped).
    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        if let Ok(mut parser) = self.parser.lock() {
            parser.screen_mut().set_size(rows, cols);
        }
    }

    /// Whether the program asked for application cursor keys.
    pub fn app_cursor(&self) -> bool {
        self.parser.lock().map(|p| p.screen().application_cursor()).unwrap_or(false)
    }

    pub fn with_screen<R>(&self, read: impl FnOnce(&vt100::Screen) -> R) -> Option<R> {
        self.parser.lock().ok().map(|p| read(p.screen()))
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(session: &ShellSession, what: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if session.with_screen(|s| s.contents().contains(what)).unwrap_or(false) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn sh(script: &str) -> CommandBuilder {
        let mut command = CommandBuilder::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn program_output_lands_on_the_emulated_screen_and_the_exit_is_noticed() {
        let session = ShellSession::spawn(sh("printf hello-from-pty"), 10, 40).unwrap();
        assert!(wait_for(&session, "hello-from-pty"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.exited());
    }

    #[test]
    fn typed_bytes_reach_the_program() {
        let mut session = ShellSession::spawn(sh("read line; echo got:$line"), 10, 40).unwrap();
        session.send(b"abc\r");
        assert!(wait_for(&session, "got:abc"));
    }

    #[test]
    fn resizing_changes_the_screen_and_the_program_sees_it() {
        let mut session = ShellSession::spawn(sh("sleep 0.3; stty size"), 10, 40).unwrap();
        session.resize(20, 60);
        assert!(wait_for(&session, "20 60"));
    }
}
