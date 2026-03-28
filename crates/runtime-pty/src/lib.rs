use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, bounded};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc::{Receiver as AsyncReceiver, channel};

pub mod reconciliation;

pub const CRATE_NAME: &str = "runtime-pty";

pub const DEFAULT_READ_BUFFER_SIZE: usize = 65536;
pub const DEFAULT_CHANNEL_SIZE: usize = 50;

#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub read_buffer_size: usize,
    pub channel_size: usize,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            read_buffer_size: DEFAULT_READ_BUFFER_SIZE,
            channel_size: DEFAULT_CHANNEL_SIZE,
        }
    }
}

pub struct PtyBytes {
    pub bytes: Vec<u8>,
}

pub enum PtyEvent {
    Bytes(PtyBytes),
    Exit(i32),
}

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    output_rx: Option<Receiver<Vec<u8>>>,
    _reader_thread: thread::JoinHandle<()>,
}

impl PtySession {
    pub fn spawn(command: &str, args: &[&str], cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with_config(command, args, cols, rows, None, PtyConfig::default())
    }

    pub fn spawn_in_dir(
        command: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> Result<Self> {
        Self::spawn_with_config(command, args, cols, rows, cwd, PtyConfig::default())
    }

    pub fn spawn_with_config(
        command: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        config: PtyConfig,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open PTY")?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn command in PTY: {command}"))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .context("failed to take PTY writer")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;

        let (tx, rx) = bounded(config.channel_size);
        let buffer_size = config.read_buffer_size;
        let reader_thread = thread::spawn(move || {
            let mut buf = vec![0_u8; buffer_size];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            child,
            output_rx: Some(rx),
            _reader_thread: reader_thread,
        })
    }

    pub fn send_input(&mut self, input: &str) -> Result<()> {
        self.writer
            .write_all(input.as_bytes())
            .context("failed writing to PTY")?;
        self.writer.flush().context("failed flushing PTY writer")?;
        Ok(())
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer
            .write_all(bytes)
            .context("failed writing to PTY")?;
        self.writer.flush().context("failed flushing PTY writer")?;
        Ok(())
    }

    pub fn take_output_receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.output_rx.take()
    }

    pub fn output_receiver(&self) -> Option<&Receiver<Vec<u8>>> {
        self.output_rx.as_ref()
    }

    pub fn read_output(&self, timeout: Duration) -> Result<Vec<u8>> {
        let Some(rx) = &self.output_rx else {
            return Ok(Vec::new());
        };

        const MAX_CHUNKS_PER_READ: usize = 64;
        const COALESCE_WAIT: Duration = Duration::from_millis(5);
        let mut out = Vec::new();

        if let Ok(first_chunk) = rx.recv_timeout(timeout) {
            out.extend_from_slice(&first_chunk);
            let mut chunk_count = 1;
            while chunk_count < MAX_CHUNKS_PER_READ {
                let chunk = match rx.recv_timeout(COALESCE_WAIT) {
                    Ok(chunk) => chunk,
                    Err(_) => break,
                };
                out.extend_from_slice(&chunk);
                chunk_count += 1;
            }
        }

        Ok(out)
    }

    pub fn try_read_output(&self) -> Result<Vec<u8>> {
        let Some(rx) = &self.output_rx else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to resize PTY")?;
        Ok(())
    }

    pub fn interrupt(&mut self) -> Result<()> {
        self.child.kill().context("failed to interrupt PTY child")
    }

    pub fn terminate(&mut self) -> Result<()> {
        let kill_result = self.child.kill();
        let wait_result = self.child.wait();

        match (kill_result, wait_result) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(kill_err), Ok(_)) => {
                let message = kill_err.to_string();
                if message.contains("No such process") {
                    Ok(())
                } else {
                    Err(kill_err).context("failed to terminate PTY child")
                }
            }
            (Ok(_), Err(wait_err)) => Err(wait_err).context("failed waiting for PTY child exit"),
            (Err(kill_err), Err(wait_err)) => {
                let message = kill_err.to_string();
                if message.contains("No such process") {
                    Err(wait_err).context("failed waiting for PTY child exit")
                } else {
                    Err(anyhow::anyhow!(
                        "failed to terminate PTY child: kill error: {kill_err}; wait error: {wait_err}"
                    ))
                }
            }
        }
    }

    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct AsyncPtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    output_rx: AsyncReceiver<Vec<u8>>,
    _reader_thread: thread::JoinHandle<()>,
}

