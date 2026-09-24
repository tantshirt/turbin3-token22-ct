// every read goes through StateWithExtensions. a plain unpack only sees the
// first 82 bytes and misses every extension.

use std::sync::Arc;

use solana_address::Address;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, StateWithExtensions,
    },
    state::{Account, AccountState, Mint},
};

// what the mint charges at this epoch
pub async fn epoch_fee(rpc: &Arc<RpcClient>, mint: &Address, amount: u64) -> anyhow::Result<u64> {
    let account = rpc.get_account(mint).await?;
    let state = StateWithExtensions::<Mint>::unpack(&account.data)?;
    let config = state.get_extension::<TransferFeeConfig>()?;

    let epoch = rpc.get_epoch_info().await?.epoch;

    config
        .calculate_epoch_fee(epoch, amount)
        .ok_or_else(|| anyhow::anyhow!("fee math overflowed on amount {amount}"))
}

pub async fn is_frozen(rpc: &Arc<RpcClient>, account: &Address) -> anyhow::Result<bool> {
    let raw = rpc.get_account(account).await?;
    let state = StateWithExtensions::<Account>::unpack(&raw.data)?;
    Ok(state.base.state == AccountState::Frozen)
}

pub async fn balance(rpc: &Arc<RpcClient>, account: &Address) -> anyhow::Result<u64> {
    let raw = rpc.get_account(account).await?;
    let state = StateWithExtensions::<Account>::unpack(&raw.data)?;
    Ok(state.base.amount)
}
