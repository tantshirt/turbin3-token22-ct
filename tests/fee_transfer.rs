// fee boundaries. each one says which comparison it pinned.

use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use turbin3_token22_ct::{
    fee_mint::{self, FeeMintConfig},
    ata, create_ata, fee_transfer, funded_payer, inspect, kyc, mint_to, rpc, DECIMALS,
};

const BPS: u16 = 500; // 5 percent
const MAX_FEE: u64 = 1_000;

// a mint plus two thawed accounts with tokens in the first one.
async fn setup(amount: u64) -> (Keypair, Address, Keypair, Address, Keypair, Address) {
    let rpc = rpc();
    let payer = funded_payer().await;
    let mint_kp = Keypair::new();
    let mint = fee_mint::create(
        &rpc,
        &payer,
        &mint_kp,
        &FeeMintConfig { fee_basis_points: BPS, maximum_fee: MAX_FEE },
    )
    .await
    .unwrap();

    let alice = Keypair::new();
    let bob = Keypair::new();
    let alice_ata = ata(&mint, &alice.pubkey());
    let bob_ata = ata(&mint, &bob.pubkey());
    create_ata(&rpc, &payer, &mint, &alice.pubkey()).await.unwrap();
    create_ata(&rpc, &payer, &mint, &bob.pubkey()).await.unwrap();

    // born frozen, so KYC both before anything moves
    kyc::approve_kyc(&rpc, &payer, &mint, &alice_ata, &payer).await.unwrap();
    kyc::approve_kyc(&rpc, &payer, &mint, &bob_ata, &payer).await.unwrap();

    mint_to(&rpc, &payer, &mint, &alice_ata, &payer, amount).await.unwrap();

    (payer, mint, alice, alice_ata, bob, bob_ata)
}

#[tokio::test]
async fn transfer_charges_the_epoch_fee() {
    let rpc = rpc();
    let (payer, mint, alice, alice_ata, _bob, bob_ata) = setup(100_000).await;

    let amount = 10_000;
    let (quoted, net) = fee_transfer::quote(&rpc, &mint, amount).await.unwrap();
    assert_eq!(quoted, 500, "5 percent of 10000");
    assert_eq!(net, 9_500);

    let charged =
        fee_transfer::send_with_fee(&rpc, &payer, &mint, &alice_ata, &bob_ata, &alice, amount)
            .await
            .unwrap();
    assert_eq!(charged, quoted);

    // the fee sits withheld on the destination until the issuer sweeps it
    assert_eq!(inspect::balance(&rpc, &bob_ata).await.unwrap(), 9_500);
    assert_eq!(inspect::balance(&rpc, &alice_ata).await.unwrap(), 90_000);
}

#[tokio::test]
async fn maximum_fee_caps_the_percentage() {
    let rpc = rpc();
    let (_payer, mint, _a, _aa, _b, _bb) = setup(1).await;

    // under the cap: plain 5 percent
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 10_000).await.unwrap(), 500);

    // exactly at the cap
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 20_000).await.unwrap(), MAX_FEE);

    // one over. cmp::min, so it binds strictly above maximum_fee.
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 20_001).await.unwrap(), MAX_FEE);
}

#[tokio::test]
async fn fee_rounds_up_never_to_zero() {
    let rpc = rpc();
    let (_payer, mint, _a, _aa, _b, _bb) = setup(1).await;

    // zero short circuits before any math
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 0).await.unwrap(), 0);

    // ceil_div not floor. if it floored you could move money one unit at a time
    // for free.
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 1).await.unwrap(), 1);

    // first amount where ceil and floor agree
    assert_eq!(inspect::epoch_fee(&rpc, &mint, 20).await.unwrap(), 1);
}

#[tokio::test]
async fn wrong_fee_is_rejected_both_directions() {
    let rpc = rpc();
    let (payer, mint, alice, alice_ata, _bob, bob_ata) = setup(100_000).await;

    let amount = 10_000;
    let right = inspect::epoch_fee(&rpc, &mint, amount).await.unwrap();

    // the check is `!=`, so you can't overpay either
    for wrong in [right - 1, right + 1] {
        let err = fee_transfer::send_with_explicit_fee(
            &rpc, &payer, &mint, &alice_ata, &bob_ata, &alice, amount, wrong,
        )
        .await;
        assert!(err.is_err(), "fee {wrong} should have been rejected");
    }

    // and the exact one goes through
    fee_transfer::send_with_explicit_fee(
        &rpc, &payer, &mint, &alice_ata, &bob_ata, &alice, amount, right,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn plain_transfer_checked_hides_the_fee() {
    let rpc = rpc();
    let (payer, mint, alice, alice_ata, _bob, bob_ata) = setup(100_000).await;

    // transfer_checked isn't rejected on a fee mint. it goes through and takes
    // the fee anyway, so the caller never had to know there was one.
    fee_transfer::send_plain(&rpc, &payer, &mint, &alice_ata, &bob_ata, &alice, 10_000)
        .await
        .unwrap();
    assert_eq!(inspect::balance(&rpc, &bob_ata).await.unwrap(), 9_500);

    // with_fee makes you state the number and fails if it's wrong. same money
    // moves, but a stale rate gets caught here.
    let fee = inspect::epoch_fee(&rpc, &mint, 10_000).await.unwrap();
    fee_transfer::send_with_explicit_fee(
        &rpc, &payer, &mint, &alice_ata, &bob_ata, &alice, 10_000, fee,
    )
    .await
    .unwrap();
    assert_eq!(inspect::balance(&rpc, &bob_ata).await.unwrap(), 19_000);
    let _ = DECIMALS;
}
