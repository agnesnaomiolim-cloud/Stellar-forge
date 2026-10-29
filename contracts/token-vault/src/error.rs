use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,

    InsufficientBalance = 3,
    InsufficientAllowance = 4,
    ZeroAmount = 5,
    Paused = 6,
    Unauthorized = 7,
    Overflow = 8,
    InvalidExpiration = 9,
    MaxSupplyExceeded = 10,
}
