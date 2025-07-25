use crate::{open_orders_authority, open_orders_init_authority};
use anchor_lang::prelude::*;
use solana_program::{
    msg, 
    pubkey::Pubkey, 
    entrypoint::ProgramResult, 
    account_info::AccountInfo,
    program_error::ProgramError,
};
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token;
use serum_dex;
use serum_dex::instruction::*;
use serum_dex::matching::Side;

declare_id!("9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin");

/// Per request context. Can be used to share data between middleware handlers.
pub struct Context<'a, 'info> {
    pub program_id: &'a Pubkey,
    pub dex_program_id: &'a Pubkey,
    pub accounts: Vec<AccountInfo<'info>>,
    pub seeds: Seeds,
    // Instructions to execute *prior* to the DEX relay CPI.
    pub pre_instructions: Vec<(Instruction, Vec<AccountInfo<'info>>, Seeds)>,
    // Instructions to execution *after* the DEX relay CPI.
    pub post_instructions: Vec<(Instruction, Vec<AccountInfo<'info>>, Seeds)>,
    pub post_callbacks: Vec<(PostCallback<'a, 'info>, Vec<AccountInfo<'info>>, Vec<u8>)>,
}

type PostCallback<'a, 'info> = fn(
    // program_id
    &'a Pubkey,
    // AccountInfos needed for post callback.
    Vec<AccountInfo<'info>>,
    // Market instruction.
    Vec<u8>,
    // Arguments to post callback.
    Vec<u8>,
) -> ProgramResult;

type Seeds = Vec<Vec<Vec<u8>>>;

impl<'a, 'info> Context<'a, 'info> {
    pub fn new(
        program_id: &'a Pubkey,
        dex_program_id: &'a Pubkey,
        accounts: Vec<AccountInfo<'info>>,
    ) -> Self {
        Self {
            program_id,
            dex_program_id,
            accounts,
            seeds: Vec::new(),
            pre_instructions: Vec::new(),
            post_instructions: Vec::new(),
            post_callbacks: Vec::new(),
        }
    }
}

/// Implementing this trait allows one to hook into requests to the Serum DEX
/// via a frontend proxy.
pub trait MarketMiddleware {
    /// Called before any instruction, giving middleware access to the raw
    /// instruction data. This can be used to access extra data that is
    /// prepended to the DEX data, allowing one to expand the capabilities of
    /// any instruction by reading the instruction data here and then
    /// using it in any of the method handlers.
    fn instruction(&mut self, _data: &mut &[u8]) -> ProgramResult {
        Ok(())
    }

    fn init_open_orders(&self, _ctx: &mut Context) -> ProgramResult {
        Ok(())
    }

    fn new_order_v3(&self, _ctx: &mut Context, _ix: &mut NewOrderInstructionV3) -> ProgramResult {
        Ok(())
    }

    fn cancel_order_v2(
        &self,
        _ctx: &mut Context,
        _ix: &mut CancelOrderInstructionV2,
    ) -> ProgramResult {
        Ok(())
    }

    fn cancel_order_by_client_id_v2(
        &self,
        _ctx: &mut Context,
        _client_id: &mut u64,
    ) -> ProgramResult {
        Ok(())
    }

    fn settle_funds(&self, _ctx: &mut Context) -> ProgramResult {
        Ok(())
    }

    fn close_open_orders(&self, _ctx: &mut Context) -> ProgramResult {
        Ok(())
    }

    fn consume_events(&self, _ctx: &mut Context, _limit: &mut u16) -> ProgramResult {
        Ok(())
    }

    fn consume_events_permissioned(&self, _ctx: &mut Context, _limit: &mut u16) -> ProgramResult {
        Ok(())
    }

    fn prune(&self, _ctx: &mut Context, _limit: &mut u16) -> ProgramResult {
        Ok(())
    }

    /// Called when the instruction data doesn't match any DEX instruction.
    fn fallback(&self, _ctx: &mut Context) -> ProgramResult {
        Ok(())
    }
}

/// Checks that the given open orders account signs the transaction and then
/// replaces it with the open orders account, which must be a PDA.
#[derive(Default)]
pub struct OpenOrdersPda {
    bump: u8,
    bump_init: u8,
}

impl OpenOrdersPda {
    pub fn new() -> Self {
        Self {
            bump: 0,
            bump_init: 0,
        }
    }

    fn prepare_pda<'info>(acc_info: &AccountInfo<'info>) -> AccountInfo<'info> {
        let mut acc_info = acc_info.clone();
        acc_info.is_signer = true;
        acc_info
    }

    /// Validates the accounts structure for init_open_orders
    fn validate_init_accounts<'info>(accounts: &[AccountInfo<'info>]) -> ProgramResult {
        if accounts.len() < 5 {
            msg!("Not enough accounts provided for init_open_orders");
            return Err(ProgramError::NotEnoughAccountKeys.into());
        }

        let authority = &accounts[1];
        if !authority.is_signer {
            msg!("Authority must be signer");
            return Err(ProgramError::MissingRequiredSignature.into());
        }

        Ok(())
    }
}

