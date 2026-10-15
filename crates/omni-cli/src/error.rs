//! The one error type of the CLI: a single line that says what went wrong and
//! how to fix it. `main` prints it on standard error and exits with code 1.

use std::fmt;

#[derive(Debug)]
pub struct CliError(String);

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        // An error is one line, whatever the text it was built from.
        let message: String = message.into();
        Self(message.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
