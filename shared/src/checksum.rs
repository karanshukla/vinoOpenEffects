use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Lower-case hex SHA-256 of a file, read in chunks so a model never sits in memory whole.
pub fn sha256_hex(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_digests() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(
            sha256_hex(&empty).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let abc = dir.path().join("abc");
        std::fs::write(&abc, b"abc").unwrap();
        assert_eq!(
            sha256_hex(&abc).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_file_longer_than_one_chunk_hashes_the_same_as_in_one_piece() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long");
        let bytes = vec![0x5a_u8; 8192 * 2 + 17];
        std::fs::write(&path, &bytes).unwrap();
        let whole: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(sha256_hex(&path).unwrap(), whole);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(sha256_hex(Path::new("/nonexistent/model.onnx")).is_err());
    }
}
