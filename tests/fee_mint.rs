use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_token_2022_interface::{
    extension::{
        default_account_state::DefaultAccountState, metadata_pointer::MetadataPointer,
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
        StateWithExtensions,
    },
    state::{AccountState, Mint},
};
use spl_token_metadata_interface::state::TokenMetadata;
use turbin3_token22_ct::{fee_mint, ata, burn, close_mint, create_ata, funded_payer, mint_to, rpc, DECIMALS};

#[tokio::test]
async fn mint_has_all_four_extensions() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();

    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &fee_mint::FeeMintConfig {
            fee_basis_points: 500,
            maximum_fee: 1_000,
        },
    )
    .await
    .expect("mint creation failed");

    let account = rpc.get_account(&mint).await.unwrap();

    // task 3. read it through StateWithExtensions, never Mint::unpack.
    let state = StateWithExtensions::<Mint>::unpack(&account.data).unwrap();

    let types = state.get_extension_types().unwrap();
    for wanted in [
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
        ExtensionType::DefaultAccountState,
        ExtensionType::MintCloseAuthority,
    ] {
        assert!(types.contains(&wanted), "missing {wanted:?}");
    }

    let fee = state.get_extension::<TransferFeeConfig>().unwrap();
    assert_eq!(u16::from(fee.newer_transfer_fee.transfer_fee_basis_points), 500);
    assert_eq!(u64::from(fee.newer_transfer_fee.maximum_fee), 1_000);

    // the pointer has to name the mint itself, that's the point of task 1
    let pointer = state.get_extension::<MetadataPointer>().unwrap();
    let target: Option<Address> = pointer.metadata_address.into();
    assert_eq!(target, Some(mint));

    let default_state = state.get_extension::<DefaultAccountState>().unwrap();
    assert_eq!(default_state.state, u8::from(AccountState::Frozen));

    assert_eq!(state.base.decimals, DECIMALS);
}

#[tokio::test]
async fn space_matches_try_calculate_account_len() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();

    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &fee_mint::FeeMintConfig {
            fee_basis_points: 500,
            maximum_fee: 1_000,
        },
    )
    .await
    .unwrap();

    let expected = ExtensionType::try_calculate_account_len::<Mint>(&[
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
        ExtensionType::DefaultAccountState,
        ExtensionType::MintCloseAuthority,
    ])
    .unwrap();

    let account = rpc.get_account(&mint).await.unwrap();
    assert_eq!(account.data.len(), expected);
}

#[tokio::test]
async fn metadata_lives_on_the_mint_itself() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();

    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &fee_mint::FeeMintConfig { fee_basis_points: 500, maximum_fee: 1_000 },
    )
    .await
    .unwrap();

    let before = rpc.get_account(&mint).await.unwrap().data.len();

    fee_mint::write_metadata(&rpc, &payer, &mint, "Turbin3 Remit", "T3R", "https://example.com/t3r.json")
        .await
        .unwrap();

    // read it back off the mint account, same as a wallet would
    let raw = rpc.get_account(&mint).await.unwrap();
    let state = StateWithExtensions::<Mint>::unpack(&raw.data).unwrap();
    let meta = state.get_variable_len_extension::<TokenMetadata>().unwrap();
    assert_eq!(meta.name, "Turbin3 Remit");
    assert_eq!(meta.symbol, "T3R");
    assert_eq!(meta.mint, mint);

    // the account had to grow. that's why metadata can't ride along in the
    // create transaction with the other four.
    let after = rpc.get_account(&mint).await.unwrap().data.len();
    assert!(after > before, "mint should have been reallocated for the metadata");
}

#[tokio::test]
async fn close_authority_only_closes_an_empty_mint() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();
    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &fee_mint::FeeMintConfig { fee_basis_points: 500, maximum_fee: 1_000 },
    )
    .await
    .unwrap();
    let holder = Keypair::new();
    let holder_ata = ata(&mint, &holder.pubkey());
    create_ata(&rpc, &payer, &mint, &holder.pubkey()).await.unwrap();
    turbin3_token22_ct::kyc::approve_kyc(&rpc, &payer, &mint, &holder_ata, &payer).await.unwrap();
    mint_to(&rpc, &payer, &mint, &holder_ata, &payer, 1_000).await.unwrap();

    // supply is 1000, so decommissioning is off the table
    assert!(close_mint(&rpc, &payer, &mint, &payer).await.is_err());

    // burn it all and the mint can go
    burn(&rpc, &payer, &mint, &holder_ata, &holder, 1_000).await.unwrap();
    close_mint(&rpc, &payer, &mint, &payer).await.unwrap();
    assert!(rpc.get_account(&mint).await.is_err(), "mint should be gone");
}

#[tokio::test]
async fn extension_init_after_initialize_mint_fails() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();

    // InitializeMint stamps the account type byte at offset 165. after that the
    // extension init sees an initialized mint and refuses, so the whole
    // transaction dies. this is why the order in create() is what it is.
    assert!(
        fee_mint::create_in_wrong_order(&rpc, &payer, &mint_kp).await.is_err(),
        "initializing an extension after InitializeMint should fail"
    );
    assert!(rpc.get_account(&mint_kp.pubkey()).await.is_err(), "no mint should exist");
}