impl MarketMiddleware for OpenOrdersPda {
    fn instruction(&mut self, data: &mut &[u8]) -> ProgramResult {
        // Strip the discriminator.
        let disc = data[0];
        *data = &data[1..];

        // Discriminator == 0 implies it's the init instruction.
        if disc == 0 {
            self.bump = data[0];
            self.bump_init = data[1];
            *data = &data[2..];
        }
        Ok(())
    }

    /// Accounts:
    ///
    /// 0. Dex program.
    /// 1. System program.
    /// .. serum_dex::MarketInstruction::InitOpenOrders.
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// 1..2 Borsh(struct { bump: u8, bump_init: u8 }).
    /// ..
    fn init_open_orders<'a, 'info>(&self, ctx: &mut Context<'a, 'info>) -> ProgramResult {
        let market = &ctx.accounts[4];
        let user = &ctx.accounts[3];

        // Skip first 2 accounts (dex_program and system_program) for validation
        let remaining_accounts = &ctx.accounts[2..];
        
        // Validate account structure
        Self::validate_init_accounts(remaining_accounts)?;

        // Add PDA seeds to context
        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });
        
        ctx.seeds.push(open_orders_init_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            bump = self.bump_init
        });

        // Update accounts (skip first 2)
        ctx.accounts = ctx.accounts[2..].to_vec();

        // Set PDAs - make sure we have enough accounts
        if ctx.accounts.len() > 1 {
            ctx.accounts[1] = Self::prepare_pda(&ctx.accounts[0]);
        }
        
        if ctx.accounts.len() > 4 {
            ctx.accounts[4].is_signer = true;
        }

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn new_order_v3(&self, ctx: &mut Context, ix: &mut NewOrderInstructionV3) -> ProgramResult {
        // The user must authorize the tx.
        let user = &ctx.accounts[7];
        if !user.is_signer {
            return Err(ProgramError::Custom(ErrorCode::UnauthorizedUser as u32).into());
        }

        let market = &ctx.accounts[0];
        let open_orders = &ctx.accounts[1];
        let token_account_payer = &ctx.accounts[6];

        // Pre: Give the PDA delegate access.
        let pre_instruction = {
            let amount = match ix.side {
                Side::Bid => ix.max_native_pc_qty_including_fees.get(),
                Side::Ask => {
                    // +5 for padding.
                    let coin_lot_idx = 5 + 43 * 8;
                    let data = market.try_borrow_data()?;
                    let mut coin_lot_array = [0u8; 8];
                    coin_lot_array.copy_from_slice(&data[coin_lot_idx..coin_lot_idx + 8]);
                    let coin_lot_size = u64::from_le_bytes(coin_lot_array);
                    ix.max_coin_qty.get().checked_mul(coin_lot_size).unwrap()
                }
            };
            let ix = spl_token::instruction::approve(
                &spl_token::ID,
                token_account_payer.key,
                open_orders.key,
                user.key,
                &[],
                amount,
            )?;
            let accounts = vec![
                token_account_payer.clone(),
                open_orders.clone(),
                user.clone(),
            ];
            (ix, accounts, Vec::new())
        };
        ctx.pre_instructions.push(pre_instruction);

        // Post: Revoke the PDA's delegate access.
        let post_instruction = {
            let ix = spl_token::instruction::revoke(
                &spl_token::ID,
                token_account_payer.key,
                user.key,
                &[],
            )?;
            let accounts = vec![token_account_payer.clone(), user.clone()];
            (ix, accounts, Vec::new())
        };
        ctx.post_instructions.push(post_instruction);

        // Proxy: PDA must sign the new order.
        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });
        ctx.accounts[7] = Self::prepare_pda(open_orders);

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn cancel_order_v2(
        &self,
        ctx: &mut Context,
        _ix: &mut CancelOrderInstructionV2,
    ) -> ProgramResult {
        let market = &ctx.accounts[0];
        let user = &ctx.accounts[4];
        if !user.is_signer {
            return Err(ProgramError::Custom(ErrorCode::UnauthorizedUser as u32).into());
        }

        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });

        ctx.accounts[4] = Self::prepare_pda(&ctx.accounts[3]);

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn cancel_order_by_client_id_v2(
        &self,
        ctx: &mut Context,
        _client_id: &mut u64,
    ) -> ProgramResult {
        let market = &ctx.accounts[0];
        let user = &ctx.accounts[4];
        if !user.is_signer {
            return Err(ProgramError::Custom(ErrorCode::UnauthorizedUser as u32).into());
        }

        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });

        ctx.accounts[4] = Self::prepare_pda(&ctx.accounts[3]);

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn settle_funds(&self, ctx: &mut Context) -> ProgramResult {
        let market = &ctx.accounts[0];
        let user = &ctx.accounts[2];
        if !user.is_signer {
            return Err(ProgramError::Custom(ErrorCode::UnauthorizedUser as u32).into());
        }

        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });

        ctx.accounts[2] = Self::prepare_pda(&ctx.accounts[1]);

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn close_open_orders(&self, ctx: &mut Context) -> ProgramResult {
        let market = &ctx.accounts[3];
        let user = &ctx.accounts[1];
        if !user.is_signer {
            return Err(ProgramError::Custom(ErrorCode::UnauthorizedUser as u32).into());
        }

        ctx.seeds.push(open_orders_authority! {
            program = ctx.program_id,
            dex_program = ctx.dex_program_id,
            market = market.key,
            authority = user.key,
            bump = self.bump
        });

        ctx.accounts[1] = Self::prepare_pda(&ctx.accounts[0]);

        Ok(())
    }

    /// Accounts:
    ///
    /// ..
    ///
    /// Data:
    ///
    /// 0.   Discriminant.
    /// ..
    fn prune(&self, ctx: &mut Context, _limit: &mut u16) -> ProgramResult {
        // Set owner of open orders to be itself.
        ctx.accounts[5] = ctx.accounts[4].clone();
        Ok(())
    }
}

