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
    #[msg("The oracle account is not owned by the Pyth Receiver program")]
    InvalidOracleOwner,
    #[msg("The oracle feed_id does not match the expected SOL/USD feed")]
    MismatchedFeedId,
    #[msg("The oracle account data is malformed or too short")]
    InvalidOracleData,
}
