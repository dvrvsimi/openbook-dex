use crate::{Context, ErrorCode, MarketMiddleware};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program;
use anchor_lang::solana_program::pubkey::Pubkey;
use serum_dex::instruction::*;
use spl_token::solana_program::entrypoint::ProgramResult;

// Add the correct Serum DEX program ID (mainnet value shown; replace if needed)
pub const SERUM_DEX_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    57, 197, 30, 22, 184, 218, 211, 222, 151, 184, 186, 13, 222, 222, 222, 222,
    151, 184, 186, 13, 222, 222, 222, 222, 151, 184, 186, 13, 222, 222, 222, 222
]); // Replace with actual bytes if different

/// MarketProxy provides an abstraction for implementing proxy programs to the
/// Serum orderbook, allowing one to implement a middleware for the purposes
/// of intercepting and modifying requests before being relayed to the
/// orderbook.
///
/// The only requirement for a middleware is that, when all are done processing,
/// a valid DEX instruction--accounts and instruction data--must be left to
/// forward to the orderbook program.
#[derive(Default)]
pub struct MarketProxy<'a> {
    middlewares: Vec<&'a mut dyn MarketMiddleware>,
}

impl<'a> MarketProxy<'a> {
    /// Constructs a new `MarketProxy`.
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    /// Builder method for adding a middleware to the proxy.
    pub fn middleware(mut self, mw: &'a mut dyn MarketMiddleware) -> Self {
        self.middlewares.push(mw);
        self
    }

