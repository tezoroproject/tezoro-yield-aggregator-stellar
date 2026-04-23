use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    NotInitialized = 100,
    AlreadyInitialized = 101,
    Unauthorized = 102,
    Paused = 103,
    ZeroAmount = 104,
    InsufficientShares = 105,
    InsufficientAssets = 106,
    DepositCapExceeded = 107,
    MaxStrategies = 108,
    StrategyAlreadyActive = 109,
    StrategyNotActive = 110,
    InvalidBps = 111,
    StrategyHasBalance = 112,
    NoPendingAdmin = 113,
    StaleAttestation = 114,
    InvalidBalance = 115,
    UpgradeNotScheduled = 116,
    UpgradeTooEarly = 117,
    IdleBufferViolation = 118,
    NoFeesToCollect = 119,
    StrategyUnhealthy = 120,
}
