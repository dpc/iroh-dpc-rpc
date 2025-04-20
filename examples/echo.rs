use std::time::{Duration, Instant};

use bincode::{Decode, Encode};
use clap::{Parser, Subcommand};
use iroh::Endpoint;
use iroh::protocol::Router;
use iroh_base::ticket::NodeTicket;
use iroh_dpc_rpc::DpcRpc;
use iroh_dpc_rpc::bincode::RpcExtBincode as _;
use tracing::info;

const ECHO_RPC_ID: u16 = 1;
pub const ECHO_RPC_ALPN: &[u8] = b"echo-rpc";

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run as a server
    Server,
    /// Run as a client
    Client {
        /// The connectivity ticket provided by the server
        #[arg(short, long)]
        ticket: String,
        /// The message to echo
        #[arg(short, long)]
        message: String,
    },
    /// Benchmark client performance
    ClientBenchmark {
        /// The connectivity ticket provided by the server
        #[arg(short, long)]
        ticket: String,
        /// Number of requests to send
        #[arg(short, long, default_value = "1000")]
        count: usize,
        /// Message size in bytes
        #[arg(short, long, default_value = "100")]
        size: usize,
        /// Number of concurrent tasks to use
        #[arg(short = 'T', long, default_value = "1")]
        tasks: usize,
    },
}

#[derive(Debug, Encode, Decode)]
struct EchoRequest {
    message: String,
}

#[derive(Debug, Encode, Decode)]
struct EchoResponse {
    message: String,
}

async fn run_server() -> anyhow::Result<()> {
    // Create the RPC service
    let rpc = DpcRpc::builder(())
        .handler(ECHO_RPC_ID, |_, mut w, mut r| async move {
            // Read the request
            let req: EchoRequest = r.read_message_bincode().await.unwrap();
            info!("Received echo request: {}", req.message);

            // Send the response
            let resp = EchoResponse {
                message: req.message,
            };
            w.write_message_bincode(&resp).await.unwrap();
        })
        .build();

    let endpoint = Endpoint::builder().bind().await?;

    let mut node_addr = endpoint.node_addr().await?;
    node_addr.direct_addresses = Default::default();
    let ticket = NodeTicket::new(node_addr);

    let router = Router::builder(endpoint)
        .accept(ECHO_RPC_ALPN, rpc)
        .spawn()
        .await?;

    println!("Server is running. Share this ticket with clients:");
    println!("{}", ticket);
    // wait until the user wants to
    tokio::signal::ctrl_c().await?;
    router.shutdown().await?;

    Ok(())
}

async fn run_client(ticket_str: &str, message: &str) -> anyhow::Result<()> {
    let ticket: NodeTicket = ticket_str.parse()?;

    let endpoint = Endpoint::builder().bind().await?;

    let mut conn = endpoint.connect(ticket, ECHO_RPC_ALPN).await?;

    let request = EchoRequest {
        message: message.to_string(),
    };

    println!("Sending message: {}", message);
    let response: EchoResponse = conn
        .make_request_response_bincode(ECHO_RPC_ID, request)
        .await?;

    println!("Received echo response: {}", response.message);

    Ok(())
}

async fn run_client_benchmark(
    ticket_str: &str,
    count: usize,
    size: usize,
    tasks: usize,
) -> anyhow::Result<()> {
    let ticket: NodeTicket = ticket_str.parse()?;

    // Generate a message of the specified size
    let message = "A".repeat(size);

    println!(
        "Starting benchmark with {} requests of {} bytes each using {} tasks",
        count, size, tasks
    );

    // Calculate requests per task
    let requests_per_task = count / tasks;
    let remainder = count % tasks;

    // Create a mutex to collect results from all tasks
    let latencies_mutex = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::with_capacity(count)));
    let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(tasks);

    for task_id in 0..tasks {
        // Calculate how many requests this task should handle
        let task_requests = if task_id < remainder {
            requests_per_task + 1
        } else {
            requests_per_task
        };

        if task_requests == 0 {
            continue;
        }

        // Clone shared resources for this task
        let ticket_clone = ticket.clone();
        let message_clone = message.clone();
        let latencies_clone = latencies_mutex.clone();
        let progress_clone = progress.clone();

        // Spawn a task
        let handle = tokio::spawn(async move {
            let mut task_latencies = Vec::with_capacity(task_requests);
            let endpoint = Endpoint::builder().bind().await?;
            let mut conn = endpoint.connect(ticket_clone, ECHO_RPC_ALPN).await?;

            for _ in 0..task_requests {
                let request = EchoRequest {
                    message: message_clone.clone(),
                };

                let start = Instant::now();
                let _: EchoResponse = conn
                    .make_request_response_bincode(ECHO_RPC_ID, request)
                    .await?;
                let elapsed = start.elapsed();

                task_latencies.push(elapsed);

                // Update and report progress
                let current = progress_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if current % 100 == 0 || current == count {
                    println!("Progress: {}/{}", current, count);
                }
            }

            // Add results to the shared collection
            let mut latencies = latencies_clone.lock().await;
            latencies.extend(task_latencies);

            Ok::<_, anyhow::Error>(())
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await??;
    }

    // Get all latencies
    let mut latencies = std::sync::Arc::try_unwrap(latencies_mutex)
        .expect("All tasks should be done with the mutex")
        .into_inner();

    // Calculate statistics
    latencies.sort();

    let min = latencies.first().unwrap_or(&Duration::ZERO);
    let max = latencies.last().unwrap_or(&Duration::ZERO);
    let median = latencies.get(count / 2).unwrap_or(&Duration::ZERO);
    let p99 = latencies
        .get((count as f64 * 0.99) as usize)
        .unwrap_or(&Duration::ZERO);

    let sum: Duration = latencies.iter().sum();
    let mean = sum / count as u32;

    // Print histogram
    println!("\nLatency Statistics:");
    println!("Min: {:?}", min);
    println!("Max: {:?}", max);
    println!("Mean: {:?}", mean);
    println!("Median: {:?}", median);
    println!("p99: {:?}", p99);

    // Create a simple histogram with 10 buckets
    let range = max.as_micros() - min.as_micros();
    let bucket_size = range / 10;

    if bucket_size > 0 {
        println!("\nHistogram (microseconds):");

        let mut buckets = [0; 10];
        for latency in &latencies {
            let bucket = ((latency.as_micros() - min.as_micros()) / bucket_size).min(9) as usize;
            buckets[bucket] += 1;
        }

        let max_count = *buckets.iter().max().unwrap_or(&1) as f64;

        for (i, count) in buckets.iter().enumerate() {
            let start = min.as_micros() + (i as u128 * bucket_size);
            let end = min.as_micros() + ((i + 1) as u128 * bucket_size);
            let bar_length = ((*count as f64 / max_count) * 40.0) as usize;
            let bar = "#".repeat(bar_length);

            println!("{:6}-{:6} µs [{:4}]: {}", start, end, count, bar);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server => run_server().await?,
        Commands::Client { ticket, message } => run_client(&ticket, &message).await?,
        Commands::ClientBenchmark {
            ticket,
            count,
            size,
            tasks,
        } => run_client_benchmark(&ticket, count, size, tasks).await?,
    }

    Ok(())
}
