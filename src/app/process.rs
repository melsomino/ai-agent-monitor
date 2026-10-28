//! Bounded subprocess reads shared by discovery backends.
use anyhow::{Context, Result};
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<(Option<i32>, String, String)> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().context("missing process output")?;
    let reader = thread::spawn(move || {
        let mut output = String::new();
        stdout.read_to_string(&mut output).map(|_| output)
    });
    let mut stderr = child
        .stderr
        .take()
        .context("missing process error output")?;
    let error_reader = thread::spawn(move || {
        let mut errors = String::new();
        stderr.read_to_string(&mut errors).map(|_| errors)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("process output reader failed"))??;
    let errors = error_reader
        .join()
        .map_err(|_| anyhow::anyhow!("process error reader failed"))??;
    let status = status.context("session discovery timed out")?;
    Ok((status.code(), output, errors))
}
