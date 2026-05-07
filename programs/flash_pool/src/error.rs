use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("The provided oracle account is invalid")]
    InvalidOracleAccount,
    #[msg("The oracle price is stale")]
    StaleOracle,
    #[msg("Mathematical overflow")]
    Overflow,
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Prediction is outside the valid range")]
    InvalidPrediction,
}
