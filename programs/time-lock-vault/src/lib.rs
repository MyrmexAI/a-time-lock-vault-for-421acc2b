//! Time-lock vault for SPL Token (classic Token program) mints.
//!
//! One lock per (owner, mint). `lock` moves tokens into a program-controlled
//! vault token account; `withdraw` returns the full balance to the owner once
//! the unlock time has passed and closes both accounts.

use pinocchio::{entrypoint, error::ProgramError, AccountView, Address, ProgramResult};

pub mod error;
mod lock;
pub mod state;
mod withdraw;

entrypoint!(process_instruction);

pub const LOCK: u8 = 0;
pub const WITHDRAW: u8 = 1;

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    match instruction_data.split_first() {
        Some((&LOCK, data)) => lock::process(program_id, accounts, data),
        Some((&WITHDRAW, data)) => withdraw::process(program_id, accounts, data),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
