use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsAcceptor;
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_util::either::Either;

const TLS_HANDSHAKE_RECORD: u8 = 0x16;
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct LocalPlainAcceptor {
    tls: RustlsAcceptor,
}

impl LocalPlainAcceptor {
    pub(crate) fn new(tls: RustlsAcceptor) -> Self {
        Self { tls }
    }
}

impl<S> Accept<TcpStream, S> for LocalPlainAcceptor
where
    S: Send + 'static,
{
    type Stream = Either<TcpStream, TlsStream<TcpStream>>;
    type Service = S;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let tls = self.tls.clone();
        Box::pin(async move {
            if is_local_peer(stream.peer_addr().ok()) && !opens_with_tls(&stream).await? {
                return Ok((Either::Left(stream), service));
            }
            let (stream, service) = tls.accept(stream, service).await?;
            Ok((Either::Right(stream), service))
        })
    }
}

fn is_local_peer(peer: Option<SocketAddr>) -> bool {
    peer.is_some_and(|peer| peer.ip().to_canonical().is_loopback())
}

async fn opens_with_tls(stream: &TcpStream) -> io::Result<bool> {
    let mut first = [0_u8; 1];
    let read = tokio::time::timeout(FIRST_BYTE_TIMEOUT, stream.peek(&mut first))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connection sent nothing"))??;
    Ok(read == 1 && first[0] == TLS_HANDSHAKE_RECORD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_peers_may_skip_tls() {
        for local in ["127.0.0.1:9", "[::1]:9", "[::ffff:127.0.0.1]:9"] {
            assert!(is_local_peer(Some(local.parse().unwrap())), "{local}");
        }
        for remote in [
            "192.168.1.20:9",
            "100.64.0.7:9",
            "[::ffff:192.168.1.20]:9",
            "[fe80::1]:9",
        ] {
            assert!(!is_local_peer(Some(remote.parse().unwrap())), "{remote}");
        }
        assert!(!is_local_peer(None));
    }

    #[tokio::test]
    async fn one_port_serves_plain_http_and_tls_to_this_computer() {
        crate::util::tls::install_crypto_provider().unwrap();
        let root = std::env::temp_dir().join(format!("bobe-listener-{}", uuid::Uuid::new_v4()));
        let paths = crate::util::paths::DataPaths::resolve(
            &root,
            &format!("sqlite:{}/data.db", root.display()),
        )
        .unwrap()
        .prepare_root()
        .unwrap();
        let identity = crate::util::automatic_tls::ensure(&paths, &["127.0.0.1".into()])
            .await
            .unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            &identity.certificate_path,
            &identity.private_key_path,
        )
        .await
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let app = axum::Router::new().route("/health", axum::routing::get(async || "ok"));
        let server = axum_server::from_tcp(listener)
            .unwrap()
            .acceptor(LocalPlainAcceptor::new(RustlsAcceptor::new(tls)))
            .handle(handle.clone())
            .serve(app.into_make_service());
        let serving = tokio::spawn(server);

        let plain = reqwest::get(format!("http://{address}/health"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(plain, "ok");

        let certificate = tokio::fs::read(&identity.certificate_path).await.unwrap();
        let client = reqwest::Client::builder()
            .tls_certs_merge([reqwest::Certificate::from_pem(&certificate).unwrap()])
            .build()
            .unwrap();
        let secure = client
            .get(format!("https://127.0.0.1:{}/health", address.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(secure, "ok");

        handle.shutdown();
        serving.await.unwrap().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
