use crate::cert::get_or_create_certificate;
use crate::verifier::PinnedCertVerifier;
use bytes::Bytes;
use proto::{deserialize_packet, serialize_packet, Packet};
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream, ServerConfig};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub struct QuicEndpoint {
    endpoint: Endpoint,
    cert_fingerprint: String,
    trusted_fingerprints: Arc<Mutex<Vec<String>>>,
    discovered_fingerprint: Arc<Mutex<Option<String>>>,
}

impl QuicEndpoint {
    pub fn new(
        bind_addr: SocketAddr,
        device_name: &str,
        trusted: Vec<String>,
    ) -> Result<Self, String> {
        let (cert, key, fingerprint) = get_or_create_certificate(device_name)?;

        let trusted_fingerprints = Arc::new(Mutex::new(trusted));
        let discovered_fingerprint = Arc::new(Mutex::new(None));

        // 1. Server Config
        let server_crypto = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| e.to_string())?;

        let quic_server_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|e| e.to_string())?;
        let mut server_config = ServerConfig::with_crypto(Arc::new(quic_server_crypto));

        // Transport configuration
        let mut transport_config = quinn::TransportConfig::default();
        transport_config.max_idle_timeout(Some(Duration::from_secs(8).try_into().unwrap()));
        transport_config.keep_alive_interval(Some(Duration::from_secs(1)));
        server_config.transport_config(Arc::new(transport_config));

        // 2. Client Config
        let verifier = Arc::new(PinnedCertVerifier::new(
            trusted_fingerprints.clone(),
            discovered_fingerprint.clone(),
        ));
        let client_crypto = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

        let quic_client_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| e.to_string())?;
        let client_config = ClientConfig::new(Arc::new(quic_client_crypto));

        // 3. Endpoint
        let mut endpoint = Endpoint::server(server_config, bind_addr)
            .map_err(|e| format!("Failed to bind QUIC endpoint on {}: {}", bind_addr, e))?;
        endpoint.set_default_client_config(client_config);

        Ok(Self {
            endpoint,
            cert_fingerprint: fingerprint,
            trusted_fingerprints,
            discovered_fingerprint,
        })
    }

    pub fn cert_fingerprint(&self) -> &str {
        &self.cert_fingerprint
    }

    pub fn trusted_fingerprints(&self) -> Vec<String> {
        self.trusted_fingerprints.lock().unwrap().clone()
    }

    pub fn add_trusted_fingerprint(&self, fp: String) {
        let mut list = self.trusted_fingerprints.lock().unwrap();
        if !list.contains(&fp) {
            list.push(fp);
        }
    }

    pub fn discovered_fingerprint(&self) -> Option<String> {
        self.discovered_fingerprint.lock().unwrap().clone()
    }

    pub fn local_addr(&self) -> Result<SocketAddr, String> {
        self.endpoint.local_addr().map_err(|e| e.to_string())
    }

    pub async fn accept(&self) -> Option<QuicSession> {
        let incoming = self.endpoint.accept().await?;
        let conn = incoming.await.ok()?;
        QuicSession::from_connection(conn, false).await.ok()
    }

    pub async fn connect(&self, target: SocketAddr) -> Result<QuicSession, String> {
        let conn = self
            .endpoint
            .connect(target, "localhost")
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;

        QuicSession::from_connection(conn, true).await
    }
}

pub struct QuicSession {
    connection: Connection,
    send_stream: SendStream,
    recv_stream: RecvStream,
}

impl QuicSession {
    async fn from_connection(connection: Connection, is_initiator: bool) -> Result<Self, String> {
        let (send_stream, recv_stream) = if is_initiator {
            connection.open_bi().await.map_err(|e| e.to_string())?
        } else {
            connection.accept_bi().await.map_err(|e| e.to_string())?
        };

        Ok(Self {
            connection,
            send_stream,
            recv_stream,
        })
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.connection.remote_address()
    }

    pub fn rtt(&self) -> Duration {
        self.connection.rtt()
    }

    /// Sends high-frequency, loss-tolerant packets (like MouseMoveRel) via unreliable QUIC datagrams.
    pub fn send_unreliable(&self, packet: &Packet) -> Result<(), String> {
        let bytes = serialize_packet(packet).map_err(|e| e.to_string())?;
        self.connection
            .send_datagram(Bytes::from(bytes))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Sends critical packets (Keys, Clicks, Enter, Leave, Clipboard, Heartbeats) via reliable ordered stream.
    pub async fn send_reliable(&mut self, packet: &Packet) -> Result<(), String> {
        let payload = serialize_packet(packet).map_err(|e| e.to_string())?;
        let len = payload.len() as u32;
        self.send_stream
            .write_all(&len.to_le_bytes())
            .await
            .map_err(|e| e.to_string())?;
        self.send_stream
            .write_all(&payload)
            .await
            .map_err(|e| e.to_string())?;
        self.send_stream.flush().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reads next packet from either unreliable datagrams or reliable stream.
    pub async fn read_next_packet(&mut self) -> Result<Packet, String> {
        tokio::select! {
            // Priority 1: Unreliable datagram (mouse delta)
            datagram_res = self.connection.read_datagram() => {
                let bytes = datagram_res.map_err(|e| e.to_string())?;
                deserialize_packet(&bytes).map_err(|e| e.to_string())
            }
            // Priority 2: Reliable stream packet
            stream_res = read_stream_packet(&mut self.recv_stream) => {
                stream_res
            }
        }
    }
}

async fn read_stream_packet(stream: &mut RecvStream) -> Result<Packet, String> {
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > 32 * 1024 * 1024 {
        return Err(format!("Reliable packet size {} exceeds 32MB limit", len));
    }

    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    deserialize_packet(&buf).map_err(|e| e.to_string())
}
