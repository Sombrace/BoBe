use bobe_pairing_core::{ClientStart, PairingError};
use sha2::{Digest, Sha256};
use std::ptr;
use std::slice;
use x509_parser::prelude::FromDer;
use zeroize::Zeroize;

#[repr(C)]
#[derive(Clone, Copy)]
pub enum BobePairingStatus {
    Ok = 0,
    InvalidArgument = 1,
    InvalidMessage = 2,
    AuthenticationFailed = 3,
    InternalError = 4,
}

#[repr(C)]
pub struct BobePairingBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl BobePairingBuffer {
    const EMPTY: Self = Self {
        data: ptr::null_mut(),
        len: 0,
    };
}

pub struct BobePairingClient {
    state: Option<ClientStart>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bobe_pairing_client_start(
    code: *const u8,
    code_len: usize,
    client_out: *mut *mut BobePairingClient,
    request_out: *mut BobePairingBuffer,
) -> BobePairingStatus {
    if client_out.is_null() || request_out.is_null() {
        return BobePairingStatus::InvalidArgument;
    }
    unsafe {
        *client_out = ptr::null_mut();
        *request_out = BobePairingBuffer::EMPTY;
    }
    let Some(code) = (unsafe { input_slice(code, code_len) }) else {
        return BobePairingStatus::InvalidArgument;
    };
    let result = std::panic::catch_unwind(|| ClientStart::begin(code));
    let client = match result {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => return map_error(&error),
        Err(_) => return BobePairingStatus::InternalError,
    };
    let request = client.request().to_vec();
    unsafe {
        *request_out = into_buffer(request);
        *client_out = Box::into_raw(Box::new(BobePairingClient {
            state: Some(client),
        }));
    }
    BobePairingStatus::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bobe_pairing_client_finish(
    client: *mut BobePairingClient,
    code: *const u8,
    code_len: usize,
    response: *const u8,
    response_len: usize,
    context: *const u8,
    context_len: usize,
    finalization_out: *mut BobePairingBuffer,
    session_key_out: *mut BobePairingBuffer,
) -> BobePairingStatus {
    if client.is_null() || finalization_out.is_null() || session_key_out.is_null() {
        return BobePairingStatus::InvalidArgument;
    }
    unsafe {
        *finalization_out = BobePairingBuffer::EMPTY;
        *session_key_out = BobePairingBuffer::EMPTY;
    }
    let Some(code) = (unsafe { input_slice(code, code_len) }) else {
        return BobePairingStatus::InvalidArgument;
    };
    let Some(response) = (unsafe { input_slice(response, response_len) }) else {
        return BobePairingStatus::InvalidArgument;
    };
    let Some(context) = (unsafe { input_slice(context, context_len) }) else {
        return BobePairingStatus::InvalidArgument;
    };
    let state = unsafe { &mut *client }.state.take();
    let Some(state) = state else {
        return BobePairingStatus::InvalidArgument;
    };
    let result = std::panic::catch_unwind(|| state.finish(code, response, context));
    let finish = match result {
        Ok(Ok(finish)) => finish,
        Ok(Err(error)) => return map_error(&error),
        Err(_) => return BobePairingStatus::InternalError,
    };
    unsafe {
        *finalization_out = into_buffer(finish.finalization().to_vec());
        *session_key_out = into_buffer(finish.session_key().to_vec());
    }
    BobePairingStatus::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bobe_pairing_spki_sha256(
    certificate_der: *const u8,
    certificate_der_len: usize,
    output: *mut u8,
) -> BobePairingStatus {
    if output.is_null() {
        return BobePairingStatus::InvalidArgument;
    }
    let Some(certificate_der) = (unsafe { input_slice(certificate_der, certificate_der_len) })
    else {
        return BobePairingStatus::InvalidArgument;
    };
    let result = std::panic::catch_unwind(|| {
        let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(certificate_der)
            .map_err(|_| BobePairingStatus::InvalidMessage)?;
        Ok::<[u8; 32], BobePairingStatus>(
            Sha256::digest(certificate.tbs_certificate.subject_pki.raw).into(),
        )
    });
    match result {
        Ok(Ok(digest)) => {
            unsafe { ptr::copy_nonoverlapping(digest.as_ptr(), output, digest.len()) };
            BobePairingStatus::Ok
        }
        Ok(Err(status)) => status,
        Err(_) => BobePairingStatus::InternalError,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bobe_pairing_client_destroy(client: *mut BobePairingClient) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bobe_pairing_buffer_destroy(buffer: *mut BobePairingBuffer) {
    if buffer.is_null() {
        return;
    }
    let buffer = unsafe { &mut *buffer };
    if !buffer.data.is_null() {
        let bytes = ptr::slice_from_raw_parts_mut(buffer.data, buffer.len);
        let mut bytes = unsafe { Box::from_raw(bytes) };
        bytes.zeroize();
    }
    *buffer = BobePairingBuffer::EMPTY;
}

unsafe fn input_slice<'a>(data: *const u8, len: usize) -> Option<&'a [u8]> {
    if data.is_null() {
        return (len == 0).then_some(&[]);
    }
    Some(unsafe { slice::from_raw_parts(data, len) })
}

fn into_buffer(bytes: Vec<u8>) -> BobePairingBuffer {
    let len = bytes.len();
    BobePairingBuffer {
        data: Box::into_raw(bytes.into_boxed_slice()).cast::<u8>(),
        len,
    }
}

fn map_error(error: &PairingError) -> BobePairingStatus {
    match error {
        PairingError::InvalidMessage => BobePairingStatus::InvalidMessage,
        PairingError::AuthenticationFailed => BobePairingStatus::AuthenticationFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bobe_pairing_core::{Invitation, ServerStart};

    #[test]
    fn ffi_client_interoperates_with_rust_server_and_clears_buffers() {
        let code = b"012345";
        let context = b"authority|epoch|origin|spki";
        let invitation = Invitation::create(code).unwrap();
        let mut client = ptr::null_mut();
        let mut request = BobePairingBuffer::EMPTY;
        assert_eq!(
            unsafe {
                bobe_pairing_client_start(code.as_ptr(), code.len(), &mut client, &mut request)
                    as u32
            },
            BobePairingStatus::Ok as u32
        );
        let request_bytes = unsafe { slice::from_raw_parts(request.data, request.len) };
        let server = ServerStart::begin(&invitation, request_bytes, context).unwrap();
        let mut finalization = BobePairingBuffer::EMPTY;
        let mut client_key = BobePairingBuffer::EMPTY;
        assert_eq!(
            unsafe {
                bobe_pairing_client_finish(
                    client,
                    code.as_ptr(),
                    code.len(),
                    server.response().as_ptr(),
                    server.response().len(),
                    context.as_ptr(),
                    context.len(),
                    &mut finalization,
                    &mut client_key,
                ) as u32
            },
            BobePairingStatus::Ok as u32
        );
        let finalization_bytes =
            unsafe { slice::from_raw_parts(finalization.data, finalization.len) };
        let server_key = server.finish(finalization_bytes, context).unwrap();
        let client_key_bytes = unsafe { slice::from_raw_parts(client_key.data, client_key.len) };
        assert_eq!(client_key_bytes, server_key.as_slice());

        unsafe {
            bobe_pairing_buffer_destroy(&mut request);
            bobe_pairing_buffer_destroy(&mut finalization);
            bobe_pairing_buffer_destroy(&mut client_key);
            bobe_pairing_client_destroy(client);
        }
        assert!(request.data.is_null());
        assert_eq!(request.len, 0);
        assert!(finalization.data.is_null());
        assert!(client_key.data.is_null());
    }

    #[test]
    fn ffi_rejects_non_six_digit_code() {
        let code = b"12345x";
        let mut client = ptr::null_mut();
        let mut request = BobePairingBuffer::EMPTY;
        assert_eq!(
            unsafe {
                bobe_pairing_client_start(code.as_ptr(), code.len(), &mut client, &mut request)
                    as u32
            },
            BobePairingStatus::InvalidMessage as u32
        );
        assert!(client.is_null());
        assert!(request.data.is_null());
    }
}
