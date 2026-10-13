use pinocchio::error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultError {
    ZeroAmount = 0,
    UnlockTimeNotInFuture = 1,
    StillLocked = 2,
    NotLockOwner = 3,
    InvalidAccount = 4,
}

impl From<VaultError> for ProgramError {
    fn from(e: VaultError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
