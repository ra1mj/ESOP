use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use esop_cli::{ParseOutcome, USAGE, View, parse_args, render};
use esop_proto::CURRENT_SCHEMA_VERSION;
use esop_proto::v1::QueryRequest;
use esop_zenoh_gateway::KeySpace;
use esop_zenoh_gateway::runtime::ZenohGateway;

const EXIT_USAGE: i32 = 2;
const EXIT_DEGRADED: i32 = 3;
const EXIT_CONFIGURATION: i32 = 4;
const EXIT_TIMEOUT: i32 = 5;
const EXIT_QUERY: i32 = 6;
const EXIT_RENDER: i32 = 7;
const EXIT_OUTPUT: i32 = 8;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

fn write_output(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
}

async fn close(gateway: Arc<ZenohGateway>) {
    // Zenoh builders can synchronously wait while being converted to futures.
    // Keep those waits off the thread that enforces the wall-clock deadline.
    let closing = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(gateway.close())
    });
    match tokio::time::timeout(CLOSE_TIMEOUT, closing).await {
        Ok(Ok(Ok(()))) => {}
        Ok(error) => {
            eprintln!("esop: failed to close Zenoh session: {error:?}");
            std::process::exit(EXIT_QUERY);
        }
        Err(_) => {
            eprintln!("esop: session closing timed out after 1000 ms");
            // A timed-out blocking worker cannot be cancelled. Exit rather
            // than waiting indefinitely for it during Tokio runtime shutdown.
            std::process::exit(EXIT_TIMEOUT);
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    let outcome = match parse_args(std::env::args().skip(1)) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("esop: {error}\n{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    };
    let ParseOutcome::Run(cli) = outcome else {
        if let Err(error) = write_output(&format!("{USAGE}\n"))
            && error.kind() != io::ErrorKind::BrokenPipe
        {
            eprintln!("esop: failed to write output: {error}");
            std::process::exit(EXIT_OUTPUT);
        }
        return;
    };
    let key_space = match KeySpace::new(cli.fleet.as_bytes(), cli.robot.as_bytes()) {
        Ok(key_space) => key_space,
        Err(error) => {
            eprintln!("esop: invalid fleet or robot identifier: {error:?}");
            std::process::exit(EXIT_CONFIGURATION);
        }
    };
    let config = match cli.config.as_ref() {
        Some(path) => zenoh::Config::from_file(path),
        None => Ok(zenoh::Config::default()),
    };
    let config = match config {
        Ok(config) => config,
        Err(error) => {
            eprintln!("esop: failed to load Zenoh configuration: {error}");
            std::process::exit(EXIT_CONFIGURATION);
        }
    };
    let timeout = Duration::from_millis(cli.timeout_ms);
    let opening = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(ZenohGateway::open(key_space, config))
    });
    let gateway = match tokio::time::timeout(timeout, opening).await {
        Ok(Ok(Ok(gateway))) => Arc::new(gateway),
        Ok(Ok(Err(error))) => {
            eprintln!("esop: failed to open Zenoh session: {error}");
            std::process::exit(EXIT_CONFIGURATION);
        }
        Ok(Err(error)) => {
            eprintln!("esop: session opening worker failed: {error}");
            std::process::exit(EXIT_CONFIGURATION);
        }
        Err(_) => {
            eprintln!(
                "esop: session opening timed out after {} ms",
                cli.timeout_ms
            );
            std::process::exit(EXIT_TIMEOUT);
        }
    };

    let mut boot_id = cli.boot_id;
    let mut after_sequence = 0;
    let mut iteration = 0u32;
    let mut last_healthy = true;
    loop {
        let request = QueryRequest {
            robot_id: cli.robot.clone(),
            boot_id,
            after_sequence,
            limit: cli.limit,
            schema_version: CURRENT_SCHEMA_VERSION,
        };
        let response = match tokio::time::timeout(timeout, gateway.query_typed(&request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                eprintln!("esop: query failed: {error:?}");
                close(gateway.clone()).await;
                std::process::exit(EXIT_QUERY);
            }
            Err(_) => {
                eprintln!("esop: query timed out after {} ms", cli.timeout_ms);
                close(gateway.clone()).await;
                std::process::exit(EXIT_TIMEOUT);
            }
        };
        if boot_id == 0 {
            boot_id = response.boot_id;
        }
        if let Some(sequence) = response.states.last().map(|state| state.sequence) {
            after_sequence = sequence;
        }
        let display = response;
        let incident_delta = cli.view != View::IncidentList
            && display.states.is_empty()
            && !display.incidents.is_empty()
            && iteration > 0;
        let has_display_data = cli.view == View::IncidentList
            || !display.states.is_empty()
            || iteration == 0
            || incident_delta;
        if has_display_data {
            let view = if incident_delta {
                View::IncidentList
            } else {
                cli.view
            };
            let mut rendered = match render(view, &display) {
                Ok(rendered) => rendered,
                Err(error) => {
                    eprintln!("esop: invalid query result: {error}");
                    close(gateway.clone()).await;
                    std::process::exit(EXIT_RENDER);
                }
            };
            if incident_delta {
                // An incident-only update cannot replace the last state or
                // recover an earlier degraded state without fresh evidence.
                last_healthy &= rendered.healthy;
                if cli.view == View::Doctor {
                    rendered.text.push_str(if last_healthy {
                        "result=healthy\n"
                    } else {
                        "result=degraded\n"
                    });
                }
            } else {
                last_healthy = rendered.healthy;
            }
            if let Err(error) = write_output(&rendered.text) {
                close(gateway.clone()).await;
                if error.kind() == io::ErrorKind::BrokenPipe {
                    return;
                }
                eprintln!("esop: failed to write output: {error}");
                std::process::exit(EXIT_OUTPUT);
            }
        }

        iteration = iteration.saturating_add(1);
        let Some(watch) = cli.watch else {
            break;
        };
        if watch.iterations.is_some_and(|limit| iteration >= limit) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(watch.interval_ms)).await;
    }
    close(gateway).await;
    if cli.view == View::Doctor && !last_healthy {
        std::process::exit(EXIT_DEGRADED);
    }
}
