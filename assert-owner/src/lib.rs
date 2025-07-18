use std::convert::TryFrom;

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg,
    program_error::ProgramError, pubkey::Pubkey,
};

entrypoint!(entry);
fn entry(_program_id: &Pubkey, accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
    let account = accounts.get(0).ok_or(ProgramError::NotEnoughAccountKeys)?;
    if instruction_data.len() != 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let expected_owner = Pubkey::try_from(instruction_data)
    .map_err(|_| ProgramError::InvalidInstructionData)?;
    if expected_owner != *account.owner {
        msg!("Account owner mismatch");
        return Err(ProgramError::Custom(0x100));
    }
    Ok(())
}
