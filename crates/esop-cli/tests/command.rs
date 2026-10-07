#[path = "support/command.rs"]
mod command;

use command::{assert_exit, run};

const OFFLINE: &str = r#"{
    mode: "peer", listen: { endpoints: [] }, connect: { endpoints: [] },
    scouting: { multicast: { enabled: false }, gossip: { enabled: false } }
}"#;

#[tokio::test]
async fn binary_help_usage_and_configuration_errors() {
    assert!(assert_exit(&run(OFFLINE, &["--help"]).await, 0).contains("commands:"));
    assert_exit(
        &command::run_with_stdout(OFFLINE, &["--help"], true).await,
        0,
    );
    for args in [
        vec![],
        vec!["unknown"],
        vec!["watch", "--interval-ms", "0"],
        vec!["status", "--timeout-ms", "0"],
        vec!["status", "--limit", "33"],
    ] {
        assert_exit(&run(OFFLINE, &args).await, 2);
    }
    assert_exit(&run("not valid configuration", &["status"]).await, 4);
    assert_exit(&run(OFFLINE, &["--robot", "bad/id", "status"]).await, 4);
}

#[tokio::test]
async fn every_operational_form_handles_absent_provider_without_panicking() {
    for view in [
        vec!["status"],
        vec!["domain", "list"],
        vec!["dc"],
        vec!["lifecycle"],
        vec!["incident", "list"],
        vec!["doctor"],
        vec!["watch", "--iterations", "1"],
    ] {
        let output = run(OFFLINE, &view).await;
        assert_exit(&output, 6);
        assert!(String::from_utf8_lossy(&output.stderr).contains("query failed"));
    }
}

#[tokio::test]
async fn session_opening_has_a_wall_clock_bound() {
    use std::net::TcpListener;

    // Keep a TCP port open without completing a Zenoh handshake. The CLI's
    // own deadline must win over the deliberately long transport timeout.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let config = format!(
        r#"{{
        mode: "client", listen: {{ endpoints: [] }},
        connect: {{ endpoints: ["tcp/{endpoint}"], timeout_ms: 60000 }},
        scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }}
    }}"#
    );
    let output = run(&config, &["--timeout-ms", "100", "status"]).await;
    assert_exit(&output, 5);
    assert!(String::from_utf8_lossy(&output.stderr).contains("session opening timed out"));
}
