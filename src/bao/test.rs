use iroh::protocol::Router;
use iroh::{Endpoint, Watcher as _};
use iroh_base::ticket::NodeTicket;
use test_log::test;

use crate::{DpcRpc, RpcExt};

const TEST_RPC_ID: u16 = 1;
const TEST_RPC_ALPN: &[u8] = b"bao-test-rpc";

#[test(tokio::test)]
async fn test_bao_rpc() {
    let server_endpoint = Endpoint::builder()
        .relay_mode(iroh::endpoint::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();

    let rpc = DpcRpc::builder(())
        .handler(TEST_RPC_ID, |_, _w, mut r| async move {
            let data_raw = r.read_message_raw().await.unwrap();
            let hash = bao_tree::blake3::hash(&data_raw);

            let data_bao = r
                .read_message_bao(data_raw.len().try_into().unwrap(), hash)
                .await
                .unwrap();

            assert_eq!(data_raw, data_bao);
            Ok(())
        })
        .build();

    // Create server router
    let server_addr = server_endpoint.node_addr().initialized().await.unwrap();
    let ticket = NodeTicket::new(server_addr);

    let router = Router::builder(server_endpoint)
        .accept(TEST_RPC_ALPN, rpc)
        .spawn();

    // Create client endpoint
    let client_endpoint = Endpoint::builder().bind().await.unwrap();

    // Connect to server
    let mut conn = client_endpoint
        .connect(ticket, TEST_RPC_ALPN)
        .await
        .unwrap();

    // Create test data of various sizes to verify bao encoding/decoding
    let test_data_sizes = [10, 100, 1000, 10000];

    for size in test_data_sizes {
        let test_data = (0..size).map(|i| (i % 256) as u8).collect::<Vec<u8>>();

        // Make RPC request
        let result = conn
            .make_rpc_raw(TEST_RPC_ID, move |mut w, _r| async move {
                let hash = bao_tree::blake3::hash(&test_data);

                w.write_message_raw(&test_data).await?;

                let encoded_hash = w.write_message_bao(&test_data).await?;

                assert_eq!(encoded_hash, hash);

                Ok(())
            })
            .await;

        // Ensure the RPC succeeded
        assert!(
            result.is_ok(),
            "RPC failed for data size {}: {:?}",
            size,
            result.err()
        );
    }

    // Shutdown the server
    router.shutdown().await.unwrap();
}
