use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::Request;
use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsAcceptor;
use sha2::{Digest, Sha256};
use tokio_rustls::server::TlsStream;
use tower::Service;

use super::identity::{BodyIdentityIndex, BodyPeerIdentity};

#[derive(Clone)]
pub(crate) struct BodyIdentityAcceptor {
    inner: RustlsAcceptor,
    identities: Arc<BodyIdentityIndex>,
}

impl BodyIdentityAcceptor {
    pub(crate) fn new(inner: RustlsAcceptor, identities: Arc<BodyIdentityIndex>) -> Self {
        Self { inner, identities }
    }
}

#[derive(Clone)]
pub(crate) struct BodyIdentityService<S> {
    inner: S,
    identity: BodyPeerIdentity,
}

impl<S, B> Service<Request<B>> for BodyIdentityService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        request.extensions_mut().insert(self.identity.clone());
        self.inner.call(request)
    }
}

impl<I, S> Accept<I, S> for BodyIdentityAcceptor
where
    RustlsAcceptor: Accept<I, S, Stream = TlsStream<I>, Service = S>,
    <RustlsAcceptor as Accept<I, S>>::Future: Send + 'static,
    I: Send + 'static,
    S: Send + 'static,
{
    type Stream = TlsStream<I>;
    type Service = BodyIdentityService<S>;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let future = self.inner.accept(stream, service);
        let identities = Arc::clone(&self.identities);
        Box::pin(async move {
            let (stream, service) = future.await?;
            let certificate = stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certificates| certificates.first())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "missing client certificate",
                    )
                })?;
            let digest: [u8; 32] = Sha256::digest(certificate.as_ref()).into();
            let identity = identities.by_certificate(&digest).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "client certificate is not enrolled",
                )
            })?;
            Ok((
                stream,
                BodyIdentityService {
                    inner: service,
                    identity,
                },
            ))
        })
    }
}
