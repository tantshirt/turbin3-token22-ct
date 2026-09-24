// task 4. thawing one account after KYC, and proving it's a different lever from
// the mint level default state.

use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_token_2022_interface::{
    extension::{
        default_account_state::DefaultAccountState, BaseStateWithExtensions, StateWithExtensions,
    },
    state::{AccountState, Mint},
};
use turbin3_token22_ct::{
    fee_mint::{self, FeeMintConfig},
    ata, create_ata, funded_payer, inspect, kyc, mint_to, rpc,
};

async fn setup() -> (Keypair, Address) {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();
    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &FeeMintConfig { fee_basis_points: 500, maximum_fee: 1_000 },
    )
    .await
    .unwrap();
    (payer, mint)
}

#[tokio::test]
async fn new_accounts_are_born_frozen() {
    let rpc = rpc();
    let (payer, mint) = setup().await;
    let alice = Keypair::new();
    let account = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();

    // nobody sent a Freeze instruction. DefaultAccountState did it.
    assert!(inspect::is_frozen(&rpc, &account).await.unwrap());
}

#[tokio::test]
async fn frozen_then_thawed_then_frozen_again() {
    let rpc = rpc();
    let (payer, mint) = setup().await;
    let alice = Keypair::new();
    let account = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();

    // frozen: can't receive anything
    assert!(mint_to(&rpc, &payer, &mint, &account, &payer, 100).await.is_err());

    // KYC clears, freeze authority thaws this one account
    kyc::approve_kyc(&rpc, &payer, &mint, &account, &payer).await.unwrap();
    assert!(!inspect::is_frozen(&rpc, &account).await.unwrap());
    mint_to(&rpc, &payer, &mint, &account, &payer, 100).await.unwrap();
    assert_eq!(inspect::balance(&rpc, &account).await.unwrap(), 100);

    // and it can go back the other way
    kyc::revoke_kyc(&rpc, &payer, &mint, &account, &payer).await.unwrap();
    assert!(inspect::is_frozen(&rpc, &account).await.unwrap());
    assert!(mint_to(&rpc, &payer, &mint, &account, &payer, 100).await.is_err());
}

#[tokio::test]
async fn only_the_freeze_authority_can_thaw() {
    let rpc = rpc();
    let (payer, mint) = setup().await;
    let alice = Keypair::new();
    let nobody = funded_payer().await;
    let account = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();

    assert!(kyc::approve_kyc(&rpc, &payer, &mint, &account, &nobody).await.is_err());
    assert!(inspect::is_frozen(&rpc, &account).await.unwrap());
}

#[tokio::test]
async fn thawing_one_account_leaves_the_mint_alone() {
    let rpc = rpc();
    let (payer, mint) = setup().await;
    let alice = Keypair::new();
    let bob = Keypair::new();
    let alice_ata = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();
    kyc::approve_kyc(&rpc, &payer, &mint, &alice_ata, &payer).await.unwrap();

    // this is the "separate from any mint level change" clause. alice got
    // through, the mint still says frozen, and bob shows up frozen like always.
    let raw = rpc.get_account(&mint).await.unwrap();
    let state = StateWithExtensions::<Mint>::unpack(&raw.data).unwrap();
    let default_state = state.get_extension::<DefaultAccountState>().unwrap();
    assert_eq!(default_state.state, u8::from(AccountState::Frozen));

    let bob_ata = ata(&mint, &bob.pubkey());
    create_ata(&rpc, &payer, &mint, &bob.pubkey()).await.unwrap();
    assert!(inspect::is_frozen(&rpc, &bob_ata).await.unwrap());
}

#[tokio::test]
async fn changing_the_mint_default_is_not_retroactive() {
    let rpc = rpc();
    let (payer, mint) = setup().await;
    let alice = Keypair::new();
    let bob = Keypair::new();
    let alice_ata = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();

    // flip the mint so new accounts come up usable
    kyc::set_default_state(&rpc, &payer, &mint, &payer, AccountState::Initialized)
        .await
        .unwrap();

    // bob is new, so bob is fine
    let bob_ata = ata(&mint, &bob.pubkey());
    create_ata(&rpc, &payer, &mint, &bob.pubkey()).await.unwrap();
    assert!(!inspect::is_frozen(&rpc, &bob_ata).await.unwrap());

    // alice already existed, so alice is still frozen. the mint default only
    // ever decides what the NEXT account looks like.
    assert!(inspect::is_frozen(&rpc, &alice_ata).await.unwrap());
}
