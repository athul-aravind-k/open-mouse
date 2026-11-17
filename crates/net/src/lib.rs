pub mod cert;
pub mod quic;
pub mod verifier;

pub use cert::{format_fingerprint, get_certs_dir, get_or_create_certificate};
pub use quic::{read_stream_packet, write_stream_packet, QuicEndpoint, QuicSession};
pub use verifier::PinnedCertVerifier;
pub use quinn;

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{MouseBtn, Packet};
    use std::net::SocketAddr;

    #[tokio::test]
    async fn test_quic_loopback_datagram_and_stream() {
        let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = QuicEndpoint::new(server_addr, "ServerHost", vec![]).unwrap();
        let bound_addr = server.local_addr().unwrap();

        let client =
            QuicEndpoint::new("127.0.0.1:0".parse().unwrap(), "ClientHost", vec![]).unwrap();

        let server_handle = tokio::spawn(async move {
            let mut session = server.accept().await.expect("Accept failed");
            let mut received = Vec::new();

            for _ in 0..2 {
                let pkt = session
                    .read_next_packet()
                    .await
                    .expect("Read packet failed");
                received.push(pkt);
            }
            received
        });

        let mut client_session = client.connect(bound_addr).await.expect("Connect failed");

        // 1. Send unreliable datagram (mouse move)
        let mouse_pkt = Packet::MouseMoveRel { dx: 15, dy: -25 };
        client_session.send_unreliable(&mouse_pkt).unwrap();

        // 2. Send reliable stream packet (click)
        let click_pkt = Packet::MouseButton {
            button: MouseBtn::Left,
            pressed: true,
        };
        client_session.send_reliable(&click_pkt).await.unwrap();

        let received = server_handle.await.unwrap();
        assert_eq!(received.len(), 2);
        assert!(received.contains(&mouse_pkt));
        assert!(received.contains(&click_pkt));
    }
}
