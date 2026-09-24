// the remittance mint. raw instructions rather than a helper so the sizing and
// the instruction order are actually visible.

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

// payer is every authority. a real issuer would split these up.
pub async fn create(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Keypair,
    config: &FeeMintConfig,
) -> anyhow::Result<Address> {
    let authority = payer.pubkey();

    // no TokenMetadata here, it's variable length and goes in a second tx
    let extensions = [
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
        ExtensionType::DefaultAccountState,
        ExtensionType::MintCloseAuthority,
    ];

    // don't hand compute this, it also pads mints that collide with Multisig::LEN
    let space = ExtensionType::try_calculate_account_len::<Mint>(&extensions)?;
    let rent = rpc.get_minimum_balance_for_rent_exemption(space).await?;

    // InitializeMint stamps the account type byte and every extension init
    // refuses after that, so it goes last
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
        // points at the mint itself so wallets don't need an off chain list
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
