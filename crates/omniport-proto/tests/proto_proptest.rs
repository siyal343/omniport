use omniport_proto::codec::{read_message, write_message};
use omniport_proto::{MAX_FRAME_SIZE, ProtoError, Request, Response, SerialConfigDto};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn prop_request_codec_roundtrip(
        port_a in "[a-zA-Z0-9_]{3,8}",
        port_b in "[a-zA-Z0-9_]{3,8}",
        baud in 1200u32..921_600u32,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let req = Request::CreateVirtualPair {
                port_a,
                port_b,
                config: SerialConfigDto {
                    baud_rate: baud,
                    data_bits: 8,
                    stop_bits: 1,
                    parity: 0,
                },
            };

            let (mut client, mut server) = tokio::io::duplex(65536);
            write_message(&mut client, &req).await.unwrap();

            let decoded: Request = read_message(&mut server).await.unwrap();
            assert_eq!(decoded, req);
        });
    }

    #[test]
    fn prop_response_codec_roundtrip(
        uptime in 0u64..1_000_000u64,
        active_eps in 0u32..500u32,
        active_routes in 0u32..500u32,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let resp = Response::Health {
                status: "RUNNING".to_string(),
                uptime_secs: uptime,
                active_endpoints: active_eps,
                active_routes,
            };

            let (mut client, mut server) = tokio::io::duplex(65536);
            write_message(&mut client, &resp).await.unwrap();

            let decoded: Response = read_message(&mut server).await.unwrap();
            assert_eq!(decoded, resp);
        });
    }
}

#[tokio::test]
async fn test_frame_too_large_rejection() {
    let (mut client, mut server) = tokio::io::duplex(65536);
    use tokio::io::AsyncWriteExt;

    // Send a frame length exceeding MAX_FRAME_SIZE
    let oversized = (MAX_FRAME_SIZE as u32) + 1;
    client.write_all(&oversized.to_be_bytes()).await.unwrap();

    let res: Result<Request, ProtoError> = read_message(&mut server).await;
    assert!(res.is_err());
    match res.err().unwrap() {
        ProtoError::FrameTooLarge { size, max } => {
            assert_eq!(size, (MAX_FRAME_SIZE) + 1);
            assert_eq!(max, MAX_FRAME_SIZE);
        }
        other => panic!("Unexpected error: {:?}", other),
    }
}
