// tasks 5 and 6. the re-issued mint and the full confidential lifecycle.

use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_token_2022_interface::{
    extension::{
        confidential_transfer::ConfidentialTransferMint, permanent_delegate::PermanentDelegate,
        BaseStateWithExtensions, StateWithExtensions,
    },
    state::Mint,
};
use turbin3_token22_ct::{
    confidential::{self, Keys},
    ata, create_ata, ct_mint, fee_transfer, funded_payer, inspect, kyc, mint_to, rpc, DECIMALS,
};

struct World {
    payer: Keypair,
    mint: Address,
    seizer: Keypair,
    alice: Keypair,
    alice_ata: Address,
    alice_keys: Keys,
    bob: Keypair,
    bob_ata: Address,
    bob_keys: Keys,
}

// mint B, two configured and approved accounts, alice holding public tokens.
async fn world(alice_tokens: u64, max_pending: Option<u64>) -> World {
    let rpc = rpc();
    let payer = funded_payer().await;
    let seizer = Keypair::new();
    let mint_kp = Keypair::new();
    let mint = ct_mint::create(&rpc, &payer, &mint_kp, &seizer.pubkey()).await.unwrap();

    let alice = Keypair::new();
    let bob = Keypair::new();

    let alice_ata = ata(&mint, &alice.pubkey());
    let bob_ata = ata(&mint, &bob.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();
    create_ata(&rpc, &payer, &mint, &bob.pubkey()).await.unwrap();

    kyc::approve_kyc(&rpc, &payer, &mint, &alice_ata, &payer).await.unwrap();
    kyc::approve_kyc(&rpc, &payer, &mint, &bob_ata, &payer).await.unwrap();

    let alice_keys = confidential::keys(&alice, &alice_ata).unwrap();
    let bob_keys = confidential::keys(&bob, &bob_ata).unwrap();

    for (ata, owner, keys) in [
        (&alice_ata, &alice, &alice_keys),
        (&bob_ata, &bob, &bob_keys),
    ] {
        confidential::configure(&rpc, &payer, &mint, ata, owner, keys, max_pending)
            .await
            .unwrap();
        confidential::approve(&rpc, &payer, &mint, ata, &payer).await.unwrap();
    }

    if alice_tokens > 0 {
        mint_to(&rpc, &payer, &mint, &alice_ata, &payer, alice_tokens).await.unwrap();
    }

    World { payer, mint, seizer, alice, alice_ata, alice_keys, bob, bob_ata, bob_keys }
}

#[tokio::test]
async fn reissued_mint_carries_seizure_and_manual_approve() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let seizer = Keypair::new();
    let mint_kp = Keypair::new();
    let mint = ct_mint::create(&rpc, &payer, &mint_kp, &seizer.pubkey()).await.unwrap();

    let raw = rpc.get_account(&mint).await.unwrap();
    let state = StateWithExtensions::<Mint>::unpack(&raw.data).unwrap();

    for wanted in ct_mint::EXTENSIONS {
        assert!(state.get_extension_types().unwrap().contains(&wanted), "missing {wanted:?}");
    }

    let delegate = state.get_extension::<PermanentDelegate>().unwrap();
    let who: Option<Address> = delegate.delegate.into();
    assert_eq!(who, Some(seizer.pubkey()));

    // approve_policy = manual. this is the flag that means it.
    let ct = state.get_extension::<ConfidentialTransferMint>().unwrap();
    assert!(!bool::from(ct.auto_approve_new_accounts));
}

