use std::fs::{self, OpenOptions};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Each test owns its router, port and cleanup, including on assertion failure.
pub struct Router {
    child: Option<Child>,
    address: SocketAddr,
    log_path: PathBuf,
}

impl Router {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("allocate loopback port");
        let address = listener.local_addr().unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let log_path =
            std::env::temp_dir().join(format!("esop-zenoh-{}-{nonce}.log", std::process::id()));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&log_path)
            .expect("create private router log");
        let mut router = Self {
            child: None,
            address,
            log_path,
        };
        drop(listener);
        router.start();
        router
    }

    pub fn endpoint(&self) -> String {
        format!("tcp/{}", self.address)
    }

    pub fn start(&mut self) {
        assert!(self.child.is_none(), "router is already running");
        let log = OpenOptions::new()
            .append(true)
            .open(&self.log_path)
            .unwrap();
        self.child = Some(
            Command::new(std::env::var_os("ZENOHD").unwrap_or_else(|| "zenohd".into()))
                .args([
                    "--listen",
                    &self.endpoint(),
                    "--no-multicast-scouting",
                    "--cfg=scouting/gossip/enabled:false",
                ])
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .expect("start zenohd (install 1.10.1 or set ZENOHD)"),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "router exited during startup; see {}",
                self.log_path.display()
            );
            if TcpStream::connect_timeout(&self.address, Duration::from_millis(100)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "router startup timed out");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn stop(&mut self) {
        let mut child = self.child.take().expect("router is running");
        let _ = child.kill();
        child.wait().expect("reap router");
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking() {
            eprintln!(
                "router log {}:\n{}",
                self.log_path.display(),
                fs::read_to_string(&self.log_path).unwrap_or_default()
            );
        } else {
            let _ = fs::remove_file(&self.log_path);
        }
    }
}

pub fn client_config(endpoint: &str) -> zenoh::Config {
    zenoh::Config::from_json5(&format!(
        r#"{{
        mode: "client",
        listen: {{ endpoints: [] }},
        connect: {{ endpoints: ["{endpoint}"], timeout_ms: 3000,
            retry: {{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 2 }} }},
        scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }}
    }}"#
    ))
    .expect("valid isolated client configuration")
}
