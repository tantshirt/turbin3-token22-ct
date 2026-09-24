// deposits and incoming transfers land in pending, which isn't spendable.
// apply_pending moves it to available. skip that and withdraw proves the wrong
// number and dies before it's even sent.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use solana_zk_sdk::{
    encryption::{auth_encryption::AeKey, derivation::derive_confidential_keys, elgamal::{ElGamalKeypair, ElGamalPubkey}},
    zk_elgamal_proof_program::pubkey_validity::build_pubkey_validity_proof_data,
};
use spl_token_2022_interface::{
    extension::{
        confidential_transfer::{instruction as ct, ConfidentialTransferAccount},
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account, Mint},
};
use solana_zk_elgamal_proof_interface::{
    instruction::{close_context_state, ContextStateInfo, ProofInstruction},
    state::ProofContextState,
    proof_data::ZkProofData,
};
use spl_token_confidential_transfer_proof_extraction::instruction::ProofLocation;

use crate::{
    ct_helpers::{ApplyPendingBalanceAccountInfo, TransferAccountInfo, WithdrawAccountInfo},
    send, DECIMALS,
};

// derived from the owner signing a message, which is why only the owner can
// configure an account and why nothing has to be stored.
pub struct Keys {
    pub elgamal: ElGamalKeypair,
    pub ae: AeKey,
}

pub fn keys(owner: &Keypair, account: &Address) -> anyhow::Result<Keys> {
    let (elgamal, ae) = derive_confidential_keys(owner, &account.to_bytes())
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(Keys { elgamal, ae })
}

// copied out, not borrowed, because the unpacked view borrows the raw bytes.
async fn ct_account(
    rpc: &Arc<RpcClient>,
    account: &Address,
) -> anyhow::Result<ConfidentialTransferAccount> {
    let raw = rpc.get_account(account).await?;
    let state = StateWithExtensions::<Account>::unpack(&raw.data)?;
    Ok(*state.get_extension::<ConfidentialTransferAccount>()?)
}

// an ATA isn't sized for optional extensions, so make room first. without this
// ConfigureAccount just says InvalidAccountData.
pub async fn make_room(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
) -> anyhow::Result<()> {
    let ix = spl_token_2022_interface::instruction::reallocate(
        &spl_token_2022_interface::id(),
        account,
        &payer.pubkey(),
        &owner.pubkey(),
        &[],
        &[ExtensionType::ConfidentialTransferAccount],
    )?;
    let _ = mint;
    send(rpc, payer, &[ix], &[payer, owner]).await
}


// one withdraw with its proofs inline is 2284 bytes and the limit is 1232. so
// each proof gets verified into a throwaway account and the instruction points
// at that instead.
async fn verify_into_context<T, U>(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    proof_instruction: ProofInstruction,
    proof_data: &T,
) -> anyhow::Result<Keypair>
where
    T: bytemuck::Pod + ZkProofData<U>,
    U: bytemuck::Pod,
{
    let context = Keypair::new();
    let space = std::mem::size_of::<ProofContextState<U>>();
    let rent = rpc.get_minimum_balance_for_rent_exemption(space).await?;

    let create = solana_system_interface::instruction::create_account(
        &payer.pubkey(),
        &context.pubkey(),
        rent,
        space as u64,
        &solana_zk_elgamal_proof_interface::id(),
    );
    let verify = proof_instruction.encode_verify_proof(
        Some(ContextStateInfo {
            context_state_account: &context.pubkey(),
            context_state_authority: &payer.pubkey(),
        }),
        proof_data,
    );

    // separate transactions, the proof data is the thing that's too big
    send(rpc, payer, &[create], &[payer, &context]).await?;
    send(rpc, payer, &[verify], &[payer]).await?;
    Ok(context)
}

async fn close_contexts(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    contexts: &[Keypair],
) -> anyhow::Result<()> {
    for c in contexts {
        let ix = close_context_state(
            ContextStateInfo {
                context_state_account: &c.pubkey(),
                context_state_authority: &payer.pubkey(),
            },
            &payer.pubkey(),
        );
        send(rpc, payer, &[ix], &[payer]).await?;
    }
    Ok(())
}

pub async fn configure(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
    keys: &Keys,
    max_pending: Option<u64>,
) -> anyhow::Result<()> {
    make_room(rpc, payer, mint, account, owner).await?;

    // built from the secret key, so only the owner can produce it
    let proof = build_pubkey_validity_proof_data(&keys.elgamal)
        .map_err(|e| anyhow::anyhow!("pubkey validity proof failed: {e}"))?;

    let ixs = ct::configure_account(
        &spl_token_2022_interface::id(),
        account,
        mint,
        &keys.ae.encrypt(0).into(),
        max_pending.unwrap_or(65536),
        &owner.pubkey(),
        &[],
        ProofLocation::InstructionOffset(1.try_into().unwrap(), &proof),
    )?;

    send(rpc, payer, &ixs, &[payer, owner]).await
}

// only needed because this mint is auto_approve_new_accounts: false.
pub async fn approve(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    ct_authority: &Keypair,
) -> anyhow::Result<()> {
    let ix = ct::approve_account(
        &spl_token_2022_interface::id(),
        account,
        mint,
        &ct_authority.pubkey(),
        &[],
    )?;
    send(rpc, payer, &[ix], &[payer, ct_authority]).await
}

