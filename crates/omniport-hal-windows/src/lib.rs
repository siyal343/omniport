//! Windows Overlapped UART and com0com / UMDF2 virtual bus controller.

pub mod com0com;
pub mod error;
pub mod uart;

pub use com0com::{Com0ComController, VirtualPair, VirtualPairGuard};
pub use error::{HalError, Result};
pub use uart::{Win32SerialEndpoint, format_com_path};
