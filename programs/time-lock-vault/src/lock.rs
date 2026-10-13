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
    pinocchio_system::create_account_with_minimum_balance_signed,
    pinocchio_token::{
        instructions::{InitializeAccount3, TransferChecked},
        state::{Account as TokenAccount, Mint},
    },
};

/// Accounts:
///   0. `[writable, signer]` owner (pays rent)
///   1. `[writable]` owner token account (source)
///   2. `[]` mint
///   3. `[writable]` lock state PDA: ["lock", owner, mint]
///   4. `[writable]` vault token account PDA: ["vault", lock state]
///   5. `[]` system program
///   6. `[]` token program
///
/// Data: amount (u64 LE) | unlock_at (i64 LE, unix seconds)
pub fn process(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [owner, owner_token, mint, lock, vault, system_program, token_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if data.len() != 16 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let unlock_at = i64::from_le_bytes(data[8..16].try_into().unwrap());

    if amount == 0 {
        return Err(VaultError::ZeroAmount.into());
    }
    if unlock_at <= Clock::get()?.unix_timestamp {
        return Err(VaultError::UnlockTimeNotInFuture.into());
    }

    if !owner.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !owner.is_writable() || !owner_token.is_writable() || !lock.is_writable() || !vault.is_writable()
    {
        return Err(ProgramError::InvalidAccountData);
    }
    if system_program.address() != &pinocchio_system::ID
        || token_program.address() != &pinocchio_token::ID
    {
        return Err(ProgramError::IncorrectProgramId);
    }

    let decimals = Mint::from_account_view(mint)?.decimals();
    {
        let source = TokenAccount::from_account_view(owner_token)?;
        if source.mint() != mint.address() || source.owner() != owner.address() {
            return Err(VaultError::InvalidAccount.into());
        }
    }

    let (lock_key, lock_bump) = Address::find_program_address(
        &[
            LOCK_SEED,
            owner.address().as_array(),
            mint.address().as_array(),
        ],
        program_id,
    );
    if lock.address() != &lock_key {
        return Err(ProgramError::InvalidSeeds);
    }
    let (vault_key, vault_bump) = Address::find_program_address(
        &[VAULT_SEED, lock.address().as_array()],
        program_id,
    );
    if vault.address() != &vault_key {
        return Err(ProgramError::InvalidSeeds);
    }

    let lock_bump_seed = [lock_bump];
    let lock_seeds = [
        Seed::from(LOCK_SEED),
        Seed::from(owner.address().as_array()),
        Seed::from(mint.address().as_array()),
        Seed::from(&lock_bump_seed),
    ];
    create_account_with_minimum_balance_signed(
        lock,
        Lock::LEN,
        program_id,
        owner,
        None,
        &[Signer::from(&lock_seeds)],
    )?;

    let vault_bump_seed = [vault_bump];
    let vault_seeds = [
        Seed::from(VAULT_SEED),
        Seed::from(lock.address().as_array()),
        Seed::from(&vault_bump_seed),
    ];
    create_account_with_minimum_balance_signed(
        vault,
        TokenAccount::LEN,
        &pinocchio_token::ID,
        owner,
        None,
        &[Signer::from(&vault_seeds)],
    )?;
    InitializeAccount3::new(vault, mint, lock.address()).invoke()?;

    Lock {
        bump: lock_bump,
        vault_bump,
        owner: owner.address().clone(),
        mint: mint.address().clone(),
        unlock_at,
    }
    .write(&mut lock.try_borrow_mut()?)?;

    TransferChecked::new(owner_token, mint, vault, owner, amount, decimals).invoke()
}