impl AsyncPtySession {
    pub fn spawn(command: &str, args: &[&str], cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with_config(command, args, cols, rows, None, PtyConfig::default())
    }

    pub fn spawn_in_dir(
        command: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> Result<Self> {
        Self::spawn_with_config(command, args, cols, rows, cwd, PtyConfig::default())
    }

    pub fn spawn_with_config(
        command: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        config: PtyConfig,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to open PTY")?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn command in PTY: {command}"))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .context("failed to take PTY writer")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone PTY reader")?;

        let (tx, rx) = channel(config.channel_size);
        let buffer_size = config.read_buffer_size;
        let reader_thread = thread::spawn(move || {
            let mut buf = vec![0_u8; buffer_size];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            child,
            output_rx: rx,
            _reader_thread: reader_thread,
        })
    }

    pub fn send_input(&mut self, input: &str) -> Result<()> {
        self.writer
            .write_all(input.as_bytes())
            .context("failed writing to PTY")?;
        self.writer.flush().context("failed flushing PTY writer")?;
        Ok(())
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer
            .write_all(bytes)
            .context("failed writing to PTY")?;
        self.writer.flush().context("failed flushing PTY writer")?;
        Ok(())
    }

    pub async fn read_output(&mut self) -> Option<Vec<u8>> {
        self.output_rx.recv().await
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to resize PTY")?;
        Ok(())
    }

    pub fn interrupt(&mut self) -> Result<()> {
        self.child.kill().context("failed to interrupt PTY child")
    }

    pub fn terminate(&mut self) -> Result<()> {
        let kill_result = self.child.kill();
        let wait_result = self.child.wait();

        match (kill_result, wait_result) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(kill_err), Ok(_)) => {
                let message = kill_err.to_string();
                if message.contains("No such process") {
                    Ok(())
                } else {
                    Err(kill_err).context("failed to terminate PTY child")
                }
            }
            (Ok(_), Err(wait_err)) => Err(wait_err).context("failed waiting for PTY child exit"),
            (Err(kill_err), Err(wait_err)) => {
                let message = kill_err.to_string();
                if message.contains("No such process") {
                    Err(wait_err).context("failed waiting for PTY child exit")
                } else {
                    Err(anyhow::anyhow!(
                        "failed to terminate PTY child: kill error: {kill_err}; wait error: {wait_err}"
                    ))
                }
            }
        }
    }

    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
}

