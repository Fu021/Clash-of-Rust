use anyhow::{Context, Result};

pub struct Mode {
    original: libc::termios,
}

impl Mode {
    pub fn enter() -> Result<Self> {
        let mut original = std::mem::MaybeUninit::uninit();
        anyhow::ensure!(
            unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } == 0,
            "无法读取终端属性：{}",
            std::io::Error::last_os_error()
        );
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        anyhow::ensure!(
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } == 0,
            "无法设置终端属性：{}",
            std::io::Error::last_os_error()
        );
        Ok(Self { original })
    }

    pub fn size(&self) -> Result<(usize, usize)> {
        let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("无法读取终端尺寸");
        }
        let size = unsafe { size.assume_init() };
        Ok((usize::from(size.ws_col), usize::from(size.ws_row)))
    }

    pub fn read(&mut self) -> Result<Vec<u8>> {
        let mut descriptor = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                return Ok(Vec::new());
            }
            return Err(error.into());
        }
        anyhow::ensure!(
            descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) == 0,
            "终端已断开"
        );
        if ready == 0 {
            return Ok(Vec::new());
        }
        let mut bytes = vec![0; 1024];
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        bytes.truncate(count as usize);
        Ok(bytes)
    }
}

impl Drop for Mode {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
    }
}
