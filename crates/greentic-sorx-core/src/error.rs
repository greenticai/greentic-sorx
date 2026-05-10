use std::fmt;

pub type SorxResult<T> = Result<T, SorxError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorxError {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}

impl SorxError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: None,
        }
    }

    pub fn at_path(
        code: impl Into<String>,
        message: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: Some(path.into()),
        }
    }
}

impl fmt::Display for SorxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{} at {path}", self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for SorxError {}