impl Drop for AsyncPtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::{AsyncPtySession, PtyConfig, PtySession};

    #[test]
    fn launches_interactive_session_and_streams_output() {
        let mut session = PtySession::spawn("/bin/sh", &["-c", "cat"], 80, 24)
            .expect("cat process should spawn in PTY");

        session
            .send_input("hello from pty\n")
            .expect("input should be sent");

        let output = session
            .read_output(Duration::from_secs(2))
            .expect("should read output bytes");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("hello from pty"),
            "expected echoed input in output, got: {text:?}"
        );

        session.resize(100, 40).expect("resize should succeed");
        session.terminate().expect("terminate should succeed");
    }

    #[test]
    fn handles_resize_storm_without_failure() {
        let mut session = PtySession::spawn("/bin/sh", &["-c", "cat"], 80, 24)
            .expect("cat process should spawn in PTY");

        for i in 0..60 {
            let cols = 80 + (i % 40) as u16;
            let rows = 24 + (i % 20) as u16;
            session.resize(cols, rows).expect("resize should succeed");
        }

        session.terminate().expect("terminate should succeed");
    }

    #[test]
    fn supports_bracketed_paste_payload_roundtrip() {
        let mut session = PtySession::spawn("/bin/sh", &["-c", "cat"], 80, 24)
            .expect("cat process should spawn in PTY");

        let payload = "\u{1b}[200~pasted text\u{1b}[201~\n";
        session
            .send_input(payload)
            .expect("pasted payload should send");
        let mut text = String::new();
        for _ in 0..5 {
            let output = session
                .read_output(Duration::from_millis(400))
                .expect("output should read");
            text.push_str(&String::from_utf8_lossy(&output));
            if text.contains("pasted text") {
                break;
            }
        }

        assert!(text.contains("pasted text"));
        session.terminate().expect("terminate should succeed");
    }

    #[test]
    fn interrupt_stops_long_running_process() {
        let mut session = PtySession::spawn("/bin/sh", &["-c", "sleep 5"], 80, 24)
            .expect("sleep process should spawn in PTY");

        session.interrupt().expect("interrupt should succeed");
    }

    #[test]
    fn spawn_in_dir_runs_command_in_requested_workdir() {
        let cwd = std::env::temp_dir();
        let session = PtySession::spawn_in_dir("/bin/sh", &["-c", "pwd"], 80, 24, Some(&cwd))
            .expect("pwd process should spawn in PTY");

        let output = session
            .read_output(Duration::from_secs(1))
            .expect("should read output bytes");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains(cwd.to_string_lossy().as_ref()),
            "expected pwd output to contain cwd, got: {text:?}"
        );
    }

    #[test]
    fn bounded_channel_backpressure_works() {
        let config = PtyConfig {
            read_buffer_size: 4096,
            channel_size: 2,
        };

        let mut session =
            PtySession::spawn_with_config("/bin/sh", &["-c", "yes hello"], 80, 24, None, config)
                .expect("yes process should spawn in PTY");

        std::thread::sleep(Duration::from_millis(100));

        let output = session
            .read_output(Duration::from_millis(500))
            .expect("should read output");

        assert!(!output.is_empty());

        session.terminate().expect("terminate should succeed");
    }

    #[test]
    fn slo_latency_p95_local_echo_under_budget() {
        let mut session = PtySession::spawn("/bin/sh", &["-c", "cat"], 80, 24)
            .expect("cat process should spawn in PTY");

        let sample_count = 30;
        let mut samples_ms = Vec::with_capacity(sample_count);
        for idx in 0..sample_count {
            let payload = format!("slo-sample-{idx}\n");
            let start = Instant::now();
            session
                .send_input(&payload)
                .expect("input write should succeed");

            let expected = format!("slo-sample-{idx}");
            let mut combined = String::new();
            let mut matched = false;
            for _ in 0..20 {
                let output = session
                    .read_output(Duration::from_millis(100))
                    .expect("output should be readable");
                combined.push_str(&String::from_utf8_lossy(&output));
                if combined.contains(&expected) {
                    matched = true;
                    break;
                }
            }

            assert!(
                matched,
                "timed out waiting for echoed sample '{}', got: {:?}",
                expected, combined
            );
            samples_ms.push(start.elapsed().as_millis() as u64);
        }

        samples_ms.sort_unstable();
        let p95_index = ((samples_ms.len() * 95) / 100).min(samples_ms.len() - 1);
        let p95 = samples_ms[p95_index];
        assert!(p95 <= 150, "p95 input-to-echo latency too high: {p95}ms");

        session.terminate().expect("terminate should succeed");
    }

    #[tokio::test]
    async fn async_session_reads_output() {
        let mut session = AsyncPtySession::spawn("/bin/sh", &["-c", "echo async-test"], 80, 24)
            .expect("echo process should spawn in PTY");

        let output = session.read_output().await.expect("should receive output");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("async-test"),
            "expected 'async-test' in output, got: {text:?}"
        );

        session.terminate().expect("terminate should succeed");
    }

    #[tokio::test]
    async fn async_session_handles_input() {
        let mut session = AsyncPtySession::spawn("/bin/sh", &["-c", "cat"], 80, 24)
            .expect("cat process should spawn in PTY");

        session
            .send_input("hello async\n")
            .expect("input should be sent");

        let output = session.read_output().await.expect("should receive output");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("hello async"),
            "expected 'hello async' in output, got: {text:?}"
        );

        session.terminate().expect("terminate should succeed");
    }
}
