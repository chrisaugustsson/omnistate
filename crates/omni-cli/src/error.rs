//! The one error type of the CLI: a single line that says what went wrong and
//! how to fix it. `main` prints it on standard error and exits with code 1,
//! unless the error carries its own code (`omni wait` ends with 2 or 3).

use std::fmt;

#[derive(Debug)]
pub struct CliError {
    message: String,
    code: u8,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_code(1, message)
    }

    /// An error that ends the command with exit code `code`.
    pub fn with_code(code: u8, message: impl Into<String>) -> Self {
        // An error is one line, whatever the text it was built from.
        let message: String = message.into();
        Self {
            message: message.split_whitespace().collect::<Vec<_>>().join(" "),
            code,
        }
    }

    pub fn code(&self) -> u8 {
        self.code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub type Result<T> = std::result::Result<T, CliError>;

/// Returns early with an error built from a format string.
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::CliError::new(format!($($arg)*)))
    };
}
pub(crate) use bail;
