// the fee gets recomputed every time instead of cached. TransferFeeConfig holds
// an old rate and a new one and swaps at an epoch boundary, and the program
// checks your number against its own.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use spl_token_2022_interface::extension::transfer_fee;

use crate::{inspect, send, DECIMALS};

// (fee, what the receiver actually gets)
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

// pass your own fee. only the boundary tests use this.
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

// the plain instruction, for the test that shows what it does differently
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
