use std::io::IsTerminal;
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;

pub(crate) struct HiddenInput {
    original: libc::termios,
}

impl HiddenInput {
    #[expect(unsafe_code, reason = "termios has no std wrapper")]
    pub(crate) fn start() -> std::io::Result<Option<Self>> {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return Ok(None);
        }
        let fd = stdin.as_raw_fd();
        let mut attributes = MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, attributes.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let original = unsafe { attributes.assume_init() };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw const hidden) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Some(Self { original }))
    }
}

impl Drop for HiddenInput {
    #[expect(unsafe_code, reason = "termios has no std wrapper")]
    fn drop(&mut self) {
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const self.original) }
            != 0
        {
            tracing::error!(error = %std::io::Error::last_os_error(), "terminal.echo_restore_failed");
        }
    }
}
