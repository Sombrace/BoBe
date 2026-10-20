#[expect(unsafe_code, reason = "signalling a process group has no std wrapper")]
pub(crate) fn signal_group(pid: u32, signal: i32) {
    if let Ok(pid) = i32::try_from(pid) {
        let result = unsafe { libc::kill(-pid, signal) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                tracing::warn!(%error, pid, "process_group.signal_failed");
            }
        }
    }
}

#[expect(unsafe_code, reason = "probing a process group has no std wrapper")]
pub(crate) fn group_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    let result = unsafe { libc::kill(-pid, 0) };
    result == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}
