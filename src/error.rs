use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum BotError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Matrix SDK error: {0}")]
    Matrix(#[from] matrix_sdk::Error),

    #[error("Invalid command format: {0}")]
    InvalidCommand(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Module error: {0}")]
    Module(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[allow(dead_code)]
pub type BotResult<T> = Result<T, BotError>;
