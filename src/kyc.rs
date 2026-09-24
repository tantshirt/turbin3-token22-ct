// two separate levers. thaw is one account right now. default state is what the
// next account gets. neither one touches the other.

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

// only affects accounts created after this
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
