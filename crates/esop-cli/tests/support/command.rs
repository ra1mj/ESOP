use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

struct Process {
    child: Option<Child>,
    directory: PathBuf,
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub async fn run(config: &str, args: &[&str]) -> Output {
    run_with_stdout(config, args, false).await
}

pub async fn run_with_stdout(config: &str, args: &[&str], broken_pipe: bool) -> Output {
    let directory = std::env::temp_dir().join(format!(
        "esop-cli-process-{}-{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).expect("create isolated process fixture");
    let mut process = Process {
        child: None,
        directory,
    };
    let path = process.directory.join("zenoh.json5");
    std::fs::write(&path, config).expect("write isolated config");
    // Files avoid pipe backpressure hiding shutdown failures.
    let stdout = process.directory.join("stdout");
    let stderr = process.directory.join("stderr");
    process.child = Some(
        Command::new(env!("CARGO_BIN_EXE_esop"))
            .args(["--fleet", "fleet_a", "--robot", "robot_01", "--config"])
            .arg(path)
            .args(["--timeout-ms", "1500"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(if broken_pipe {
                Stdio::piped()
            } else {
                std::fs::File::create(&stdout).unwrap().into()
            })
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .expect("start real esop binary"),
    );
    if broken_pipe {
        drop(process.child.as_mut().unwrap().stdout.take());
        std::fs::write(&stdout, []).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = process.child.as_mut().unwrap().try_wait().unwrap() {
            return Output {
                status,
                stdout: std::fs::read(stdout).unwrap(),
                stderr: std::fs::read(stderr).unwrap(),
            };
        }
        assert!(
            Instant::now() < deadline,
            "CLI exceeded test deadline: {args:?}; stderr={}",
            std::fs::read_to_string(&stderr).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn assert_exit(output: &Output, expected: i32) -> String {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    String::from_utf8(output.stdout.clone()).expect("UTF-8 command output")
}
