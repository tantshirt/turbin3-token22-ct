// copied from spl-token-client 0.19.1, src/zk_proofs/confidential_transfer.rs.
// that crate won't build at any published version (spl-memo-interface 2.1 wants
// solana-instruction 3.4, its own source wants solana-transaction 3.x, nothing
// satisfies both) and these used to live in spl-token-2022 before they moved.
// all they do is hand the account's ciphertexts to the proof generator.

#![allow(dead_code)]

use {
    solana_zk_sdk::{
        encryption::{
            auth_encryption::{AeCiphertext, AeKey},
            elgamal::{ElGamalKeypair, ElGamalPubkey, ElGamalSecretKey},
        },
    },
    spl_token_2022_interface::{
        error::TokenError,
        extension::confidential_transfer::{
            ConfidentialTransferAccount, DecryptableBalance, EncryptedBalance,
            PENDING_BALANCE_LO_BIT_LENGTH,
        },
    },
    spl_token_confidential_transfer_proof_generation::{
        transfer::{transfer_split_proof_data, TransferProofData},
        withdraw::{withdraw_proof_data, WithdrawProofData},
    },
};

/// Confidential Transfer extension information needed to construct an
/// `ApplyPendingBalance` instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApplyPendingBalanceAccountInfo {
    /// The total number of `Deposit` and `Transfer` instructions that have
    /// credited `pending_balance`
    pub(crate) pending_balance_credit_counter: u64,
    /// The low 16 bits of the pending balance (encrypted by `elgamal_pubkey`)
    pub(crate) pending_balance_lo: EncryptedBalance,
    /// The high 32 bits of the pending balance (encrypted by `elgamal_pubkey`)
    pub(crate) pending_balance_hi: EncryptedBalance,
    /// The decryptable available balance
    pub(crate) decryptable_available_balance: DecryptableBalance,
}
impl ApplyPendingBalanceAccountInfo {
    /// Create the `ApplyPendingBalance` instruction account information from
    /// `ConfidentialTransferAccount`.
    pub fn new(account: &ConfidentialTransferAccount) -> Self {
        Self {
            pending_balance_credit_counter: account.pending_balance_credit_counter.into(),
            pending_balance_lo: account.pending_balance_lo,
            pending_balance_hi: account.pending_balance_hi,
            decryptable_available_balance: account.decryptable_available_balance,
        }
    }

    /// Return the pending balance credit counter of the account.
    pub fn pending_balance_credit_counter(&self) -> u64 {
        self.pending_balance_credit_counter
    }

    fn decrypted_pending_balance_lo(
        &self,
        elgamal_secret_key: &ElGamalSecretKey,
    ) -> Result<u64, TokenError> {
        let pending_balance_lo = self
            .pending_balance_lo
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        elgamal_secret_key
            .decrypt_u32(&pending_balance_lo)
            .ok_or(TokenError::AccountDecryption)
    }

    fn decrypted_pending_balance_hi(
        &self,
        elgamal_secret_key: &ElGamalSecretKey,
    ) -> Result<u64, TokenError> {
        let pending_balance_hi = self
            .pending_balance_hi
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        elgamal_secret_key
            .decrypt_u32(&pending_balance_hi)
            .ok_or(TokenError::AccountDecryption)
    }

    fn decrypted_available_balance(&self, aes_key: &AeKey) -> Result<u64, TokenError> {
        let decryptable_available_balance = self
            .decryptable_available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        aes_key
            .decrypt(&decryptable_available_balance)
            .ok_or(TokenError::AccountDecryption)
    }

    /// Update the decryptable available balance.
    pub fn new_decryptable_available_balance(
        &self,
        elgamal_secret_key: &ElGamalSecretKey,
        aes_key: &AeKey,
    ) -> Result<AeCiphertext, TokenError> {
        let decrypted_pending_balance_lo = self.decrypted_pending_balance_lo(elgamal_secret_key)?;
        let decrypted_pending_balance_hi = self.decrypted_pending_balance_hi(elgamal_secret_key)?;
        let pending_balance =
            combine_balances(decrypted_pending_balance_lo, decrypted_pending_balance_hi)
                .ok_or(TokenError::AccountDecryption)?;
        let current_available_balance = self.decrypted_available_balance(aes_key)?;
        let new_decrypted_available_balance = current_available_balance
            .checked_add(pending_balance)
            .unwrap(); // total balance cannot exceed `u64`

        Ok(aes_key.encrypt(new_decrypted_available_balance))
    }

    /// Decrypt and return the pending balance for this account.
    ///
    /// This combines the low 16 bits and high 48 bits of the pending balance
    /// into a single u64 value.
    pub fn get_pending_balance(
        &self,
        elgamal_secret_key: &ElGamalSecretKey,
    ) -> Result<u64, TokenError> {
        let decrypted_lo = self.decrypted_pending_balance_lo(elgamal_secret_key)?;
        let decrypted_hi = self.decrypted_pending_balance_hi(elgamal_secret_key)?;

        combine_balances(decrypted_lo, decrypted_hi).ok_or(TokenError::AccountDecryption)
    }

    /// Check if this account has any pending balance.
    pub fn has_pending_balance(&self) -> bool {
        self.pending_balance_credit_counter > 0
    }

    /// Get the available balance for this account.
    pub fn get_available_balance(&self, aes_key: &AeKey) -> Result<u64, TokenError> {
        self.decrypted_available_balance(aes_key)
    }

