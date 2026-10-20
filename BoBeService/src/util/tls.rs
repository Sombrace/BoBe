pub(crate) fn install_crypto_provider() {
    drop(rustls::crypto::ring::default_provider().install_default());
}
