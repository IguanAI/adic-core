use thiserror::Error;

#[derive(Error, Debug)]
pub enum MathError {
    #[error("Prime mismatch: expected {expected}, got {actual}")]
    PrimeMismatch { expected: u32, actual: u32 },
    
    #[error("Invalid digit: {0} exceeds prime {1}")]
    InvalidDigit(u8, u32),
    
    #[error("Precision mismatch: expected {expected}, got {actual}")]
    PrecisionMismatch { expected: usize, actual: usize },
    
    #[error("Overflow in calculation")]
    Overflow,
    
    #[error("Division by zero")]
    DivisionByZero,
    
    #[error("Invalid valuation")]
    InvalidValuation,
}

pub type Result<T> = std::result::Result<T, MathError>;