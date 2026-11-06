// Running the programs of a build: what they print is followed line by line,
// and a build that is cancelled is stopped with everything it started.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Runs a command and hands every line it prints (output and errors, the
/// carriage return of a progress bar counted as a line end) to `on_line`.
/// Returns its exit code, or 130 when `cancel` stopped it.
pub fn run_stream(mut cmd: Command, cancel: &AtomicBool, on_line: &mut dyn FnMut(&str)) -> Result<i32, String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Its own group, so that cancelling stops make and every compiler below it.
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut stream = stream;
            let mut buf = [0u8; 8192];
            let mut line: Vec<u8> = Vec::new();
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        for &b in &buf[..n] {
                            if b == b'\n' || b == b'\r' {
                                if !line.is_empty() {
                                    let _ = tx.send(String::from_utf8_lossy(&line).trim().to_string());
                                    line.clear();
                                }
                            } else {
                                line.push(b);
                            }
                        }
                    }
                }
            }
            if !line.is_empty() {
                let _ = tx.send(String::from_utf8_lossy(&line).trim().to_string());
            }
        }));
    }
    drop(tx);

    let mut cancelled = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if !line.is_empty() {
                    on_line(&line);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            stop_group(&mut child);
            break;
        }
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if cancelled {
        return Ok(130);
    }
    Ok(status.code().unwrap_or(1))
}

// TERM to the whole group, then KILL if it is still there after a few seconds.
fn stop_group(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
}

/// Runs a command to the end and returns its exit code and what it printed.
pub fn run_capture(mut cmd: Command) -> (i32, String) {
    match cmd.output() {
        Ok(o) => (o.status.code().unwrap_or(1), String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)),
        Err(e) => (127, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn lines_from_both_streams_and_the_exit_code() {
        let cancel = AtomicBool::new(false);
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo one; echo two 1>&2; printf 'a\\rb\\n'; exit 3"]);
        let mut lines: Vec<String> = Vec::new();
        let code = run_stream(cmd, &cancel, &mut |l| lines.push(l.to_string())).unwrap();
        assert_eq!(code, 3);
        lines.sort();
        assert_eq!(lines, vec!["a", "b", "one", "two"]);
    }

    #[test]
    fn cancelling_stops_the_whole_group() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            flag.store(true, Ordering::Relaxed);
        });
        let mut cmd = Command::new("sh");
        // A child of the shell that would run for a minute.
        cmd.args(["-c", "sleep 60 & echo started; wait"]);
        let start = Instant::now();
        let code = run_stream(cmd, &cancel, &mut |_| {}).unwrap();
        stopper.join().unwrap();
        assert_eq!(code, 130);
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn capture() {
        let (code, out) = run_capture({
            let mut c = Command::new("sh");
            c.args(["-c", "echo hi"]);
            c
        });
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "hi");
        assert_eq!(run_capture(Command::new("this-program-does-not-exist")).0, 127);
    }
}
