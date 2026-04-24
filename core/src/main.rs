mod benchmarks;
mod errors;
mod simulation_service;

use crate::errors::AppError;
use axum::{routing::get, Router};
use simulation_service::SimulationService;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let db_path =
        env::var("SOROSCOPE_DB_PATH").unwrap_or_else(|_| "soroscope_metrics.db".to_string());
    let webhook_url = env::var("SOROSCOPE_ALERT_WEBHOOK_URL").ok();
    let simulation_service = match SimulationService::new(db_path, webhook_url) {
        Ok(service) => Arc::new(service),
        Err(err) => {
            eprintln!("Failed to initialize simulation service: {}", err);
            return;
        }
    };

    // CLI Argument Handling
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "benchmark" {
        println!("Starting SoroScope Benchmark...");
        // Locate WASM - assuming running from workspace root or core
        // Attempt to find it relative to current directory or know location
        let possible_paths = vec![
            "target/wasm32-unknown-unknown/release/soroban_token_contract.wasm",
            "../target/wasm32-unknown-unknown/release/soroban_token_contract.wasm",
        ];

        let mut wasm_path = None;
        for p in possible_paths {
            let path = PathBuf::from(p);
            if path.exists() {
                wasm_path = Some(path);
                break;
            }
        }

        if let Some(path) = wasm_path {
            if let Err(e) = benchmarks::run_token_benchmark(path, simulation_service.as_ref()).await
            {
                eprintln!("Benchmark failed: {}", e);
            }
        } else {
            eprintln!("Could not find soroban_token_contract.wasm. Make sure to build the contract first.");
        }
        return;
    }

    // Default Web Server
    println!("SoroScope CLI Initialized. Run with 'benchmark' argument to profile token contract.");

    // build our application with a single route
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                "Hello from SoroScope! Usage: cargo run -p soroscope-core -- benchmark"
            }),
        )
        .route(
            "/error",
            get(|| async { Err::<&str, AppError>(AppError::BadRequest("Test error".to_string())) }),
        );

    // run it with hyper on localhost:3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