#[tokio::test]
async fn anyone_creates_the_ata_but_only_the_owner_configures_it() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let seizer = Keypair::new();
    let mint_kp = Keypair::new();
    let mint = ct_mint::create(&rpc, &payer, &mint_kp, &seizer.pubkey()).await.unwrap();

    let alice = Keypair::new();
    let ata = ata(&mint, &alice.pubkey());

    // payer makes alice's account and alice never signs anything
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();
    assert!(rpc.get_account(&ata).await.is_ok());

    // but the payer can't configure it. the ElGamal key comes out of the owner
    // signing, so there is nothing the payer could even put in there.
    let payer_keys = confidential::keys(&payer, &ata).unwrap();
    assert!(
        confidential::configure(&rpc, &payer, &mint, &ata, &payer, &payer_keys, None)
            .await
            .is_err(),
        "payer should not be able to configure somebody else's account"
    );

    let alice_keys = confidential::keys(&alice, &ata).unwrap();
    confidential::configure(&rpc, &payer, &mint, &ata, &alice, &alice_keys, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn manual_approve_gates_the_account() {
    let rpc = rpc();
    let payer = funded_payer().await;
    let seizer = Keypair::new();
    let mint_kp = Keypair::new();
    let mint = ct_mint::create(&rpc, &payer, &mint_kp, &seizer.pubkey()).await.unwrap();

    let alice = Keypair::new();
    let ata = ata(&mint, &alice.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();
    kyc::approve_kyc(&rpc, &payer, &mint, &ata, &payer).await.unwrap();

    let keys = confidential::keys(&alice, &ata).unwrap();
    confidential::configure(&rpc, &payer, &mint, &ata, &alice, &keys, None).await.unwrap();
    mint_to(&rpc, &payer, &mint, &ata, &payer, 1_000).await.unwrap();

    // configured but not approved, so the confidential side is closed
    assert!(
        confidential::deposit(&rpc, &payer, &mint, &ata, &alice, 500).await.is_err(),
        "unapproved account should not be able to deposit"
    );

    confidential::approve(&rpc, &payer, &mint, &ata, &payer).await.unwrap();
    confidential::deposit(&rpc, &payer, &mint, &ata, &alice, 500).await.unwrap();
}

#[tokio::test]
async fn deposit_lands_in_pending_and_apply_moves_it() {
    let rpc = rpc();
    let w = world(1_000, None).await;

    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 600)
        .await
        .unwrap();

    // public balance went down straight away
    assert_eq!(inspect::balance(&rpc, &w.alice_ata).await.unwrap(), 400);

    // but it's in pending, not available. available is still zero.
    assert_eq!(
        confidential::pending_counter(&rpc, &w.alice_ata).await.unwrap(),
        1
    );
    assert_eq!(
        confidential::available_balance(&rpc, &w.alice_ata, &w.alice_keys)
            .await
            .unwrap(),
        0
    );

    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();

    // now it's spendable, and the counter reset
    assert_eq!(
        confidential::available_balance(&rpc, &w.alice_ata, &w.alice_keys)
            .await
            .unwrap(),
        600
    );
    assert_eq!(
        confidential::pending_counter(&rpc, &w.alice_ata).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn pending_credit_counter_boundary() {
    let rpc = rpc();
    // cap it at 2 so the boundary is cheap to hit
    let w = world(1_000, Some(2)).await;

    // one under the cap
    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 100).await.unwrap();
    assert_eq!(
        confidential::pending_counter(&rpc, &w.alice_ata).await.unwrap(),
        1
    );

    // exactly at the cap. the check on chain is `new > maximum`, so landing on
    // the maximum is fine. this is the one I had backwards at first.
    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 100).await.unwrap();
    assert_eq!(
        confidential::pending_counter(&rpc, &w.alice_ata).await.unwrap(),
        2
    );

    // one over and it's rejected
    assert!(
        confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 100)
            .await
            .is_err(),
        "third deposit should blow the counter"
    );

    // apply is the release valve, not a workaround
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();
    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 100).await.unwrap();
}

#[tokio::test]
async fn full_lifecycle_deposit_transfer_withdraw() {
    let rpc = rpc();
    let w = world(1_000, None).await;

    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 800).await.unwrap();
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();

    confidential::transfer(
        &rpc,
        &w.payer,
        &w.mint,
        &w.alice_ata,
        &w.bob_ata,
        &w.alice,
        &w.alice_keys,
        w.bob_keys.elgamal.pubkey(),
        300,
    )
    .await
    .unwrap();

    // nothing public moved. that's the whole point.
    assert_eq!(inspect::balance(&rpc, &w.alice_ata).await.unwrap(), 200);
    assert_eq!(inspect::balance(&rpc, &w.bob_ata).await.unwrap(), 0);

    assert_eq!(
        confidential::available_balance(&rpc, &w.alice_ata, &w.alice_keys)
            .await
            .unwrap(),
        500
    );

    // bob's 300 is in pending. he has to apply before he can do anything.
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.bob_ata, &w.bob, &w.bob_keys)
        .await
        .unwrap();
    assert_eq!(
        confidential::available_balance(&rpc, &w.bob_ata, &w.bob_keys)
            .await
            .unwrap(),
        300
    );

    confidential::withdraw(&rpc, &w.payer, &w.mint, &w.bob_ata, &w.bob, &w.bob_keys, 300)
        .await
        .unwrap();

    // back out in public where anyone can see it
    assert_eq!(inspect::balance(&rpc, &w.bob_ata).await.unwrap(), 300);
    let _ = DECIMALS;
}