/// Logs each request.
pub struct Logger;

impl MarketMiddleware for Logger {
    fn init_open_orders(&self, _ctx: &mut Context) -> ProgramResult {
        msg!("proxying open orders");
        Ok(())
    }

    fn new_order_v3(&self, _ctx: &mut Context, ix: &mut NewOrderInstructionV3) -> ProgramResult {
        msg!("proxying new order v3 {:?}", ix);
        Ok(())
    }

    fn cancel_order_v2(
        &self,
        _ctx: &mut Context,
        ix: &mut CancelOrderInstructionV2,
    ) -> ProgramResult {
        msg!("proxying cancel order v2 {:?}", ix);
        Ok(())
    }

    fn cancel_order_by_client_id_v2(
        &self,
        _ctx: &mut Context,
        client_id: &mut u64,
    ) -> ProgramResult {
        msg!("proxying cancel order by client id v2 {:?}", client_id);
        Ok(())
    }

    fn settle_funds(&self, _ctx: &mut Context) -> ProgramResult {
        msg!("proxying settle funds");
        Ok(())
    }

    fn close_open_orders(&self, _ctx: &mut Context) -> ProgramResult {
        msg!("proxying close open orders");
        Ok(())
    }

    fn prune(&self, _ctx: &mut Context, limit: &mut u16) -> ProgramResult {
        msg!("proxying prune {:?}", limit);
        Ok(())
    }
}

/// Enforces referral fees being sent to the configured address.
pub struct ReferralFees {
    referral: Pubkey,
}

impl ReferralFees {
    pub fn new(referral: Pubkey) -> Self {
        Self { referral }
    }
}

impl MarketMiddleware for ReferralFees {
    /// Accounts:
    ///
    /// .. serum_dex::MarketInstruction::SettleFunds.
    fn settle_funds(&self, ctx: &mut Context) -> ProgramResult {
        let referral = token::accessor::authority(&ctx.accounts[9])
            .map_err(|e| Into::<ProgramError>::into(e))?;
        if referral != self.referral {
            return Err(ProgramError::Custom(ErrorCode::InvalidReferral as u32).into());
        }
        Ok(())
    }
}

// Macros.

/// Returns the seeds used for a user's open orders account PDA.
#[macro_export]
macro_rules! open_orders_authority {
    (
        program = $program:expr,
        dex_program = $dex_program:expr,
        market = $market:expr,
        authority = $authority:expr,
        bump = $bump:expr
    ) => {
        vec![
            b"open-orders".to_vec(),
            $dex_program.as_ref().to_vec(),
            $market.as_ref().to_vec(),
            $authority.as_ref().to_vec(),
            vec![$bump],
        ]
    };
    (
        program = $program:expr,
        dex_program = $dex_program:expr,
        market = $market:expr,
        authority = $authority:expr
    ) => {
        vec![
            b"open-orders".to_vec(),
            $dex_program.as_ref().to_vec(),
            $market.as_ref().to_vec(),
            $authority.as_ref().to_vec(),
            vec![
                Pubkey::find_program_address(
                    &[
                        b"open-orders".as_ref(),
                        $dex_program.as_ref(),
                        $market.as_ref(),
                        $authority.as_ref(),
                    ],
                    $program,
                )
                .1,
            ],
        ]
    };
}

