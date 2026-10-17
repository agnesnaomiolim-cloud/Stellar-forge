//! FlashLoanReceiver callback interface and invocation helpers.

use soroban_sdk::{Address, Bytes, Env, IntoVal, Symbol};

use crate::Error;

/// Standard callback interface that flash-loan borrower contracts must implement.
pub trait FlashLoanReceiver {
    /// Invoked by `FlashLoanPool` immediately after transferring `amount` of `token`
    /// to the receiver contract. The implementation must return at least
    /// `amount + fee` of `token` back to the pool before returning.
    fn execute_operation(env: Env, token: Address, amount: i128, fee: i128, data: Bytes);
}

/// Dispatches the `execute_operation` callback on a target receiver contract.
pub fn invoke_flash_loan_receiver(
    env: &Env,
    receiver: &Address,
    token: &Address,
    amount: i128,
    fee: i128,
    user_data: &Bytes,
) {
    let args = soroban_sdk::vec![
        env,
        token.clone().into_val(env),
        amount.into_val(env),
        fee.into_val(env),
        user_data.clone().into_val(env),
    ];
    env.invoke_contract::<()>(receiver, &Symbol::new(env, "execute_operation"), args);
}

/// Verifies the post-callback pool balance invariance (`balance_after >= balance_before + fee`).
pub fn verify_repayment_invariant(
    balance_before: i128,
    balance_after: i128,
    required_fee: i128,
) -> Result<i128, Error> {
    let repayment_floor = balance_before
        .checked_add(required_fee)
        .ok_or(Error::ArithmeticOverflow)?;
    if balance_after < repayment_floor {
        return Err(Error::RepaymentDeficit);
    }
    balance_after
        .checked_sub(balance_before)
        .ok_or(Error::ArithmeticOverflow)
}
