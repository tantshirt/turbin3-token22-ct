pub mod confidential;
pub mod ct_helpers;
pub mod ct_mint;
pub mod fee_mint;
pub mod fee_transfer;
pub mod inspect;
pub mod kyc;

use std::sync::Arc;

use solana_address::Address;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::{signers::Signers, Signer};
use solana_transaction::Transaction;

pub const DECIMALS: u8 = 6;

pub fn rpc() -> Arc<RpcClient> {
    Arc::new(RpcClient::new_with_commitment(
        "http://127.0.0.1:8899".to_string(),
        CommitmentConfig::confirmed(),
    ))
}

// build, sign, send. saves repeating the blockhash dance in every module.
pub async fn send<S: Signers + ?Sized>(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    instructions: &[Instruction],
    signers: &S,
) -> anyhow::Result<()> {
    let blockhash = rpc.get_latest_blockhash().await?;
    let tx = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        signers,
        blockhash,
    );
    rpc.send_and_confirm_transaction(&tx).await?;
    Ok(())
}

// the ATA client crate is on a different solana-instruction line than the rest
// of this, and it's a PDA plus a one byte instruction, so just build it here.
pub const ATA_PROGRAM: Address =
    solana_address::address!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn ata(mint: &Address, owner: &Address) -> Address {
    Address::find_program_address(
        &[
            owner.as_ref(),
            spl_token_2022_interface::id().as_ref(),
            mint.as_ref(),
        ],
        &ATA_PROGRAM,
    )
    .0
}

// anybody can pay for somebody else's account. the owner never signs.
pub async fn create_ata(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    owner: &Address,
) -> anyhow::Result<Address> {
    let account = ata(mint, owner);
    let ix = Instruction::new_with_bytes(
        ATA_PROGRAM,
        &[0], // Create
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(account, false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(solana_system_interface::program::id(), false),
            AccountMeta::new_readonly(spl_token_2022_interface::id(), false),
        ],
    );
    send(rpc, payer, &[ix], &[payer]).await?;
    Ok(account)
}

pub async fn mint_to(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    authority: &Keypair,
    amount: u64,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::mint_to(
        &spl_token_2022_interface::id(),
        mint,
        account,
        &authority.pubkey(),
        &[],
        amount,
    )?;
    send(rpc, payer, &[ix], &[payer, authority]).await
}

pub async fn funded_payer() -> Keypair {
    let rpc = rpc();
    let payer = Keypair::new();
    let sig = rpc
        .request_airdrop(&payer.pubkey(), 20 * 1_000_000_000)
        .await
        .expect("airdrop failed, is the validator running?");
    while !rpc.confirm_transaction(&sig).await.unwrap_or(false) {}
    payer
}

pub async fn burn(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
    amount: u64,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::burn(
        &spl_token_2022_interface::id(),
        account,
        mint,
        &owner.pubkey(),
        &[],
        amount,
    )?;
    send(rpc, payer, &[ix], &[payer, owner]).await
}

// only works once the supply is zero
pub async fn close_mint(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    close_authority: &Keypair,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::close_account(
        &spl_token_2022_interface::id(),
        mint,
        &payer.pubkey(),
        &close_authority.pubkey(),
        &[],
    )?;
    send(rpc, payer, &[ix], &[payer, close_authority]).await
}
