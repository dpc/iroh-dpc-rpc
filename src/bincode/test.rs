use bincode::{Decode, Encode};
use iroh::Endpoint;
use iroh::protocol::Router;
use test_log::test;

use crate::bincode::RpcExtBincode;
use crate::{DpcRpc, RpcExt};

const TEST_RPC_ID: u16 = 1;
const TEST_RPC_ALPN: &[u8] = b"bincode-test-rpc";

#[derive(Debug, Encode, Decode, PartialEq, Eq, Clone)]
struct TestRequest {
    id: u32,
    message: String,
    data: Vec<u8>,
}

#[derive(Debug, Encode, Decode, PartialEq, Eq, Clone)]
struct TestResponse {
    id: u32,
    message: String,
    data: Vec<u8>,
}

#[test(tokio::test)]
async fn test_bincode_rpc() {
    let server_endpoint = Endpoint::builder()
        .relay_mode(iroh::endpoint::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();

    let rpc = DpcRpc::builder(())
        .handler(TEST_RPC_ID, |_, mut w, mut r| async move {
            // Read the request using bincode
            let req: TestRequest = r.read_message_bincode().await.unwrap();

            // Create a response based on the request
            let resp = TestResponse {
                id: req.id,
                message: req.message,
                data: req.data,
            };

            // Send the response using bincode
            w.write_message_bincode(&resp).await.unwrap();

            Ok(())
        })
        .build();

    // Create server router
    let server_addr = server_endpoint.addr();

    let router = Router::builder(server_endpoint)
        .accept(TEST_RPC_ALPN, rpc)
        .spawn();

    // Create client endpoint
    let client_endpoint = Endpoint::builder().bind().await.unwrap();

    // Connect to server
    let mut conn = client_endpoint
        .connect(server_addr, TEST_RPC_ALPN)
        .await
        .unwrap();

    // Test with different data sizes
    let test_data_sizes = [10, 100, 1000, 10000];

    for (i, size) in test_data_sizes.iter().enumerate() {
        let test_data = (0..*size).map(|i| (i % 256) as u8).collect::<Vec<u8>>();

        // Create test request
        let request = TestRequest {
            id: i as u32,
            message: format!("Test message {}", i),
            data: test_data.clone(),
        };

        // Test the low-level API first
        let result = conn
            .make_rpc_raw(TEST_RPC_ID, move |mut w, mut r| async move {
                // Send request using bincode
                w.write_message_bincode(&request).await?;

                // Read response using bincode
                let response: TestResponse = r.read_message_bincode().await?;

                // Verify response matches request
                assert_eq!(response.id, request.id);
                assert_eq!(response.message, request.message);
                assert_eq!(response.data, request.data);

                Ok(())
            })
            .await;

        // Ensure the RPC succeeded
        assert!(
            result.is_ok(),
            "RPC with raw API failed for data size {}: {:?}",
            size,
            result.err()
        );
    }

    // Now test the high-level bincode API
    for (i, size) in test_data_sizes.iter().enumerate() {
        let test_data = (0..*size).map(|i| (i % 256) as u8).collect::<Vec<u8>>();

        // Create test request
        let request = TestRequest {
            id: (i + 100) as u32, // Use different IDs to distinguish from previous tests
            message: format!("High-level API test {}", i),
            data: test_data.clone(),
        };

        // Clone the request for comparison after the RPC call
        let request_clone = request.clone();

        // Use the high-level bincode API
        let response: TestResponse = conn
            .make_request_response_bincode(TEST_RPC_ID, request)
            .await
            .unwrap();

        // Verify response matches request
        assert_eq!(response.id, request_clone.id);
        assert_eq!(response.message, request_clone.message);
        assert_eq!(response.data, request_clone.data);
    }

    // Shutdown the server
    router.shutdown().await.unwrap();
}
