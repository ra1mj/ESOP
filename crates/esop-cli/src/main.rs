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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let outcome = match parse_args(std::env::args().skip(1)) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("esop: {error}\n{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    };
    let ParseOutcome::Run(cli) = outcome else {
        println!("{USAGE}");
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
    let gateway = match ZenohGateway::open(key_space, config).await {
        Ok(gateway) => gateway,
        Err(error) => {
            eprintln!("esop: failed to open Zenoh session: {error}");
            std::process::exit(EXIT_CONFIGURATION);
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
        let response = match tokio::time::timeout(
            Duration::from_millis(cli.timeout_ms),
            gateway.query_typed(&request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                eprintln!("esop: query failed: {error:?}");
                let _ = gateway.close().await;
                std::process::exit(EXIT_QUERY);
            }
            Err(_) => {
                eprintln!("esop: query timed out after {} ms", cli.timeout_ms);
                let _ = gateway.close().await;
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
        let has_display_data =
            cli.view == View::IncidentList || !display.states.is_empty() || iteration == 0;
        if has_display_data {
            let rendered = match render(cli.view, &display) {
                Ok(rendered) => rendered,
                Err(error) => {
                    eprintln!("esop: invalid query result: {error}");
                    let _ = gateway.close().await;
                    std::process::exit(EXIT_RENDER);
                }
            };
            last_healthy = rendered.healthy;
            print!("{}", rendered.text);
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
    let _ = gateway.close().await;
    if cli.view == View::Doctor && !last_healthy {
        std::process::exit(EXIT_DEGRADED);
    }
}