/// Returns the seeds used for the open orders init authority.
/// This is the account that must sign to create a new open orders account on
/// the DEX market.
#[macro_export]
macro_rules! open_orders_init_authority {
    (
        program = $program:expr,
        dex_program = $dex_program:expr,
        market = $market:expr,
        bump = $bump:expr
    ) => {
        vec![
            b"open-orders-init".to_vec(),
            $dex_program.as_ref().to_vec(),
            $market.as_ref().to_vec(),
            vec![$bump],
        ]
    };
}

// Errors.

#[error_code(offset = 500)]
pub enum ErrorCode {
    #[msg("Program ID does not match the Serum DEX")]
    InvalidDexPid,
    #[msg("Invalid instruction given")]
    InvalidInstruction,
    #[msg("Could not unpack the instruction")]
    CannotUnpack,
    #[msg("Invalid referral address given")]
    InvalidReferral,
    #[msg("The user didn't sign")]
    UnauthorizedUser,
    #[msg("Not enough accounts were provided")]
    NotEnoughAccounts,
    #[msg("Invalid target program ID")]
    InvalidTargetProgram,
}

// Constants.

// Padding added to every serum account.
//
// b"serum".len() + b"padding".len().
// const SERUM_PADDING: usize = 12;

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;
    use solana_program::account_info::AccountInfo;
    use solana_program::clock::Epoch;
    use std::convert::TryInto;

    fn dummy_account(is_signer: bool) -> AccountInfo<'static> {
        let key = Box::leak(Box::new(Pubkey::new_unique()));
        let owner = Box::leak(Box::new(Pubkey::default()));
        let lamports = Box::leak(Box::new(0u64));
        let data = Box::leak(Vec::new().into_boxed_slice());
        AccountInfo::new(
            key,
            is_signer,
            false,
            lamports,
            data,
            owner,
            false,
            Epoch::default(),
        )
    }

    #[test]
    fn test_instruction_parsing() {
        let mut pda = OpenOrdersPda::new();
        let mut data: &[u8] = &[0, 42, 99, 1, 2, 3];
        pda.instruction(&mut data).unwrap();
        assert_eq!(pda.bump, 42);
        assert_eq!(pda.bump_init, 99);
        assert_eq!(data, &[1, 2, 3]);
    }

    #[test]
    fn test_init_open_orders_valid() {
        let pda = OpenOrdersPda { bump: 1, bump_init: 2 };
        let program_id = Pubkey::new_unique();
        let dex_program_id = Pubkey::new_unique();
        // Provide 7 accounts: after skipping 2, 5 remain.
        // The 2nd account after skipping (index 1) must be a signer, so set original index 3 as signer.
        let accounts: Vec<_> = (0..7)
            .map(|i| dummy_account(i == 3))
            .collect();
        let mut ctx = Context::new(&program_id, &dex_program_id, accounts);
        assert!(pda.init_open_orders(&mut ctx).is_ok());
        assert_eq!(ctx.seeds.len(), 2);
    }

    #[test]
    fn test_init_open_orders_missing_signer() {
        let pda = OpenOrdersPda { bump: 1, bump_init: 2 };
        let program_id = Pubkey::new_unique();
        let dex_program_id = Pubkey::new_unique();
        let accounts: Vec<_> = (0..6)
            .map(|_| dummy_account(false))
            .collect();
        let mut ctx = Context::new(&program_id, &dex_program_id, accounts);
        assert!(pda.init_open_orders(&mut ctx).is_err());
    }

    #[test]
    fn test_logger_hooks() {
        let logger = Logger;
        let program_id = Pubkey::new_unique();
        let dex_program_id = Pubkey::new_unique();
        let accounts: Vec<_> = (0..6)
            .map(|i| dummy_account(i == 1))
            .collect();
        let mut ctx = Context::new(&program_id, &dex_program_id, accounts);
        assert!(logger.init_open_orders(&mut ctx).is_ok());
        let mut ix = NewOrderInstructionV3 {
            side: Side::Bid,
            limit_price: 1u64.try_into().unwrap(),
            max_coin_qty: 1u64.try_into().unwrap(),
            max_native_pc_qty_including_fees: 1u64.try_into().unwrap(),
            self_trade_behavior: serum_dex::instruction::SelfTradeBehavior::AbortTransaction,
            order_type: serum_dex::matching::OrderType::Limit,
            client_order_id: 0,
            limit: 1,
            max_ts: 0,
        };
        assert!(logger.new_order_v3(&mut ctx, &mut ix).is_ok());
    }
}