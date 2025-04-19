use bincode::{Decode, Encode};
use clap::{Parser, Subcommand};
use iroh::Endpoint;
use iroh::protocol::Router;
use iroh_base::ticket::NodeTicket;
use iroh_dpc_rpc::{DpcRpc, RpcExt};
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
            let req: EchoRequest = r.read_message().await.unwrap();
            info!("Received echo request: {}", req.message);

            // Send the response
            let resp = EchoResponse {
                message: req.message,
            };
            w.write_message(&resp).await.unwrap();
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
    let response: EchoResponse = conn.make_request_response(ECHO_RPC_ID, request).await?;

    println!("Received echo response: {}", response.message);

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server => run_server().await?,
        Commands::Client { ticket, message } => run_client(&ticket, &message).await?,
    }

    Ok(())
}
