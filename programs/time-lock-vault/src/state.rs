use pinocchio::{error::ProgramError, AccountView, Address};

pub const LOCK_SEED: &[u8] = b"lock";
pub const VAULT_SEED: &[u8] = b"vault";

const DISCRIMINATOR: u8 = 1;

/// Layout: discriminator(1) | bump(1) | vault_bump(1) | owner(32) | mint(32) | unlock_at(8, i64 LE)
pub struct Lock {
    pub bump: u8,
    pub vault_bump: u8,
    pub owner: Address,
    pub mint: Address,
    pub unlock_at: i64,
}

impl Lock {
    pub const LEN: usize = 1 + 1 + 1 + 32 + 32 + 8;

    pub fn write(&self, data: &mut [u8]) -> Result<(), ProgramError> {
        if data.len() != Self::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        data[0] = DISCRIMINATOR;
        data[1] = self.bump;
        data[2] = self.vault_bump;
        data[3..35].copy_from_slice(self.owner.as_array());
        data[35..67].copy_from_slice(self.mint.as_array());
        data[67..75].copy_from_slice(&self.unlock_at.to_le_bytes());
        Ok(())
    }

    /// Loads a lock record, requiring the account to be owned by this program.
    pub fn load(account: &AccountView, program_id: &Address) -> Result<Self, ProgramError> {
        if !account.owned_by(program_id) {
            return Err(ProgramError::InvalidAccountOwner);
        }
        let data = account.try_borrow()?;
        if data.len() != Self::LEN || data[0] != DISCRIMINATOR {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut owner = [0u8; 32];
        let mut mint = [0u8; 32];
        let mut unlock_at = [0u8; 8];
        owner.copy_from_slice(&data[3..35]);
        mint.copy_from_slice(&data[35..67]);
        unlock_at.copy_from_slice(&data[67..75]);
        Ok(Self {
            bump: data[1],
            vault_bump: data[2],
            owner: Address::new_from_array(owner),
            mint: Address::new_from_array(mint),
            unlock_at: i64::from_le_bytes(unlock_at),
        })
    }
}
