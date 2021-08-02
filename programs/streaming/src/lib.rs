use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount, Transfer};

#[program]
pub mod streaming {
    use super::*;

    #[access_control(CreateStream::valid_vault_owner(&ctx, nonce))]
    pub fn create_stream(
        ctx: Context<CreateStream>,
        beneficiary: Pubkey,
        original_deposit_size: u64,
        start_ts: i64,
        end_ts: i64,
        nonce: u8,
    ) -> ProgramResult {
        if original_deposit_size == 0 {
            return Err(ErrorCode::InvalidDepositAmount.into());
        }

        if !is_valid_schedule(start_ts, end_ts, ctx.accounts.clock.unix_timestamp) {
            return Err(ErrorCode::InvalidSchedule.into());
        }

        let streaming = &mut ctx.accounts.streaming;

        streaming.beneficiary = beneficiary;
        streaming.grantor = *ctx.accounts.depositor_authority.key;
        streaming.mint = ctx.accounts.vault.mint;
        streaming.original_deposit_size = original_deposit_size;
        streaming.outstanding = original_deposit_size;
        streaming.created_ts = ctx.accounts.clock.unix_timestamp;
        streaming.start_ts = start_ts;
        streaming.end_ts = end_ts;
        streaming.nonce = nonce;

        token::transfer(ctx.accounts.into(), original_deposit_size)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> ProgramResult {
        if amount > available_for_withdrawal(&ctx) {
            return Err(ErrorCode::InvalidWithdrawAmount.into());
        }

        let seeds = &[
            ctx.accounts.streaming.to_account_info().key.as_ref(),
            &[ctx.accounts.streaming.nonce],
        ];

        let signer = &[&seeds[..]];
        let cpi_ctx = CpiContext::from(&*ctx.accounts).with_signer(signer);
        token::transfer(cpi_ctx, amount)?;

        Ok(())
    }
}

// Context structs
// Notes: None

#[derive(Accounts)]
pub struct CreateStream<'info> {
    // Streaming account
    #[account(init)]
    streaming: ProgramAccount<'info, Streaming>,

    // Streaming account's token account
    #[account(mut)]
    vault: CpiAccount<'info, TokenAccount>,

    // Depositor (account that signs tx/ix)
    depositor: AccountInfo<'info>,
    #[account(signer)]
    depositor_authority: AccountInfo<'info>,

    // Misc accounts
    #[account("token_program.key == &token::ID")]
    token_program: AccountInfo<'info>,
    rent: Sysvar<'info, Rent>,
    clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut, has_one = beneficiary, has_one = vault)]
    streaming: ProgramAccount<'info, Streaming>,

    // Accounts that the streaming account has as indicated by the has_one param
    #[account(signer)]
    beneficiary: AccountInfo<'info>,
    #[account(mut)]
    vault: CpiAccount<'info, TokenAccount>,

    // PDA that controls the streaming account's vault
    #[account(seeds = [streaming.to_account_info().key.as_ref(), &[streaming.nonce]])]
    vault_authority: AccountInfo<'info>,

    // Receiver's token vault
    #[account(mut)]
    receiver_vault: CpiAccount<'info, TokenAccount>,

    // Misc accounts
    #[account("token_program.key == &token::ID")]
    token_program: AccountInfo<'info>,
    clock: Sysvar<'info, Clock>,
}

// Account structs
// Notes: None

#[account]
pub struct Streaming {
    // The pubkey that can withdraw from the streaming account
    pub beneficiary: Pubkey,
    // The pubkey that funded the streaming account
    pub grantor: Pubkey,
    // The mint of the tokens being locked up
    pub mint: Pubkey,
    // The token account of the streaming account
    pub vault: Pubkey,
    // The amount of tokens deposited into the account by the grantor
    pub original_deposit_size: u64,
    // The amount of the original_deposite_size that still remains in the account
    pub outstanding: u64,
    // The unix timestamp of when the streaming account was created
    pub created_ts: i64,
    // The unix timestamp of the stream start time
    pub start_ts: i64,
    // The unix timestamp of the stream end time
    pub end_ts: i64,
    // Number used once on account init
    pub nonce: u8,
}

// Context struct functions
// Notes: Both only implement access control functions

impl<'info> CreateStream<'info> {
    // Checks to see that the streaming account's relevant token account is actually controlled
    // by the program and not an invalid owner
    fn valid_vault_owner(ctx: &Context<CreateStream>, nonce: u8) -> ProgramResult {
        let vault_authority = Pubkey::create_program_address(
            &[
                ctx.accounts.streaming.to_account_info().key.as_ref(),
                &[nonce],
            ],
            ctx.program_id,
        )
        .map_err(|_| ErrorCode::InvalidVaultAuthority)?;

        if ctx.accounts.vault.owner != vault_authority {
            return Err(ErrorCode::InvalidVaultAuthority)?;
        }

        Ok(())
    }
}

// Trait implementations
// Notes: None

impl<'a, 'b, 'c, 'info> From<&mut CreateStream<'info>>
    for CpiContext<'a, 'b, 'c, 'info, Transfer<'info>>
{
    fn from(accounts: &mut CreateStream<'info>) -> CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: accounts.depositor.clone(),
            to: accounts.vault.to_account_info(),
            authority: accounts.depositor_authority.clone(),
        };
        let cpi_program = accounts.token_program.clone();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

impl<'a, 'b, 'c, 'info> From<&Withdraw<'info>> for CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
    fn from(accounts: &Withdraw<'info>) -> CpiContext<'a, 'b, 'c, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: accounts.vault.to_account_info(),
            to: accounts.receiver_vault.to_account_info(),
            authority: accounts.vault_authority.to_account_info(),
        };
        let cpi_program = accounts.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

// Errors
// Notes: None

#[error]
pub enum ErrorCode {
    #[msg("Stream end must be greater than the current unix timestamp.")]
    InvalidTimestamp,
    #[msg("Invalid authority used to access Godmode.")]
    InvalidGod,
    #[msg("Vault account has an invalid authority.")]
    InvalidVaultAuthority,
    #[msg("The deposit amount must be greater than zero.")]
    InvalidDepositAmount,
    #[msg("The streaming schedule is invalid.")]
    InvalidSchedule,
    #[msg("Over withdrawal limit.")]
    InvalidWithdrawAmount,
}

// Utility functions
// Notes: None

// Checks to see if a stream's proposed schedule is valid. Validaty criteria are simply
// making sure the start time is smaller than the end time and making sure the start time
// is larger than the current time by at least 1 minute. The program will expect UIs to
// enforce this manually
pub fn is_valid_schedule(start_ts: i64, end_ts: i64, current_time: i64) -> bool {
    if end_ts <= start_ts {
        return false;
    }

    if start_ts - current_time < 60 {
        return false;
    }
    true
}

// Checks a streaming account to see how much of the original_deposit is available for
// withdrawal
pub fn available_for_withdrawal(ctx: &Context<Withdraw>) -> u64 {
    let start_ts = ctx.accounts.streaming.start_ts;
    let end_ts = ctx.accounts.streaming.end_ts;
    let current_ts = ctx.accounts.clock.unix_timestamp;
    let max_balance = ctx.accounts.streaming.original_deposit_size;

    if current_ts < start_ts {
        return 0;
    } else if current_ts >= end_ts {
        return max_balance;
    } else {
        let delta = end_ts - current_ts;
        let rate: f64 = max_balance as f64 / delta as f64;

        let current = rate * delta as f64;
        current as u64
    }
}
