//! BLAKE3 hashing (US-030): the output matches the published BLAKE3 test
//! vectors, whose input is the bytes `i % 251`.

use omni_core::hash::{blake3, blake3_hex};

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

#[test]
fn the_hex_form_is_the_hash_in_lowercase() {
    let bytes = blake3(b"board");
    let hex = blake3_hex(b"board");
    assert_eq!(hex.len(), 64);
    let decoded: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex digits"))
        .collect();
    assert_eq!(decoded, bytes);
    assert_eq!(hex, hex.to_lowercase());
}
