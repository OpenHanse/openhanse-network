use openhanse_test_runner::{
    Backend, BackendSession, LaunchRequest, ProcessKey, RunnerEvent, run_native_app,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::mpsc::Sender,
    thread,
};

struct PtyBackend;

impl Backend for PtyBackend {
    fn name(&self) -> &'static str {
        "pty"
    }

    fn spawn(
        &self,
        request: LaunchRequest,
        tx: Sender<RunnerEvent>,
    ) -> Result<Box<dyn BackendSession>, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 140,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("failed to allocate PTY: {error}"))?;

        let mut command = CommandBuilder::new(&request.cli_path);
        command.cwd(&request.workspace_dir);
        command.env("TERM", "xterm-256color");
        for arg in &request.args {
            command.arg(arg);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("failed to spawn PTY child: {error}"))?;
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("failed to clone PTY reader: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("failed to take PTY writer: {error}"))?;

        spawn_pty_reader(request.key, reader, tx.clone());
        let _ = tx.send(RunnerEvent::Status {
            key: request.key,
            text: "running (pty)".to_string(),
        });

        Ok(Box::new(PtySession { child, writer }))
    }
}

struct PtySession {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

impl BackendSession for PtySession {
    fn send_line(&mut self, line: &str) -> Result<(), String> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|error| format!("failed to write to PTY: {error}"))
    }

    fn stop(&mut self) -> Result<(), String> {
        self.child
            .kill()
            .map_err(|error| format!("failed to kill PTY child: {error}"))?;
        let _ = self.child.wait();
        Ok(())
    }
}

fn spawn_pty_reader<R>(key: ProcessKey, mut reader: R, tx: Sender<RunnerEvent>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    let text = String::from_utf8_lossy(&buffer[..len]).to_string();
                    let _ = tx.send(RunnerEvent::Output {
                        key,
                        channel: "pty",
                        text,
                    });
                }
                Err(error) => {
                    let _ = tx.send(RunnerEvent::Output {
                        key,
                        channel: "error",
                        text: format!("PTY reader failed: {error}"),
                    });
                    break;
                }
            }
        }
    });
}

fn main() -> Result<(), eframe::Error> {
    run_native_app("OpenHanse Test Runner - PTY", PtyBackend)
}
