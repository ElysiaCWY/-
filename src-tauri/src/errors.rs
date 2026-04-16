use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
  #[error("{0}")]
  Message(String),

  #[error(transparent)]
  Io(#[from] std::io::Error),

  #[error(transparent)]
  Json(#[from] serde_json::Error),
}

impl AppError {
  pub fn msg<T: Into<String>>(s: T) -> Self {
    AppError::Message(s.into())
  }
}

impl serde::Serialize for AppError {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: serde::Serializer,
  {
    serializer.serialize_str(&self.to_string())
  }
}