    /// Get the total balance (pending and available) for this account.
    pub fn get_total_balance(
        &self,
        elgamal_secret_key: &ElGamalSecretKey,
        aes_key: &AeKey,
    ) -> Result<u64, TokenError> {
        let pending = self.get_pending_balance(elgamal_secret_key)?;
        let available = self.get_available_balance(aes_key)?;

        pending.checked_add(available).ok_or(TokenError::Overflow)
    }
}

/// Confidential Transfer extension information needed to construct a `Withdraw`
/// instruction.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WithdrawAccountInfo {
    /// The available balance (encrypted by `encryption_pubkey`)
    pub available_balance: EncryptedBalance,
    /// The decryptable available balance
    pub decryptable_available_balance: DecryptableBalance,
}
impl WithdrawAccountInfo {
    /// Create the `Withdraw` instruction account information from
    /// `ConfidentialTransferAccount`.
    pub fn new(account: &ConfidentialTransferAccount) -> Self {
        Self {
            available_balance: account.available_balance,
            decryptable_available_balance: account.decryptable_available_balance,
        }
    }

    fn decrypted_available_balance(&self, aes_key: &AeKey) -> Result<u64, TokenError> {
        let decryptable_available_balance = self
            .decryptable_available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        aes_key
            .decrypt(&decryptable_available_balance)
            .ok_or(TokenError::AccountDecryption)
    }

    /// Create a withdraw proof data.
    pub fn generate_proof_data(
        &self,
        withdraw_amount: u64,
        elgamal_keypair: &ElGamalKeypair,
        aes_key: &AeKey,
    ) -> Result<WithdrawProofData, TokenError> {
        let current_available_balance = self
            .available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        let current_decrypted_available_balance = self.decrypted_available_balance(aes_key)?;

        withdraw_proof_data(
            &current_available_balance,
            current_decrypted_available_balance,
            withdraw_amount,
            elgamal_keypair,
        )
        .map_err(|_| TokenError::ProofGeneration)
    }

    /// Update the decryptable available balance.
    pub fn new_decryptable_available_balance(
        &self,
        withdraw_amount: u64,
        aes_key: &AeKey,
    ) -> Result<AeCiphertext, TokenError> {
        let current_decrypted_available_balance = self.decrypted_available_balance(aes_key)?;
        let new_decrypted_available_balance = current_decrypted_available_balance
            .checked_sub(withdraw_amount)
            .ok_or(TokenError::InsufficientFunds)?;

        Ok(aes_key.encrypt(new_decrypted_available_balance))
    }
}

/// Confidential Transfer extension information needed to construct a `Transfer`
/// instruction.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransferAccountInfo {
    /// The available balance (encrypted by `encryption_pubkey`)
    pub available_balance: EncryptedBalance,
    /// The decryptable available balance
    pub decryptable_available_balance: DecryptableBalance,
}
impl TransferAccountInfo {
    /// Create the `Transfer` instruction account information from
    /// `ConfidentialTransferAccount`.
    pub fn new(account: &ConfidentialTransferAccount) -> Self {
        Self {
            available_balance: account.available_balance,
            decryptable_available_balance: account.decryptable_available_balance,
        }
    }

    fn decrypted_available_balance(&self, aes_key: &AeKey) -> Result<u64, TokenError> {
        let decryptable_available_balance = self
            .decryptable_available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        aes_key
            .decrypt(&decryptable_available_balance)
            .ok_or(TokenError::AccountDecryption)
    }

    /// Create a transfer proof data that is split into equality, ciphertext
    /// validity, and range proofs.
    pub fn generate_split_transfer_proof_data(
        &self,
        transfer_amount: u64,
        source_elgamal_keypair: &ElGamalKeypair,
        aes_key: &AeKey,
        destination_elgamal_pubkey: &ElGamalPubkey,
        auditor_elgamal_pubkey: Option<&ElGamalPubkey>,
    ) -> Result<TransferProofData, TokenError> {
        let current_available_balance = self
            .available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;
        let current_decryptable_available_balance = self
            .decryptable_available_balance
            .try_into()
            .map_err(|_| TokenError::MalformedCiphertext)?;

        transfer_split_proof_data(
            &current_available_balance,
            &current_decryptable_available_balance,
            transfer_amount,
            source_elgamal_keypair,
            aes_key,
            destination_elgamal_pubkey,
            auditor_elgamal_pubkey,
        )
        .map_err(|_| TokenError::ProofGeneration)
    }

    /// Update the decryptable available balance.
    pub fn new_decryptable_available_balance(
        &self,
        transfer_amount: u64,
        aes_key: &AeKey,
    ) -> Result<AeCiphertext, TokenError> {
        let current_decrypted_available_balance = self.decrypted_available_balance(aes_key)?;
        let new_decrypted_available_balance = current_decrypted_available_balance
            .checked_sub(transfer_amount)
            .ok_or(TokenError::InsufficientFunds)?;

        Ok(aes_key.encrypt(new_decrypted_available_balance))
    }
}


/// Combines pending balances low and high bits into singular pending balance
pub fn combine_balances(balance_lo: u64, balance_hi: u64) -> Option<u64> {
    balance_hi
        .checked_shl(PENDING_BALANCE_LO_BIT_LENGTH)?
        .checked_add(balance_lo)
}
