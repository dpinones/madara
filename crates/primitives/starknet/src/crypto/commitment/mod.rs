use alloc::vec;
use alloc::vec::Vec;

use bitvec::vec::BitVec;
use sp_core::hexdisplay::AsBytesRef;
use sp_core::{H256, U256};
use starknet_crypto::FieldElement;

use super::hash::pedersen::PedersenHasher;
use super::merkle_patricia_tree::merkle_tree::MerkleTree;
use crate::traits::hash::CryptoHasher;
use crate::transaction::types::{
    DeclareTransaction, DeployAccountTransaction, EventWrapper, InvokeTransaction, Transaction,
};

/// A Patricia Merkle tree with height 64 used to compute transaction and event commitments.
///
/// According to the [documentation](https://docs.starknet.io/docs/Blocks/header/#block-header)
/// the commitment trees are of height 64, because the key used is the 64 bit representation
/// of the index of the transaction / event within the block.
///
/// The tree height is 64 in our case since our set operation takes u64 index values.
struct CommitmentTree<T: CryptoHasher> {
    tree: MerkleTree<T>,
}

impl<T: CryptoHasher> Default for CommitmentTree<T> {
    fn default() -> Self {
        Self { tree: MerkleTree::empty() }
    }
}

impl<T: CryptoHasher> CommitmentTree<T> {
    /// Sets the value of a key in the merkle tree.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the value to set.
    /// * `value` - The value to set.
    pub fn set(&mut self, index: u64, value: FieldElement) {
        let key = index.to_be_bytes();
        self.tree.set(&BitVec::from(key.to_vec()), value)
    }

    /// Get the merkle root of the tree.
    pub fn commit(self) -> FieldElement {
        self.tree.commit()
    }
}

/// Calculate the transaction commitment, the event commitment and the event count.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
///
/// # Returns
///
/// The transaction commitment, the event commitment and the event count.
pub fn calculate_commitments<T: CryptoHasher>(transactions: &[Transaction], events: &[EventWrapper]) -> (H256, H256) {
    (calculate_transaction_commitment::<T>(transactions), calculate_event_commitment::<T>(events))
}

/// Calculate transaction commitment hash value.
///
/// The transaction commitment is the root of the Patricia Merkle tree with height 64
/// constructed by adding the (transaction_index, transaction_hash_with_signature)
/// key-value pairs to the tree and computing the root hash.
///
/// # Arguments
///
/// * `transactions` - The transactions to get the root from.
///
/// # Returns
///
/// The merkle root of the merkle tree built from the transactions.
pub fn calculate_transaction_commitment<T: CryptoHasher>(transactions: &[Transaction]) -> H256 {
    let mut tree = CommitmentTree::<T>::default();

    transactions.iter().enumerate().for_each(|(idx, tx)| {
        let idx: u64 = idx.try_into().expect("too many transactions while calculating commitment");
        let final_hash = calculate_transaction_hash_with_signature::<T>(tx);
        tree.set(idx, final_hash);
    });
    H256::from_slice(&tree.commit().to_bytes_be())
}

/// Calculate transaction commitment hash value.
///
/// The event commitment is the root of the Patricia Merkle tree with height 64
/// constructed by adding the event hash
/// (see https://docs.starknet.io/documentation/architecture_and_concepts/Events/starknet-events/#event_hash)
/// to the tree and computing the root hash.
///
/// # Arguments
///
/// * `transactions` - The transactions to get the events from.
///
/// # Returns
///
/// The merkle root of the merkle tree built from the transactions and the number of events.
pub fn calculate_event_commitment<T: CryptoHasher>(events: &[EventWrapper]) -> H256 {
    let mut tree = CommitmentTree::<T>::default();
    events.iter().enumerate().for_each(|(id, event)| {
        let final_hash = calculate_event_hash::<T>(event);
        tree.set(id as u64, final_hash);
    });
    H256::from_slice(&tree.commit().to_bytes_be())
}

