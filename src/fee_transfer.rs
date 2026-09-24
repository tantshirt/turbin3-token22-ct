// task 2. transfer_checked_with_fee, and the fee comes from calculate_epoch_fee
// every single time.
//
// it can't be cached. the program recomputes the fee on chain with its own Clock
// and rejects the transaction if your number doesn't match. TransferFeeConfig
// actually holds two rates, an older and a newer one, and swaps between them at
// an epoch boundary, so a stored rate goes stale the moment the issuer changes it.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use spl_token_2022_interface::extension::transfer_fee;

use crate::{inspect, send, DECIMALS};

// what this transfer costs right now. returns (fee, what the receiver gets).
pub async fn quote(rpc: &Arc<RpcClient>, mint: &Address, amount: u64) -> anyhow::Result<(u64, u64)> {
    let fee = inspect::epoch_fee(rpc, mint, amount).await?;
    Ok((fee, amount - fee))
}

pub async fn send_with_fee(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    from: &Address,
    to: &Address,
    owner: &Keypair,
    amount: u64,
) -> anyhow::Result<u64> {
    let (fee, _) = quote(rpc, mint, amount).await?;
    send_with_explicit_fee(rpc, payer, mint, from, to, owner, amount, fee).await?;
    Ok(fee)
}

// same thing but you pass the fee yourself. only the boundary tests use this,
// to prove the program rejects a number that doesn't match.
pub async fn send_with_explicit_fee(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    from: &Address,
    to: &Address,
    owner: &Keypair,
    amount: u64,
    fee: u64,
) -> anyhow::Result<()> {
    let ix = transfer_fee::instruction::transfer_checked_with_fee(
        &spl_token_2022_interface::id(),
        from,
        mint,
        to,
        &owner.pubkey(),
        &[],
        amount,
        DECIMALS,
        fee,
    )?;
    send(rpc, payer, &[ix], &[payer, owner]).await
}

// the ordinary instruction, for the test that shows what the difference is.
pub async fn send_plain(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    from: &Address,
    to: &Address,
    owner: &Keypair,
    amount: u64,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::transfer_checked(
        &spl_token_2022_interface::id(),
        from,
        mint,
        to,
        &owner.pubkey(),
        &[],
        amount,
        DECIMALS,
    )?;
    send(rpc, payer, &[ix], &[payer, owner]).await
}
