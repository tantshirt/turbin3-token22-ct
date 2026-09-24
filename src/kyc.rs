// task 4. the mint says every new account is frozen. this is how one account
// gets let through once its KYC clears.
//
// two levers, easy to mix up:
//   thaw          -> one account, right now, signed by the freeze authority
//   default state -> what the NEXT account looks like when it's created
// thawing somebody doesn't touch the mint, and changing the mint default doesn't
// retroactively unfreeze anybody.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use spl_token_2022_interface::{extension::default_account_state, state::AccountState};

use crate::send;

pub async fn approve_kyc(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    freeze_authority: &Keypair,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::thaw_account(
        &spl_token_2022_interface::id(),
        account,
        mint,
        &freeze_authority.pubkey(),
        &[],
    )?;
    send(rpc, payer, &[ix], &[payer, freeze_authority]).await
}

pub async fn revoke_kyc(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    freeze_authority: &Keypair,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::freeze_account(
        &spl_token_2022_interface::id(),
        account,
        mint,
        &freeze_authority.pubkey(),
        &[],
    )?;
    send(rpc, payer, &[ix], &[payer, freeze_authority]).await
}

// the mint level lever. only changes what new accounts look like.
pub async fn set_default_state(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    authority: &Keypair,
    state: AccountState,
) -> anyhow::Result<()> {
    let ix = default_account_state::instruction::update_default_account_state(
        &spl_token_2022_interface::id(),
        mint,
        &authority.pubkey(),
        &[],
        &state,
    )?;
    send(rpc, payer, &[ix], &[payer, authority]).await
}
