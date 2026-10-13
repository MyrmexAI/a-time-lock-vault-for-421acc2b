use {
    crate::{
        error::VaultError,
        state::{Lock, LOCK_SEED, VAULT_SEED},
    },
    pinocchio::{
        cpi::{Seed, Signer},
        error::ProgramError,
        sysvars::{clock::Clock, Sysvar},
        AccountView, Address, ProgramResult,
    },
    pinocchio_token::{
        instructions::{CloseAccount, TransferChecked},
        state::{Account as TokenAccount, Mint},
    },
};

/// Accounts:
///   0. `[writable, signer]` owner (receives the rent back)
///   1. `[writable]` owner token account (destination)
///   2. `[]` mint
///   3. `[writable]` lock state PDA
///   4. `[writable]` vault token account PDA
///   5. `[]` token program
///
/// Data: none
pub fn process(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [owner, owner_token, mint, lock, vault, token_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if !data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    if !owner.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !owner.is_writable() || !owner_token.is_writable() || !lock.is_writable() || !vault.is_writable()
    {
        return Err(ProgramError::InvalidAccountData);
    }
    if token_program.address() != &pinocchio_token::ID {
        return Err(ProgramError::IncorrectProgramId);
    }

    let state = Lock::load(lock, program_id)?;
    if &state.owner != owner.address() {
        return Err(VaultError::NotLockOwner.into());
    }
    if &state.mint != mint.address() {
        return Err(VaultError::InvalidAccount.into());
    }

    let lock_key = Address::create_program_address(
        &[
            LOCK_SEED,
            state.owner.as_array(),
            state.mint.as_array(),
            &[state.bump],
        ],
        program_id,
    )
    .map_err(|_| ProgramError::InvalidSeeds)?;
    if lock.address() != &lock_key {
        return Err(ProgramError::InvalidSeeds);
    }
    let vault_key = Address::create_program_address(
        &[VAULT_SEED, lock.address().as_array(), &[state.vault_bump]],
        program_id,
    )
    .map_err(|_| ProgramError::InvalidSeeds)?;
    if vault.address() != &vault_key {
        return Err(ProgramError::InvalidSeeds);
    }

    if Clock::get()?.unix_timestamp < state.unlock_at {
        return Err(VaultError::StillLocked.into());
    }

    let decimals = Mint::from_account_view(mint)?.decimals();
    {
        let destination = TokenAccount::from_account_view(owner_token)?;
        if destination.mint() != mint.address() || destination.owner() != owner.address() {
            return Err(VaultError::InvalidAccount.into());
        }
    }
    let amount = TokenAccount::from_account_view(vault)?.amount();

    let lock_bump_seed = [state.bump];
    let lock_seeds = [
        Seed::from(LOCK_SEED),
        Seed::from(state.owner.as_array()),
        Seed::from(state.mint.as_array()),
        Seed::from(&lock_bump_seed),
    ];
    let signer = Signer::from(&lock_seeds);

    if amount > 0 {
        TransferChecked::new(vault, mint, owner_token, lock, amount, decimals)
            .invoke_signed(&[signer.clone()])?;
    }
    CloseAccount::new(vault, owner, lock).invoke_signed(&[signer])?;

    let reclaimed = lock.lamports();
    let new_owner_lamports = owner
        .lamports()
        .checked_add(reclaimed)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    owner.set_lamports(new_owner_lamports);
    lock.set_lamports(0);
    lock.close()
}
