// has to be a second mint. the extension list is locked in at InitializeMint so
// confidential transfers can't be added to mint A after the fact.
//
// no transfer fee on this one. a mint with both also needs
// ConfidentialTransferFeeConfig, which is two extra proofs per transfer.

use std::sync::Arc;

use solana_address::Address;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use spl_token_2022_interface::{
    extension::{
        confidential_transfer::instruction as ct, default_account_state, metadata_pointer,
        ExtensionType,
    },
    instruction::{initialize_mint2, initialize_mint_close_authority, initialize_permanent_delegate},
    state::{AccountState, Mint},
};

use crate::{send, DECIMALS};

pub const EXTENSIONS: [ExtensionType; 5] = [
    ExtensionType::MetadataPointer,
    ExtensionType::DefaultAccountState,
    ExtensionType::MintCloseAuthority,
    ExtensionType::PermanentDelegate,
    ExtensionType::ConfidentialTransferMint,
];

pub async fn create(
    rpc: &Arc<RpcClient>,
    payer: &Keypair,
    mint: &Keypair,
    seizer: &Address,
) -> anyhow::Result<Address> {
    let authority = payer.pubkey();

    let space = ExtensionType::try_calculate_account_len::<Mint>(&EXTENSIONS)?;
    let rent = rpc.get_minimum_balance_for_rent_exemption(space).await?;

    // same rule as mint A. every extension init first, InitializeMint last.
    let instructions = vec![
        solana_system_interface::instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            rent,
            space as u64,
            &spl_token_2022_interface::id(),
        ),
        metadata_pointer::instruction::initialize(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(authority),
            Some(mint.pubkey()),
        )?,
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
        // moves tokens out of any account without asking the owner
        initialize_permanent_delegate(&spl_token_2022_interface::id(), &mint.pubkey(), seizer)?,
        // false here is approve_policy = manual
        ct::initialize_mint(
            &spl_token_2022_interface::id(),
            &mint.pubkey(),
            Some(authority),
            false,
            None,
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
