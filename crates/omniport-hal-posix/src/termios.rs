#[cfg(unix)]
use crate::error::HalError;
use crate::error::Result;
use omniport_core::SerialConfig;

#[cfg(unix)]
use std::os::unix::io::RawFd;

#[cfg(unix)]
pub fn speed_to_baud(speed: libc::speed_t) -> Result<u32> {
    let baud = match speed {
        libc::B50 => 50,
        libc::B75 => 75,
        libc::B110 => 110,
        libc::B134 => 134,
        libc::B150 => 150,
        libc::B200 => 200,
        libc::B300 => 300,
        libc::B600 => 600,
        libc::B1200 => 1200,
        libc::B1800 => 1800,
        libc::B2400 => 2400,
        libc::B4800 => 4800,
        libc::B9600 => 9600,
        libc::B19200 => 19200,
        libc::B38400 => 38400,
        libc::B57600 => 57600,
        libc::B115200 => 115_200,
        libc::B230400 => 230_400,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B460800 => 460_800,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B500000 => 500_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B576000 => 576_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B921600 => 921_600,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1000000 => 1_000_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1152000 => 1_152_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B1500000 => 1_500_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B2000000 => 2_000_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B2500000 => 2_500_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B3000000 => 3_000_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B3500000 => 3_500_000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::B4000000 => 4_000_000,
        other => return Err(HalError::UnsupportedBaud(other as u32)),
    };
    Ok(baud)
}

#[cfg(unix)]
pub fn baud_to_speed(baud: u32) -> Result<libc::speed_t> {
    let speed = match baud {
        50 => libc::B50,
        75 => libc::B75,
        110 => libc::B110,
        134 => libc::B134,
        150 => libc::B150,
        200 => libc::B200,
        300 => libc::B300,
        600 => libc::B600,
        1200 => libc::B1200,
        1800 => libc::B1800,
        2400 => libc::B2400,
        4800 => libc::B4800,
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115_200 => libc::B115200,
        230_400 => libc::B230400,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        460_800 => libc::B460800,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        500_000 => libc::B500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        576_000 => libc::B576000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        921_600 => libc::B921600,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1_000_000 => libc::B1000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1_152_000 => libc::B1152000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        1_500_000 => libc::B1500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        2_000_000 => libc::B2000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        2_500_000 => libc::B2500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        3_000_000 => libc::B3000000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        3_500_000 => libc::B3500000,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        4_000_000 => libc::B4000000,
        other => return Err(HalError::UnsupportedBaud(other)),
    };
    Ok(speed)
}

#[cfg(unix)]
pub fn read_termios_config(fd: RawFd) -> Result<SerialConfig> {
    let mut tio = std::mem::MaybeUninit::<libc::termios>::uninit();
    let res = unsafe { libc::tcgetattr(fd, tio.as_mut_ptr()) };
    if res != 0 {
        return Err(HalError::Termios(std::io::Error::last_os_error()));
    }
    let tio = unsafe { tio.assume_init() };

    let speed = unsafe { libc::cfgetospeed(&tio) };
    let baud_rate = speed_to_baud(speed).unwrap_or(115_200);

    let data_bits = match tio.c_cflag & libc::CSIZE {
        libc::CS5 => 5,
        libc::CS6 => 6,
        libc::CS7 => 7,
        _ => 8,
    };

    let stop_bits = if (tio.c_cflag & libc::CSTOPB) != 0 {
        2
    } else {
        1
    };

    let parity = if (tio.c_cflag & libc::PARENB) != 0 {
        if (tio.c_cflag & libc::PARODD) != 0 {
            1 // Odd
        } else {
            2 // Even
        }
    } else {
        0 // None
    };

    Ok(SerialConfig {
        baud_rate,
        data_bits,
        stop_bits,
        parity,
    })
}

#[cfg(unix)]
pub fn set_raw_mode(fd: RawFd) -> Result<()> {
    let mut tio = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, tio.as_mut_ptr()) } != 0 {
        return Err(HalError::Termios(std::io::Error::last_os_error()));
    }
    let mut tio = unsafe { tio.assume_init() };

    unsafe { libc::cfmakeraw(&mut tio) };

    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &tio) } != 0 {
        return Err(HalError::Termios(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Periodically polls terminal configuration on slave PTY and reports updates
#[derive(Debug)]
pub struct TermiosPoller {
    #[cfg(unix)]
    slave_fd: Option<RawFd>,
    #[cfg(not(unix))]
    mock_config: SerialConfig,
    current_config: SerialConfig,
}

impl TermiosPoller {
    #[cfg(unix)]
    pub fn new(slave_fd: Option<RawFd>, initial: SerialConfig) -> Self {
        Self {
            slave_fd,
            current_config: initial,
        }
    }

    #[cfg(not(unix))]
    pub fn new(_slave_fd: Option<i32>, initial: SerialConfig) -> Self {
        Self {
            mock_config: initial,
            current_config: initial,
        }
    }

    pub fn current_config(&self) -> &SerialConfig {
        &self.current_config
    }

    #[cfg(not(unix))]
    pub fn set_mock_config(&mut self, config: SerialConfig) {
        self.mock_config = config;
    }

    /// Polls the slave PTY. Returns `Some(new_config)` if serial configuration changed.
    pub fn poll(&mut self) -> Result<Option<SerialConfig>> {
        #[cfg(unix)]
        {
            if let Some(fd) = self.slave_fd {
                let latest = read_termios_config(fd)?;
                if latest.baud_rate != self.current_config.baud_rate
                    || latest.data_bits != self.current_config.data_bits
                    || latest.stop_bits != self.current_config.stop_bits
                    || latest.parity != self.current_config.parity
                {
                    self.current_config = latest;
                    return Ok(Some(latest));
                }
            }
            Ok(None)
        }

        #[cfg(not(unix))]
        {
            if self.mock_config.baud_rate != self.current_config.baud_rate
                || self.mock_config.data_bits != self.current_config.data_bits
                || self.mock_config.stop_bits != self.current_config.stop_bits
                || self.mock_config.parity != self.current_config.parity
            {
                self.current_config = self.mock_config;
                return Ok(Some(self.mock_config));
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termios_poller_mock() {
        let init = SerialConfig {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        let mut poller = TermiosPoller::new(None, init);
        assert_eq!(poller.current_config().baud_rate, 9600);

        // No change initially
        assert_eq!(poller.poll().unwrap(), None);

        // Simulate client reconfiguration (e.g. minicom setting 115200)
        let updated = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        #[cfg(not(unix))]
        poller.set_mock_config(updated);

        #[cfg(not(unix))]
        {
            let res = poller.poll().unwrap();
            assert!(res.is_some());
            assert_eq!(res.unwrap().baud_rate, 115_200);
            assert_eq!(poller.current_config().baud_rate, 115_200);

            // Consecutive poll without changes returns None
            assert_eq!(poller.poll().unwrap(), None);
        }
    }
}
