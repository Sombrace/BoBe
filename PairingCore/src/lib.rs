use opaque_ke::argon2::Argon2;
use opaque_ke::ciphersuite::CipherSuite;
use opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, ClientRegistration,
    ClientRegistrationFinishParameters, CredentialFinalization, CredentialRequest,
    CredentialResponse, Identifiers, ServerLogin, ServerLoginParameters, ServerRegistration,
    ServerSetup,
};
use rand::rngs::OsRng;
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const CLIENT_ID: &[u8] = b"bobe-pairing-client-v1";
const SERVER_ID: &[u8] = b"bobe-pairing-authority-v1";

struct PairingSuite;

impl CipherSuite for PairingSuite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, Sha512>;
    type Ksf = Argon2<'static>;
}

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("invalid pairing message")]
    InvalidMessage,
    #[error("pairing authentication failed")]
    AuthenticationFailed,
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Invitation {
    server_setup: Vec<u8>,
    password_file: Vec<u8>,
}

impl Invitation {
    pub fn create(code: &[u8]) -> Result<Self, PairingError> {
        validate_code(code)?;
        let mut rng = OsRng;
        let server_setup = ServerSetup::<PairingSuite>::new(&mut rng);
        let client_start = ClientRegistration::<PairingSuite>::start(&mut rng, code)
            .map_err(|_| PairingError::InvalidMessage)?;
        let server_start = ServerRegistration::<PairingSuite>::start(
            &server_setup,
            client_start.message,
            CLIENT_ID,
        )
        .map_err(|_| PairingError::InvalidMessage)?;
        let client_finish = client_start
            .state
            .finish(
                &mut rng,
                code,
                server_start.message,
                ClientRegistrationFinishParameters::new(identifiers(), None),
            )
            .map_err(|_| PairingError::InvalidMessage)?;
        let password_file = ServerRegistration::finish(client_finish.message);

        Ok(Self {
            server_setup: server_setup.serialize().to_vec(),
            password_file: password_file.serialize().to_vec(),
        })
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ClientStart {
    state: Vec<u8>,
    request: Vec<u8>,
}

impl ClientStart {
    pub fn begin(code: &[u8]) -> Result<Self, PairingError> {
        validate_code(code)?;
        let result = ClientLogin::<PairingSuite>::start(&mut OsRng, code)
            .map_err(|_| PairingError::InvalidMessage)?;
        Ok(Self {
            state: result.state.serialize().to_vec(),
            request: result.message.serialize().to_vec(),
        })
    }

    pub fn request(&self) -> &[u8] {
        &self.request
    }

    pub fn finish(
        self,
        code: &[u8],
        response: &[u8],
        context: &[u8],
    ) -> Result<ClientFinish, PairingError> {
        let state = ClientLogin::<PairingSuite>::deserialize(&self.state)
            .map_err(|_| PairingError::InvalidMessage)?;
        let response = CredentialResponse::<PairingSuite>::deserialize(response)
            .map_err(|_| PairingError::InvalidMessage)?;
        let result = state
            .finish(
                &mut OsRng,
                code,
                response,
                ClientLoginFinishParameters::new(Some(context), identifiers(), None),
            )
            .map_err(|_| PairingError::AuthenticationFailed)?;
        Ok(ClientFinish {
            finalization: result.message.serialize().to_vec(),
            session_key: result.session_key.to_vec(),
        })
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ClientFinish {
    finalization: Vec<u8>,
    session_key: Vec<u8>,
}

impl ClientFinish {
    pub fn finalization(&self) -> &[u8] {
        &self.finalization
    }

    pub fn session_key(&self) -> &[u8] {
        &self.session_key
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ServerStart {
    state: Vec<u8>,
    response: Vec<u8>,
}

impl ServerStart {
    pub fn begin(
        invitation: &Invitation,
        request: &[u8],
        context: &[u8],
    ) -> Result<Self, PairingError> {
        let server_setup = ServerSetup::<PairingSuite>::deserialize(&invitation.server_setup)
            .map_err(|_| PairingError::InvalidMessage)?;
        let password_file =
            ServerRegistration::<PairingSuite>::deserialize(&invitation.password_file)
                .map_err(|_| PairingError::InvalidMessage)?;
        let request = CredentialRequest::<PairingSuite>::deserialize(request)
            .map_err(|_| PairingError::InvalidMessage)?;
        let result = ServerLogin::start(
            &mut OsRng,
            &server_setup,
            Some(password_file),
            request,
            CLIENT_ID,
            ServerLoginParameters {
                context: Some(context),
                identifiers: identifiers(),
            },
        )
        .map_err(|_| PairingError::AuthenticationFailed)?;
        Ok(Self {
            state: result.state.serialize().to_vec(),
            response: result.message.serialize().to_vec(),
        })
    }

    pub fn response(&self) -> &[u8] {
        &self.response
    }

    pub fn finish(
        self,
        finalization: &[u8],
        context: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, PairingError> {
        let state = ServerLogin::<PairingSuite>::deserialize(&self.state)
            .map_err(|_| PairingError::InvalidMessage)?;
        let finalization = CredentialFinalization::<PairingSuite>::deserialize(finalization)
            .map_err(|_| PairingError::InvalidMessage)?;
        let result = state
            .finish(
                finalization,
                ServerLoginParameters {
                    context: Some(context),
                    identifiers: identifiers(),
                },
            )
            .map_err(|_| PairingError::AuthenticationFailed)?;
        Ok(Zeroizing::new(result.session_key.to_vec()))
    }
}

fn validate_code(code: &[u8]) -> Result<(), PairingError> {
    (code.len() == 6 && code.iter().all(u8::is_ascii_digit)).ok_or(PairingError::InvalidMessage)
}

fn identifiers() -> Identifiers<'static> {
    Identifiers {
        client: Some(CLIENT_ID),
        server: Some(SERVER_ID),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(code: &[u8], client_context: &[u8], server_context: &[u8]) -> bool {
        let invitation = Invitation::create(code).unwrap();
        let client = ClientStart::begin(code).unwrap();
        let server = ServerStart::begin(&invitation, client.request(), server_context).unwrap();
        let client = client
            .finish(code, server.response(), client_context)
            .unwrap();
        let server_key = server
            .finish(client.finalization(), server_context)
            .unwrap();
        client.session_key() == server_key.as_slice()
    }

    #[test]
    fn completes_with_leading_zero_code_and_bound_context() {
        assert!(exchange(
            b"012345",
            b"authority|epoch|origin|spki",
            b"authority|epoch|origin|spki"
        ));
    }

    #[test]
    fn wrong_code_fails_before_enrollment() {
        let invitation = Invitation::create(b"483921").unwrap();
        let client = ClientStart::begin(b"123456").unwrap();
        let server = ServerStart::begin(&invitation, client.request(), b"context").unwrap();
        assert!(
            client
                .finish(b"123456", server.response(), b"context")
                .is_err()
        );
    }

    #[test]
    fn mismatched_context_fails_confirmation() {
        let invitation = Invitation::create(b"483921").unwrap();
        let client = ClientStart::begin(b"483921").unwrap();
        let server = ServerStart::begin(&invitation, client.request(), b"server-context").unwrap();
        assert!(
            client
                .finish(b"483921", server.response(), b"client-context")
                .is_err()
        );
    }
}
