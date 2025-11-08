use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V8Error {
    message: String,
}

impl V8Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for V8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for V8Error {}

impl From<&str> for V8Error {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for V8Error {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

pub type Result<T> = std::result::Result<T, V8Error>;
