//! Merkle Tree implementation for SoroScope state commitments.
//!
//! # Design
//!
//! Leaves are SHA-256 hashed to form leaf nodes. Internal nodes are produced by
//! sorting each pair of child hashes (min ∥ max) before concatenating and hashing,
//! making proofs order-independent (the same convention used by OpenZeppelin).
//! Odd nodes are promoted by pairing with themselves (hash(x ∥ x)).

use hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ─── Proof types ─────────────────────────────────────────────────────────────

/// One step in a Merkle inclusion proof.
///
/// `hash` is the sibling at this level. `is_left` indicates whether the
/// **path node** (the one being proven) is the *left* child at this level.
/// The verifier places the running hash accordingly when combining.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofNode {
    /// The sibling hash at this level.
    pub hash: [u8; 32],
    /// Whether the path node is the left child at this level.
    pub is_left: bool,
}

/// A complete Merkle inclusion proof for one leaf.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// The raw leaf data being proven.
    pub leaf: Vec<u8>,
    /// Sibling hashes from leaf level up to (but not including) the root.
    pub proof: Vec<ProofNode>,
    /// The root hash this proof was generated against.
    pub root: [u8; 32],
}

// ─── MerkleTree ───────────────────────────────────────────────────────────────

/// A binary Merkle Tree for cryptographic state commitments.
///
/// # Example
/// ```
/// use soroscope_core::merkle_tree::MerkleTree;
///
/// let mut tree = MerkleTree::new(4);
/// tree.build(vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]).unwrap();
///
/// let proof = tree.generate_proof(0).unwrap();
/// assert!(MerkleTree::verify_proof(&proof, &tree.root));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    /// The root hash of the built tree. All-zero until `build()` is called.
    pub root: [u8; 32],
    /// Maximum depth (informational; not enforced at build time).
    pub levels: usize,
    /// Raw leaf data kept for proof generation.
    data_leaves: Vec<Vec<u8>>,
    /// All tree levels: `nodes[0]` = leaf hashes, `nodes[last]` = `[root]`.
    nodes: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Creates a new empty Merkle Tree.
    ///
    /// `levels` is informational — it does not limit the number of leaves you
    /// can pass to `build()`.
    pub fn new(levels: usize) -> Self {
        MerkleTree {
            root: [0u8; 32],
            levels,
            data_leaves: Vec::new(),
            nodes: Vec::new(),
        }
    }

    // ── Build ─────────────────────────────────────────────────────────────────

    /// Build the tree from a set of raw leaf values.
    ///
    /// Each leaf is SHA-256 hashed. Internal nodes sort child hashes before
    /// concatenating so the tree structure is order-independent. Odd nodes are
    /// promoted by pairing with themselves.
    ///
    /// # Errors
    /// Returns `Err` when `leaves` is empty.
    pub fn build(&mut self, leaves: Vec<Vec<u8>>) -> Result<(), &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build a Merkle tree from zero leaves.");
        }

        // Hash every leaf.
        let leaf_hashes: Vec<[u8; 32]> = leaves.iter().map(|l| Self::hash_leaf(l)).collect();

        self.data_leaves = leaves;
        self.nodes = Self::build_levels(leaf_hashes);
        self.root = *self.nodes.last().unwrap().first().unwrap();

        Ok(())
    }

    /// Build all levels of the tree bottom-up and return them.
    fn build_levels(mut current: Vec<[u8; 32]>) -> Vec<Vec<[u8; 32]>> {
        let mut all_levels = vec![current.clone()];

        while current.len() > 1 {
            current = Self::parent_level(&current);
            all_levels.push(current.clone());
        }

        all_levels
    }

    /// Compute the next (parent) level from a slice of child hashes.
    fn parent_level(nodes: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let mut parents = Vec::with_capacity((nodes.len() + 1) / 2);
        let mut i = 0;
        while i < nodes.len() {
            let left = nodes[i];
            let right = if i + 1 < nodes.len() { nodes[i + 1] } else { left };
            parents.push(Self::combine_hashes(&left, &right));
            i += 2;
        }
        parents
    }

    // ── Proof generation ──────────────────────────────────────────────────────

    /// Generate an inclusion proof for the leaf at `leaf_index`.
    ///
    /// # Errors
    /// Returns `Err` when the tree has not been built yet or `leaf_index` is
    /// out of range.
    pub fn generate_proof(&self, leaf_index: usize) -> Result<MerkleProof, &'static str> {
        if self.nodes.is_empty() {
            return Err("Tree has not been built. Call build() first.");
        }
        if leaf_index >= self.data_leaves.len() {
            return Err("Leaf index out of range.");
        }

        let mut proof_nodes: Vec<ProofNode> = Vec::new();
        let mut idx = leaf_index;

        for level in 0..self.nodes.len() - 1 {
            let level_nodes = &self.nodes[level];
            let is_left = path_index.is_multiple_of(2);
            let sibling_index = if is_left {
                path_index + 1
            } else {
                path_index - 1
            };

            let (sibling_index, is_left) = if idx % 2 == 0 {
                // path node is left child; sibling is to its right
                let sib = if idx + 1 < level_nodes.len() { idx + 1 } else { idx };
                (sib, true)
            } else {
                // path node is right child; sibling is to its left
                (idx - 1, false)
            };

            proof_nodes.push(ProofNode {
                hash: level_nodes[sibling_index],
            proof.push(ProofNode {
                hash: sibling_hash,
                is_left,
            });

            idx /= 2;
        }

        Ok(MerkleProof {
            leaf: self.data_leaves[leaf_index].clone(),
            proof: proof_nodes,
            root: self.root,
        })
    }

    // ── Proof verification ────────────────────────────────────────────────────

    /// Verify a `MerkleProof` against a trusted `root` hash.
    ///
    /// Starting from the leaf hash, each `ProofNode` supplies the sibling hash
    /// and the side of the path node. The hashes are combined level by level;
    /// the proof is valid iff the final computed hash equals `root`.
    ///
    /// # Returns
    /// `true`  — the leaf is included in the tree whose root matches `root`.
    /// `false` — the proof is invalid or the leaf was tampered with.
    pub fn verify_proof(proof: &MerkleProof, root: &[u8; 32]) -> bool {
        let mut running = Self::hash_leaf(&proof.leaf);

        for node in &proof.proof {
            running = if node.is_left {
                // path node is left, sibling is right
                Self::combine_hashes(&running, &node.hash)
            } else {
                // path node is right, sibling is left
                Self::combine_hashes(&node.hash, &running)
            };
        }

        &running == root
    }

    // ── Getters ───────────────────────────────────────────────────────────────

    /// Returns the root hash as a lowercase hex string.
    pub fn get_root_hex(&self) -> String {
        hex::encode(self.root)
    }

    /// Returns the number of leaves in the tree.
    pub fn leaf_count(&self) -> usize {
        self.data_leaves.len()
    }

    // ── Hashing helpers ───────────────────────────────────────────────────────

    /// SHA-256 hash of a raw leaf value.
    pub fn hash_leaf(data: &[u8]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(data);
        h.finalize().into()
    }

    /// Combine two child hashes into a parent hash.
    ///
    /// Children are sorted (min ∥ max) before hashing so the result is the
    /// same regardless of which side each child sits on.
    pub fn combine_hashes(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let (a, b) = if left <= right { (left, right) } else { (right, left) };
        let mut h = Sha256::new();
        h.update(a);
        h.update(b);
        h.finalize().into()
    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        if left <= right {
            hasher.update(left);
            hasher.update(right);
        } else {
            hasher.update(right);
            hasher.update(left);
        }
        let digest = hasher.finalize();
        digest.into()
    }

    fn build_levels(leaf_hashes: Vec<[u8; 32]>) -> Vec<Vec<[u8; 32]>> {
        let mut levels = Vec::new();
        let mut current_level = leaf_hashes;

        loop {
            levels.push(current_level.clone());
            if current_level.len() == 1 {
                break;
            }

            let mut next_level = Vec::new();
            let mut i = 0;
            while i < current_level.len() {
                let left = &current_level[i];
                let right = if i + 1 < current_level.len() {
                    &current_level[i + 1]
                } else {
                    left
                };
                next_level.push(Self::hash_pair(left, right));
                i += 2;
            }

            current_level = next_level;
        }

        levels
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(data: &[&str]) -> Vec<Vec<u8>> {
        data.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    // ── Build tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_build_single_leaf() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a"])).unwrap();
        assert_eq!(tree.leaf_count(), 1);
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_two_leaves() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b"])).unwrap();
        assert_eq!(tree.leaf_count(), 2);
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_odd_leaf_count() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b", "c"])).unwrap();
        assert_eq!(tree.leaf_count(), 3);
    }

    #[test]
    fn test_build_returns_error_on_empty() {
        let mut tree = MerkleTree::new(4);
        assert!(tree.build(vec![]).is_err());
    }

    #[test]
    fn test_same_leaves_same_root() {
        let mut t1 = MerkleTree::new(4);
        let mut t2 = MerkleTree::new(4);
        t1.build(leaves(&["x", "y", "z"])).unwrap();
        t2.build(leaves(&["x", "y", "z"])).unwrap();
        assert_eq!(t1.root, t2.root);
    }

    #[test]
    fn test_different_leaves_different_root() {
        let mut t1 = MerkleTree::new(4);
        let mut t2 = MerkleTree::new(4);
        t1.build(leaves(&["a", "b"])).unwrap();
        t2.build(leaves(&["a", "c"])).unwrap();
        assert_ne!(t1.root, t2.root);
    }

    // ── verify_proof tests ────────────────────────────────────────────────────

    #[test]
    fn test_verify_proof_valid_two_leaves() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b"])).unwrap();
        let proof = tree.generate_proof(0).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_right_leaf() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b"])).unwrap();
        let proof = tree.generate_proof(1).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_four_leaves_all_indices() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b", "c", "d"])).unwrap();
        for i in 0..4 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof failed for index {i}"
            );
        }
    }

    #[test]
    fn test_verify_proof_valid_odd_leaf_count() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b", "c"])).unwrap();
        for i in 0..3 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof failed for index {i}"
            );
        }
    }

    #[test]
    fn test_verify_proof_valid_single_leaf() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["solo"])).unwrap();
        let proof = tree.generate_proof(0).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_large_tree() {
        let data: Vec<Vec<u8>> = (0u32..16).map(|i| i.to_le_bytes().to_vec()).collect();
        let mut tree = MerkleTree::new(5);
        tree.build(data).unwrap();
        for i in 0..16 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof failed for leaf {i}"
            );
        }
    }

    #[test]
    fn test_verify_proof_tampered_leaf_fails() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b", "c"])).unwrap();
        let mut proof = tree.generate_proof(0).unwrap();
        proof.leaf = b"tampered".to_vec();
        assert!(!MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_tampered_sibling_fails() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b", "c"])).unwrap();
        let mut proof = tree.generate_proof(0).unwrap();
        if let Some(node) = proof.proof.first_mut() {
            node.hash[0] ^= 0xff; // flip a bit
        }
        assert!(!MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_wrong_root_fails() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["a", "b"])).unwrap();
        let proof = tree.generate_proof(0).unwrap();
        let bad_root = [0xde; 32];
        assert!(!MerkleTree::verify_proof(&proof, &bad_root));
    }

    #[test]
    fn test_get_root_hex_length() {
        let mut tree = MerkleTree::new(4);
        tree.build(leaves(&["hello"])).unwrap();
        assert_eq!(tree.get_root_hex().len(), 64);
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        /// Builder must never panic on any non-empty input.
        #[test]
        fn fuzz_build_never_panics(leaves in arb_leaves(1, 64)) {
            let mut tree = MerkleTree::new(32);
            let _ = tree.build(leaves);
        }

        /// Empty input must return an error, never panic.
        #[test]
        fn fuzz_empty_input_returns_err(_dummy in any::<u8>()) {
            let mut tree = MerkleTree::new(32);
            prop_assert!(tree.build(vec![]).is_err());
        }

        /// Every proof produced by the builder must verify against the tree root.
        #[test]
        fn fuzz_all_proofs_verify(leaves in arb_leaves(1, 32)) {
            let mut tree = MerkleTree::new(32);
            tree.build(leaves).expect("build succeeds");
            for i in 0..tree.leaf_count() {
                let proof = tree.generate_proof(i).expect("proof exists");
                prop_assert!(proof.verify(), "proof for leaf {i} failed");
                prop_assert!(MerkleTree::verify_proof(&proof, &tree.root));
            }
        }

        /// Root must be deterministic: same leaves always produce the same root.
        #[test]
        fn fuzz_build_is_deterministic(leaves in arb_leaves(1, 32)) {
            let mut t1 = MerkleTree::new(32);
            let mut t2 = MerkleTree::new(32);
            t1.build(leaves.clone()).unwrap();
            t2.build(leaves).unwrap();
            prop_assert_eq!(t1.root, t2.root);
        }

        /// A tampered leaf hash must cause proof.verify() to return false.
        #[test]
        fn fuzz_tampered_leaf_invalidates_proof(leaves in arb_leaves(2, 32)) {
            let mut tree = MerkleTree::new(32);
            tree.build(leaves).unwrap();
            let mut proof = tree.generate_proof(0).unwrap();
            proof.leaf_hash[0] ^= 0xff;
            prop_assert!(!proof.verify());
        }

        /// A tampered sibling hash must cause verify_proof to return false.
        #[test]
        fn fuzz_tampered_sibling_invalidates_proof(leaves in arb_leaves(2, 32)) {
            let mut tree = MerkleTree::new(32);
            tree.build(leaves).unwrap();
            let mut proof = tree.generate_proof(0).unwrap();
            if !proof.proof.is_empty() {
                proof.proof[0].hash = [0xFFu8; 32];
                prop_assert!(!MerkleTree::verify_proof(&proof, &tree.root));
            }
        }

        /// All leaves in a tree of power-of-two size must produce valid proofs.
        #[test]
        fn fuzz_power_of_two_leaf_counts_valid(
            log2_size in 1usize..=6usize,
            seed in any::<u64>(),
        ) {
            let size = 1 << log2_size;
            let leaves: Vec<Vec<u8>> = (0..size)
                .map(|i| {
                    let mut v = seed.to_le_bytes().to_vec();
                    v.extend_from_slice(&(i as u64).to_le_bytes());
                    v
                })
                .collect();
            let mut tree = MerkleTree::new(32);
            tree.build(leaves).unwrap();
            for i in 0..tree.leaf_count() {
                let proof = tree.generate_proof(i).unwrap();
                prop_assert!(MerkleTree::verify_proof(&proof, &tree.root));
            }
        }

        /// Duplicate leaves must not cause panics and must produce a valid tree.
        #[test]
        fn fuzz_duplicate_leaves_no_panic(
            leaf in arb_leaf(),
            count in 2usize..=16usize,
        ) {
            let leaves = vec![leaf; count];
            let mut tree = MerkleTree::new(32);
            if tree.build(leaves).is_ok() {
                prop_assert_eq!(tree.leaf_count(), count);
            }
        }

        /// Very large individual leaves (up to 64 KiB) must not cause panics.
        #[test]
        fn fuzz_large_leaf_values(
            leaf in prop::collection::vec(any::<u8>(), 0..=65536usize),
        ) {
            let mut tree = MerkleTree::new(32);
            let _ = tree.build(vec![leaf]);
        }

        /// hex-string round-trip: encode leaves to hex, build via from_hex_strings,
        /// result must equal direct build.
        #[test]
        fn fuzz_hex_roundtrip(leaves in arb_leaves(1, 16)) {
            let hex_leaves: Vec<String> = leaves.iter().map(hex::encode).collect();
            let tree_via_hex = MerkleTree::from_hex_strings(hex_leaves);
            let mut tree_direct = MerkleTree::new(32);
            tree_direct.build(leaves).unwrap();
            if let Ok(t) = tree_via_hex {
                prop_assert_eq!(t.root, tree_direct.root);
            }
        }
    }
}