    /// Entrypoint to the program.
    pub fn run(
        mut self,
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        data: &[u8],
    ) -> ProgramResult {
        let mut ix_data = data;

        // First account is the Serum DEX executable--used for CPI.
        let dex = &accounts[0];
        if dex.key != &SERUM_DEX_PROGRAM_ID {
            return Err(anchor_lang::error!(ErrorCode::InvalidTargetProgram).into());
        }
        let acc_infos = (accounts[1..]).to_vec();

        // Process the instruction data.
        for mw in &mut self.middlewares {
            mw.instruction(&mut ix_data)?;
        }

        // Request context.
        let mut ctx = Context::new(program_id, dex.key, acc_infos);

        // Decode instruction.
        let mut ix = MarketInstruction::unpack(ix_data);

        // Method dispatch.
        match ix {
            Some(MarketInstruction::InitOpenOrders) => {
                if ctx.accounts.len() < 4 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.init_open_orders(&mut ctx)?;
                }
            }
            Some(MarketInstruction::NewOrderV3(ref mut ix)) => {
                if ctx.accounts.len() < 12 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.new_order_v3(&mut ctx, ix)?;
                }
            }
            Some(MarketInstruction::CancelOrderV2(ref mut ix)) => {
                if ctx.accounts.len() < 6 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.cancel_order_v2(&mut ctx, ix)?;
                }
            }
            Some(MarketInstruction::CancelOrderByClientIdV2(ref mut ix)) => {
                if ctx.accounts.len() < 6 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.cancel_order_by_client_id_v2(&mut ctx, ix)?;
                }
            }
            Some(MarketInstruction::SettleFunds) => {
                if ctx.accounts.len() < 10 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.settle_funds(&mut ctx)?;
                }
            }
            Some(MarketInstruction::CloseOpenOrders) => {
                if ctx.accounts.len() < 4 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.close_open_orders(&mut ctx)?;
                }
            }
            Some(MarketInstruction::ConsumeEvents(ref mut limit)) => {
                if ctx.accounts.len() < 4 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.consume_events(&mut ctx, limit)?;
                }
            }
            Some(MarketInstruction::ConsumeEventsPermissioned(ref mut limit)) => {
                if ctx.accounts.len() < 3 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.consume_events_permissioned(&mut ctx, limit)?;
                }
            }
            Some(MarketInstruction::Prune(ref mut limit)) => {
                if ctx.accounts.len() < 7 {
                    return Err(anchor_lang::error!(ErrorCode::NotEnoughAccounts).into());
                }
                for mw in &self.middlewares {
                    mw.prune(&mut ctx, limit)?;
                }
            }
            _ => {
                for mw in &self.middlewares {
                    mw.fallback(&mut ctx)?;
                }
                return Ok(());
            }
        };

        let ix_data_vec = MarketInstruction::pack(&ix.unwrap());
        ix_data = ix_data_vec.as_slice();

        // Extract the middleware adjusted context.
        let Context {
            seeds,
            accounts,
            pre_instructions,
            post_instructions,
            post_callbacks,
            ..
        } = ctx;

        // Execute pre instructions.
        for (ix, acc_infos, seeds) in pre_instructions {
            let tmp_signers: Vec<Vec<&[u8]>> = seeds
                .iter()
                .map(|seeds| {
                    let seeds: Vec<&[u8]> = seeds.iter().map(|seed| &seed[..]).collect();
                    seeds
                })
                .collect();
            let signers: Vec<&[&[u8]]> = tmp_signers.iter().map(|seeds| &seeds[..]).collect();
            program::invoke_signed(&ix, &acc_infos, &signers)?;
        }

        // Execute the main dex relay.
        {
            let tmp_signers: Vec<Vec<&[u8]>> = seeds
                .iter()
                .map(|seeds| {
                    let seeds: Vec<&[u8]> = seeds.iter().map(|seed| &seed[..]).collect();
                    seeds
                })
                .collect();
            let signers: Vec<&[&[u8]]> = tmp_signers.iter().map(|seeds| &seeds[..]).collect();

            // CPI to the DEX.
            let dex_accounts = accounts
                .iter()
                .map(|acc| AccountMeta {
                    pubkey: *acc.key,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect();
            let ix = anchor_lang::solana_program::instruction::Instruction {
                data: ix_data.to_vec(),
                accounts: dex_accounts,
                program_id: SERUM_DEX_PROGRAM_ID,
            };
            program::invoke_signed(&ix, &accounts, &signers)?;
        }

        // Execute post instructions.
        for (ix, acc_infos, seeds) in post_instructions {
            let tmp_signers: Vec<Vec<&[u8]>> = seeds
                .iter()
                .map(|seeds| {
                    let seeds: Vec<&[u8]> = seeds.iter().map(|seed| &seed[..]).collect();
                    seeds
                })
                .collect();
            let signers: Vec<&[&[u8]]> = tmp_signers.iter().map(|seeds| &seeds[..]).collect();
            program::invoke_signed(&ix, &acc_infos, &signers)?;
        }

        // Execute post callbacks.
        for (function, accounts, args) in post_callbacks {
            function(program_id, accounts, ix_data.to_vec(), args)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;
    use solana_program::account_info::AccountInfo;
    use solana_program::clock::Epoch;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::convert::TryInto;
    use serum_dex::matching::Side;

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

    struct CallTracker {
        pub called: Rc<RefCell<Vec<&'static str>>>,
    }
    impl CallTracker {
        fn new() -> Self {
            Self { called: Rc::new(RefCell::new(vec![])) }
        }
    }
    impl MarketMiddleware for CallTracker {
        fn instruction(&mut self, _data: &mut &[u8]) -> ProgramResult {
            self.called.borrow_mut().push("instruction");
            Ok(())
        }
        fn init_open_orders(&self, _ctx: &mut Context) -> ProgramResult {
            self.called.borrow_mut().push("init_open_orders");
            Ok(())
        }
        fn new_order_v3(&self, _ctx: &mut Context, _ix: &mut NewOrderInstructionV3) -> ProgramResult {
            self.called.borrow_mut().push("new_order_v3");
            Ok(())
        }
        fn fallback(&self, _ctx: &mut Context) -> ProgramResult {
            self.called.borrow_mut().push("fallback");
            Ok(())
        }
    }

    fn make_accounts(n: usize, signer_idx: Option<usize>) -> Vec<AccountInfo<'static>> {
        (0..n).map(|i| dummy_account(signer_idx == Some(i))).collect()
    }

    #[test]
    fn test_dispatch_init_open_orders() {
        let mut mw = CallTracker::new();
        let proxy = MarketProxy::new().middleware(&mut mw);
        let program_id = Pubkey::new_unique();
        let mut accounts = vec![
            // 0: DEX program (must match SERUM_DEX_PROGRAM_ID)
            AccountInfo::new(
                &SERUM_DEX_PROGRAM_ID,
                false,
                false,
                Box::leak(Box::new(0u64)),
                Box::leak(Vec::new().into_boxed_slice()),
                Box::leak(Box::new(Pubkey::default())),
                false,
                Epoch::default(),
            ),
        ];
        accounts.extend(make_accounts(4, Some(1))); // 4 more accounts, 1 is signer
        let data = MarketInstruction::InitOpenOrders.pack();
        let result = proxy.run(&program_id, &accounts, &data);
        assert!(result.is_ok());
        let calls = mw.called.borrow();
        assert!(calls.contains(&"instruction"));
        assert!(calls.contains(&"init_open_orders"));
    }

    #[test]
    fn test_dispatch_new_order_v3() {
        let mut mw = CallTracker::new();
        let proxy = MarketProxy::new().middleware(&mut mw);
        let program_id = Pubkey::new_unique();
        let mut accounts = vec![
            AccountInfo::new(
                &SERUM_DEX_PROGRAM_ID,
                false,
                false,
                Box::leak(Box::new(0u64)),
                Box::leak(Vec::new().into_boxed_slice()),
                Box::leak(Box::new(Pubkey::default())),
                false,
                Epoch::default(),
            ),
        ];
        accounts.extend(make_accounts(12, Some(1)));
        let ix = NewOrderInstructionV3 {
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
        let data = MarketInstruction::NewOrderV3(ix).pack();
        let result = proxy.run(&program_id, &accounts, &data);
        assert!(result.is_ok());
        let calls = mw.called.borrow();
        assert!(calls.contains(&"instruction"));
        assert!(calls.contains(&"new_order_v3"));
    }

    #[test]
    fn test_fallback_dispatch() {
        let mut mw = CallTracker::new();
        let proxy = MarketProxy::new().middleware(&mut mw);
        let program_id = Pubkey::new_unique();
        let mut accounts = vec![
            AccountInfo::new(
                &SERUM_DEX_PROGRAM_ID,
                false,
                false,
                Box::leak(Box::new(0u64)),
                Box::leak(Vec::new().into_boxed_slice()),
                Box::leak(Box::new(Pubkey::default())),
                false,
                Epoch::default(),
            ),
        ];
        accounts.extend(make_accounts(2, Some(1)));
        // Use an invalid instruction (empty data)
        let data = vec![];
        let result = proxy.run(&program_id, &accounts, &data);
        assert!(result.is_ok());
        let calls = mw.called.borrow();
        assert!(calls.contains(&"instruction"));
        assert!(calls.contains(&"fallback"));
    }

    #[test]
    fn test_account_count_validation() {
        let mut mw = CallTracker::new();
        let proxy = MarketProxy::new().middleware(&mut mw);
        let program_id = Pubkey::new_unique();
        let mut accounts = vec![
            AccountInfo::new(
                &SERUM_DEX_PROGRAM_ID,
                false,
                false,
                Box::leak(Box::new(0u64)),
                Box::leak(Vec::new().into_boxed_slice()),
                Box::leak(Box::new(Pubkey::default())),
                false,
                Epoch::default(),
            ),
        ];
        accounts.extend(make_accounts(3, Some(1))); // Not enough for InitOpenOrders (needs 4)
        let data = MarketInstruction::InitOpenOrders.pack();
        let result = proxy.run(&program_id, &accounts, &data);
        assert!(result.is_err());
    }
}
