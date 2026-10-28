//! Streaming file checksums shared with the small privileged launchers.
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

/// Lowercase hex with leading zeros, independent of the digest array's traits.
pub fn hex(digest: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digest = digest.as_ref();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for &byte in digest {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    encoded
}

pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hex(hash.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_keep_lowercase_hex_and_leading_zeros() {
        assert_eq!(hex([0, 1, 15, 16, 128, 255]), "00010f1080ff");
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(hex(Sha256::digest(b"abc")), expected);
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"abc").unwrap();
        assert_eq!(hash_file(file.path()).unwrap(), (expected.into(), 3));
    }
}