#[tokio::test]
async fn withdraw_before_apply_pending_fails() {
    let rpc = rpc();
    let w = world(1_000, None).await;

    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 500).await.unwrap();

    // the money is visibly in the account and still not spendable. withdraw
    // builds its proof off the AVAILABLE ciphertext, which is still zero.
    assert!(
        confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 500)
            .await
            .is_err(),
        "withdraw should fail before apply_pending"
    );

    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();
    confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 500)
        .await
        .unwrap();
    assert_eq!(inspect::balance(&rpc, &w.alice_ata).await.unwrap(), 1_000);
}

#[tokio::test]
async fn withdraw_boundary() {
    let rpc = rpc();
    let w = world(1_000, None).await;

    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 500).await.unwrap();
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();

    // one over available. fails client side in proof generation, because there
    // is no valid range proof for a negative remainder.
    assert!(
        confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 501)
            .await
            .is_err()
    );

    // one under is fine
    confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 499)
        .await
        .unwrap();

    // and exactly what's left is fine too
    confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 1)
        .await
        .unwrap();
    assert_eq!(
        confidential::available_balance(&rpc, &w.alice_ata, &w.alice_keys)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn permanent_delegate_seizes_public_but_not_confidential() {
    let rpc = rpc();
    let w = world(1_000, None).await;
    

    // baseline: the seizure authority can take public tokens without asking
    fee_transfer::send_plain(
        &rpc, &w.payer, &w.mint, &w.alice_ata, &w.bob_ata, &w.seizer, 200,
    )
    .await
    .unwrap();
    assert_eq!(inspect::balance(&rpc, &w.alice_ata).await.unwrap(), 800);

    // now alice moves everything into the confidential system
    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 800).await.unwrap();
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();
    assert_eq!(inspect::balance(&rpc, &w.alice_ata).await.unwrap(), 0);

    // this is task 8, as a test. the permanent delegate has no instruction that
    // reaches a ciphertext, and there is nothing public left to take.
    assert!(
        fee_transfer::send_plain(
            &rpc, &w.payer, &w.mint, &w.alice_ata, &w.bob_ata, &w.seizer, 1,
        )
        .await
        .is_err(),
        "seizure should be defeated once the balance is confidential"
    );

    // the 800 is still right there, alice just has it somewhere the issuer can't
    assert_eq!(
        confidential::available_balance(&rpc, &w.alice_ata, &w.alice_keys)
            .await
            .unwrap(),
        800
    );
}

#[tokio::test]
async fn freezing_shuts_the_whole_confidential_path() {
    let rpc = rpc();
    let w = world(1_000, None).await;

    confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 500).await.unwrap();
    confidential::apply_pending(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys)
        .await
        .unwrap();

    kyc::revoke_kyc(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.payer).await.unwrap();

    // freeze is the lever that still works. it contains the money, it doesn't
    // hand it over, which is the difference the writeup is about.
    assert!(confidential::deposit(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, 100).await.is_err());
    assert!(
        confidential::transfer(
            &rpc, &w.payer, &w.mint, &w.alice_ata, &w.bob_ata, &w.alice, &w.alice_keys,
            w.bob_keys.elgamal.pubkey(), 100,
        )
        .await
        .is_err()
    );
    assert!(
        confidential::withdraw(&rpc, &w.payer, &w.mint, &w.alice_ata, &w.alice, &w.alice_keys, 100)
            .await
            .is_err()
    );
}
