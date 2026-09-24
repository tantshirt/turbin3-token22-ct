// task 1. the remittance mint: a fee on every transfer, metadata on the mint
// itself, new accounts frozen until KYC, and a way to close the mint later.
//
// built out of raw instructions on purpose. a client helper would do the same
// thing, but it does it inside the crate where you can't see the sizing or the
// ordering, and those two things are what this task is actually about.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use spl_token_2022_interface::{
    extension::{default_account_state, metadata_pointer, transfer_fee, ExtensionType},
    instruction::{initialize_mint2, initialize_mint_close_authority},
    state::{AccountState, Mint},
};

use crate::{send, DECIMALS};

pub struct FeeMintConfig {
    pub fee_basis_points: u16,
    pub maximum_fee: u64,
}

// every authority is the payer here. a real issuer would split these across
// separate keys, but that isn't what's graded and it would double the setup.
pub async fn create(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Keypair,
    config: &FeeMintConfig,
) -> anyhow::Result<Address> {
    let authority = payer.pubkey();

    // TokenMetadata is deliberately not in this list. it's variable length, so
    // try_calculate_account_len refuses it, and it gets written in a second
    // transaction that reallocs the mint.
    let extensions = [
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
        ExtensionType::DefaultAccountState,
        ExtensionType::MintCloseAuthority,
    ];

    // the whole sizing story. don't hand compute 82 plus the TLV records, this
    // also pads a mint that would otherwise land exactly on Multisig::LEN.
    let space = ExtensionType::try_calculate_account_len::<Mint>(&extensions)?;
    let rent = rpc.get_minimum_balance_for_rent_exemption(space).await?;

    // order matters and it isn't style. InitializeMint stamps the account type
    // byte, and after that every extension init sees an initialized mint and
    // refuses. so all four go first and InitializeMint goes last.
    let instructions = vec![
        solana_system_interface::instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            rent,
            space as u64,
            &spl_token_2022_interface::id(),
        ),
        transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(&authority),
            Some(&authority),
            config.fee_basis_points,
            config.maximum_fee,
        )?,
        // pointed at the mint itself, so a wallet reads the name and symbol off
        // the same account and never has to trust an off chain list.
        metadata_pointer::instruction::initialize(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(authority),
            Some(mint.pubkey()),
        )?,
        // every new account lands frozen. that's the KYC gate.
        default_account_state::instruction::initialize_default_account_state(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            &AccountState::Frozen,
        )?,
        initialize_mint_close_authority(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(&authority),
        )?,
        initialize_mint2(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            &authority,
            Some(&authority),
            DECIMALS,
        )?,
    ];

    send(rpc, payer, &instructions, &[payer, mint]).await?;
    Ok(mint.pubkey())
}

// second transaction, on purpose. TokenMetadata is variable length so
// try_calculate_account_len won't size it, which means the mint has to grow and
// get topped up on rent after it already exists.
pub async fn write_metadata(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Address,
    name: &str,
    symbol: &str,
    uri: &str,
) -> anyhow::Result<()> {
    let metadata = spl_token_metadata_interface::state::TokenMetadata {
        update_authority: Some(payer.pubkey())
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad update authority"))?,
        mint: *mint,
        name: name.to_string(),
        symbol: symbol.to_string(),
        uri: uri.to_string(),
        additional_metadata: vec![],
    };

    // the mint is already rent exempt for its fixed extensions, so we only have
    // to cover what the metadata adds on top.
    let extra = metadata.tlv_size_of()?;
    let current = rpc.get_account(mint).await?;
    let needed = rpc
        .get_minimum_balance_for_rent_exemption(current.data.len() + extra)
        .await?;
    let top_up = needed.saturating_sub(current.lamports);

    let mut instructions = vec![];
    if top_up > 0 {
        instructions.push(solana_system_interface::instruction::transfer(
            &payer.pubkey(),
            mint,
            top_up,
        ));
    }
    instructions.push(spl_token_metadata_interface::instruction::initialize(
        &spl_token_2022_interface::id(),
        mint,
        &payer.pubkey(),
        mint,
        &payer.pubkey(),
        name.to_string(),
        symbol.to_string(),
        uri.to_string(),
    ));

    send(rpc, payer, &instructions, &[payer]).await
}

// same six instructions, but InitializeMint moved up to position two. only
// exists so a test can prove the ordering rule is real and not cargo cult.
pub async fn create_in_wrong_order(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Keypair,
) -> anyhow::Result<()> {
    let authority = payer.pubkey();
    let extensions = [
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
        ExtensionType::DefaultAccountState,
        ExtensionType::MintCloseAuthority,
    ];
    let space = ExtensionType::try_calculate_account_len::<Mint>(&extensions)?;
    let rent = rpc.get_minimum_balance_for_rent_exemption(space).await?;

    let instructions = vec![
        solana_system_interface::instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            rent,
            space as u64,
            &spl_token_2022_interface::id(),
        ),
        initialize_mint2(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            &authority,
            Some(&authority),
            DECIMALS,
        )?,
        transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(&authority),
            Some(&authority),
            500,
            1_000,
        )?,
    ];

    send(rpc, payer, &instructions, &[payer, mint]).await
}