/// Compute the combined hash of the transaction hash and the signature.
///
/// Since the transaction hash doesn't take the signature values as its input
/// computing the transaction commitent uses a hash value that combines
/// the transaction hash with the array of signature values.
///
/// # Arguments
///
/// * `tx` - The transaction to compute the hash of.
///
/// # Returns
///
/// The transaction hash with signature.
fn calculate_transaction_hash_with_signature<T>(tx: &Transaction) -> FieldElement
where
    T: CryptoHasher,
{
    let signature_hash = <T as CryptoHasher>::compute_hash_on_elements(
        &tx.signature
            .iter()
            .map(|elt| FieldElement::from_byte_slice_be(elt.as_bytes()).unwrap())
            .collect::<Vec<FieldElement>>(),
    );
    <T as CryptoHasher>::hash(FieldElement::from_byte_slice_be(tx.hash.as_bytes()).unwrap(), signature_hash)
}
/// Computes the transaction hash of an invoke transaction.
///
/// # Argument
///
/// * `transaction` - The invoke transaction to get the hash of.
pub fn calculate_invoke_tx_hash(transaction: InvokeTransaction) -> H256 {
    calculate_transaction_hash_common::<PedersenHasher>(
        transaction.sender_address,
        &transaction.calldata,
        transaction.max_fee,
        transaction.nonce,
        transaction.version,
        b"invoke",
    )
}

/// Computes the transaction hash of a declare transaction.
///
/// # Argument
///
/// * `transaction` - The declare transaction to get the hash of.
pub fn calculate_declare_tx_hash(transaction: DeclareTransaction) -> H256 {
    calculate_transaction_hash_common::<PedersenHasher>(
        transaction.sender_address,
        &[U256::from_big_endian(&transaction.compiled_class_hash)],
        transaction.max_fee,
        transaction.nonce,
        transaction.version,
        b"declare",
    )
}

/// Computes the transaction hash of a deploy account transaction.
///
/// # Argument
///
/// * `transaction` - The deploy account transaction to get the hash of.
pub fn calculate_deploy_account_tx_hash(transaction: DeployAccountTransaction) -> H256 {
    calculate_transaction_hash_common::<PedersenHasher>(
        transaction.sender_address,
        &vec![
            vec![U256::from_big_endian(&transaction.account_class_hash), transaction.salt],
            transaction.calldata.into_inner(),
        ]
        .concat(),
        transaction.max_fee,
        transaction.nonce,
        transaction.version,
        b"deploy_account",
    )
}

fn calculate_transaction_hash_common<T>(
    sender_address: [u8; 32],
    calldata: &[U256],
    max_fee: U256,
    nonce: U256,
    version: u8,
    tx_prefix: &[u8],
) -> H256
where
    T: CryptoHasher,
{
    // All the values are validated before going through this function so it's safe to unwrap.
    let sender_address = FieldElement::from_bytes_be(&sender_address).unwrap();
    let calldata_hash = <T as CryptoHasher>::compute_hash_on_elements(
        &calldata.iter().map(|&val| FieldElement::from_bytes_be(&val.into()).unwrap()).collect::<Vec<FieldElement>>(),
    );
    let max_fee = FieldElement::from_bytes_be(&max_fee.into()).unwrap();
    let nonce = FieldElement::from_bytes_be(&nonce.into()).unwrap();
    let version = FieldElement::from_byte_slice_be(&version.to_be_bytes()).unwrap();
    let tx_prefix = FieldElement::from_byte_slice_be(tx_prefix).unwrap();
    // TODO: make it configurable
    // FIXME: https://github.com/keep-starknet-strange/madara/issues/364
    let chain_id = FieldElement::from_byte_slice_be(b"SN_GOERLI").unwrap();

    let tx_hash = <T as CryptoHasher>::compute_hash_on_elements(&vec![
        tx_prefix,
        version,
        sender_address,
        FieldElement::ZERO,
        calldata_hash,
        max_fee,
        chain_id,
        nonce,
    ]);
    H256::from_slice(&tx_hash.to_bytes_be())
}

/// Calculate the hash of an event.
///
/// See the [documentation](https://docs.starknet.io/docs/Events/starknet-events#event-hash)
/// for details.
pub fn calculate_event_hash<T: CryptoHasher>(event: &EventWrapper) -> FieldElement {
    let keys_hash = T::compute_hash_on_elements(
        &event
            .keys
            .iter()
            .map(|key| FieldElement::from_byte_slice_be(key.as_bytes()).unwrap())
            .collect::<Vec<FieldElement>>(),
    );
    let data_hash = T::compute_hash_on_elements(
        &event
            .data
            .iter()
            .map(|data| FieldElement::from_byte_slice_be(data.as_bytes()).unwrap())
            .collect::<Vec<FieldElement>>(),
    );
    let from_address = FieldElement::from_byte_slice_be(event.from_address.as_bytes_ref()).unwrap();
    T::compute_hash_on_elements(&[from_address, keys_hash, data_hash])
}