pub async fn deposit(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
    amount: u64,
) -> anyhow::Result<()> {
    let ix = ct::deposit(
        &spl_token_2022_interface::id(),
        account,
        mint,
        amount,
        DECIMALS,
        &owner.pubkey(),
        &[],
    )?;
    send(rpc, payer, &[ix], &[payer, owner]).await
}

pub async fn apply_pending(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
    keys: &Keys,
) -> anyhow::Result<()> {
    let ct = ct_account(rpc, account).await?;
    let info = ApplyPendingBalanceAccountInfo::new(&ct);

    let expected = info.pending_balance_credit_counter();
    let new_balance = info
        .new_decryptable_available_balance(keys.elgamal.secret(), &keys.ae)
        .map_err(|e| anyhow::anyhow!("couldn't work out the new balance: {e}"))?;

    let ix = ct::apply_pending_balance(
        &spl_token_2022_interface::id(),
        account,
        expected,
        &new_balance.into(),
        &owner.pubkey(),
        &[],
    )?;
    let _ = mint;
    send(rpc, payer, &[ix], &[payer, owner]).await
}

pub async fn transfer(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    from: &Address,
    to: &Address,
    owner: &Keypair,
    keys: &Keys,
    to_elgamal: &ElGamalPubkey,
    amount: u64,
) -> anyhow::Result<()> {
    let ct = ct_account(rpc, from).await?;
    let info = TransferAccountInfo::new(&ct);

    // equality, ciphertext validity, range
    let proofs = info
        .generate_split_transfer_proof_data(amount, &keys.elgamal, &keys.ae, to_elgamal, None)
        .map_err(|e| anyhow::anyhow!("transfer proof generation failed: {e}"))?;

    let new_balance = info
        .new_decryptable_available_balance(amount, &keys.ae)
        .map_err(|e| anyhow::anyhow!("couldn't work out the new balance: {e}"))?;

    let equality = verify_into_context(
        rpc,
        payer,
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
    )
    .await?;
    let validity = verify_into_context(
        rpc,
        payer,
        ProofInstruction::VerifyBatchedGroupedCiphertext3HandlesValidity,
        &proofs.ciphertext_validity_proof_data_with_ciphertext.proof_data,
    )
    .await?;
    let range = verify_into_context(
        rpc,
        payer,
        ProofInstruction::VerifyBatchedRangeProofU128,
        &proofs.range_proof_data,
    )
    .await?;

    let ixs = ct::transfer(
        &spl_token_2022_interface::id(),
        from,
        mint,
        to,
        &new_balance.into(),
        &proofs.ciphertext_validity_proof_data_with_ciphertext.ciphertext_lo,
        &proofs.ciphertext_validity_proof_data_with_ciphertext.ciphertext_hi,
        &owner.pubkey(),
        &[],
        ProofLocation::ContextStateAccount(&equality.pubkey()),
        ProofLocation::ContextStateAccount(&validity.pubkey()),
        ProofLocation::ContextStateAccount(&range.pubkey()),
    )?;

    send(rpc, payer, &ixs, &[payer, owner]).await?;
    close_contexts(rpc, payer, &[equality, validity, range]).await
}

pub async fn withdraw(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    account: &Address,
    owner: &Keypair,
    keys: &Keys,
    amount: u64,
) -> anyhow::Result<()> {
    let ct = ct_account(rpc, account).await?;
    let info = WithdrawAccountInfo::new(&ct);

    let proofs = info
        .generate_proof_data(amount, &keys.elgamal, &keys.ae)
        .map_err(|e| anyhow::anyhow!("withdraw proof generation failed, is the balance applied? {e}"))?;

    let new_balance = info
        .new_decryptable_available_balance(amount, &keys.ae)
        .map_err(|e| anyhow::anyhow!("couldn't work out the new balance: {e}"))?;

    let equality = verify_into_context(
        rpc,
        payer,
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
    )
    .await?;
    let range = verify_into_context(
        rpc,
        payer,
        ProofInstruction::VerifyBatchedRangeProofU64,
        &proofs.range_proof_data,
    )
    .await?;

    let ixs = ct::withdraw(
        &spl_token_2022_interface::id(),
        account,
        mint,
        amount,
        DECIMALS,
        &new_balance.into(),
        &owner.pubkey(),
        &[],
        ProofLocation::ContextStateAccount(&equality.pubkey()),
        ProofLocation::ContextStateAccount(&range.pubkey()),
    )?;

    send(rpc, payer, &ixs, &[payer, owner]).await?;
    close_contexts(rpc, payer, &[equality, range]).await
}

pub async fn pending_counter(rpc: &Arc<RpcClient>, account: &Address) -> anyhow::Result<u64> {
    Ok(ct_account(rpc, account).await?.pending_balance_credit_counter.into())
}

pub async fn available_balance(
    rpc: &Arc<RpcClient>,
    account: &Address,
    keys: &Keys,
) -> anyhow::Result<u64> {
    let ct = ct_account(rpc, account).await?;
    keys.ae
        .decrypt(&ct.decryptable_available_balance.try_into()?)
        .ok_or_else(|| anyhow::anyhow!("couldn't decrypt the available balance"))
}

pub async fn is_approved(rpc: &Arc<RpcClient>, account: &Address) -> anyhow::Result<bool> {
    Ok(bool::from(ct_account(rpc, account).await?.approved))
}

pub fn _mint_marker(_: &Mint) {}
