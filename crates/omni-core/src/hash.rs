//! BLAKE3 hashing, which names the chunks of a workspace in RustFS
//! (`ws/{workspace_id}/chunks/{hash}`, US-030). This is the portable
//! reference algorithm of the BLAKE3 specification for the default hash of
//! 32 bytes: no SIMD and no threads, so it builds natively and for
//! WebAssembly alike and has no dependency. Chunks are small, so speed is not
//! what matters; tests/hash.rs checks the output against the published
//! BLAKE3 test vectors.

const IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
const PARENT: u32 = 4;
const ROOT: u32 = 8;

/// The bytes of one chunk of the BLAKE3 tree, and of one block of a chunk.
const CHUNK_LEN: usize = 1024;
const BLOCK_LEN: usize = 64;

#[inline(always)]
fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn round(state: &mut [u32; 16], m: &[u32; 16]) {
    quarter_round(state, 0, 4, 8, 12, m[0], m[1]);
    quarter_round(state, 1, 5, 9, 13, m[2], m[3]);
    quarter_round(state, 2, 6, 10, 14, m[4], m[5]);
    quarter_round(state, 3, 7, 11, 15, m[6], m[7]);
    quarter_round(state, 0, 5, 10, 15, m[8], m[9]);
    quarter_round(state, 1, 6, 11, 12, m[10], m[11]);
    quarter_round(state, 2, 7, 8, 13, m[12], m[13]);
    quarter_round(state, 3, 4, 9, 14, m[14], m[15]);
}

/// The compression function: the 16 words of output for a block.
fn compress(
    chaining: &[u32; 8],
    block: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut state = [
        chaining[0],
        chaining[1],
        chaining[2],
        chaining[3],
        chaining[4],
        chaining[5],
        chaining[6],
        chaining[7],
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];
    let mut message = *block;
    for round_number in 0..7 {
        round(&mut state, &message);
        if round_number < 6 {
            let mut permuted = [0u32; 16];
            for (i, word) in permuted.iter_mut().enumerate() {
                *word = message[MSG_PERMUTATION[i]];
            }
            message = permuted;
        }
    }
    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining[i];
    }
    state
}

fn first_eight(words: [u32; 16]) -> [u32; 8] {
    let mut out = [0u32; 8];
    out.copy_from_slice(&words[..8]);
    out
}

fn words_of_block(bytes: &[u8]) -> [u32; 16] {
    let mut padded = [0u8; BLOCK_LEN];
    padded[..bytes.len()].copy_from_slice(bytes);
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(padded.chunks_exact(4)) {
        *word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    words
}

/// The output of a tree node before it is given its final form: the words of
/// the last compression, which is the chaining value of the node, or, for the
/// root, the hash.
fn chunk_output(bytes: &[u8], counter: u64, root: bool) -> [u32; 16] {
    let mut chaining = IV;
    let blocks = bytes.len().div_ceil(BLOCK_LEN).max(1);
    for (index, block) in (0..blocks).map(|i| {
        let start = (i * BLOCK_LEN).min(bytes.len());
        let end = (start + BLOCK_LEN).min(bytes.len());
        (i, &bytes[start..end])
    }) {
        let mut flags = 0;
        if index == 0 {
            flags |= CHUNK_START;
        }
        let last = index == blocks - 1;
        if last {
            flags |= CHUNK_END;
            if root {
                flags |= ROOT;
            }
        }
        let output = compress(
            &chaining,
            &words_of_block(block),
            counter,
            block.len() as u32,
            flags,
        );
        if last {
            return output;
        }
        chaining = first_eight(output);
    }
    unreachable!("a chunk has at least one block")
}

fn parent_output(left: [u32; 8], right: [u32; 8], root: bool) -> [u32; 16] {
    let mut block = [0u32; 16];
    block[..8].copy_from_slice(&left);
    block[8..].copy_from_slice(&right);
    compress(
        &IV,
        &block,
        0,
        BLOCK_LEN as u32,
        PARENT | if root { ROOT } else { 0 },
    )
}

/// The output of the subtree over `bytes`, whose first chunk is chunk number
/// `counter` of the input. The left subtree of a parent holds the largest power
/// of two of chunks that leaves at least one chunk for the right.
fn subtree_output(bytes: &[u8], counter: u64, root: bool) -> [u32; 16] {
    if bytes.len() <= CHUNK_LEN {
        return chunk_output(bytes, counter, root);
    }
    let chunks = bytes.len().div_ceil(CHUNK_LEN);
    let left_chunks = 1usize << (usize::BITS - 1 - (chunks - 1).leading_zeros());
    let (left, right) = bytes.split_at(left_chunks * CHUNK_LEN);
    let left = first_eight(subtree_output(left, counter, false));
    let right = first_eight(subtree_output(right, counter + left_chunks as u64, false));
    parent_output(left, right, root)
}

/// The BLAKE3 hash of `bytes`: 32 bytes.
pub fn blake3(bytes: &[u8]) -> [u8; 32] {
    let words = subtree_output(bytes, 0, true);
    let mut out = [0u8; 32];
    for (target, word) in out.chunks_exact_mut(4).zip(&words[..8]) {
        target.copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// The BLAKE3 hash of `bytes` as 64 lowercase hexadecimal characters, which
/// is how a chunk is named.
pub fn blake3_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in blake3(bytes) {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    //! The output matches the published BLAKE3 test vectors, whose input is
    //! the bytes `i % 251`.

    use super::blake3_hex;

    fn input(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn the_hash_matches_the_blake3_test_vectors() {
        let vectors = [
            (
                0,
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                1,
                "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            ),
            (
                1023,
                "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
            ),
            (
                1024,
                "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
            ),
            (
                1025,
                "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
            ),
            (
                2048,
                "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
            ),
            (
                3072,
                "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2",
            ),
            (
                8192,
                "aae792484c8efe4f19e2ca7d371d8c467ffb10748d8a5a1ae579948f718a2a63",
            ),
        ];
        for (len, expected) in vectors {
            assert_eq!(blake3_hex(&input(len)), expected, "{len} bytes");
        }
    }
}
