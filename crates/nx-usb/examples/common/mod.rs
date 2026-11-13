use std::fs::File;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use nx_session::Transport;

/// A transport that writes every transfer to a log, so a run can be compared with the captures.
pub struct Logged<T> {
    inner: T,
    log: File,
    start: Instant,
}

impl<T> Logged<T> {
    pub fn new(inner: T, log: File) -> Self {
        Self { inner, log, start: Instant::now() }
    }

    fn line(&mut self, dir: &str, data: &[u8]) {
        let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
        _ = writeln!(self.log, "{:10.6} {dir} {hex}", self.start.elapsed().as_secs_f64());
    }
}

impl<T: Transport> Transport for Logged<T> {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()> {
        self.line(">", transfer);
        self.inner.write(transfer)
    }

    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let got = self.inner.read(timeout)?;
        if let Some(data) = &got {
            self.line("<", data);
        }
        Ok(got)
    }
}
