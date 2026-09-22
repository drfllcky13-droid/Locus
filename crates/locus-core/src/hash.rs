//! Streaming SHA-256 for evidence files.

use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

const BUF: usize = 1 << 20;

/// Hash everything `r` yields. `progress` receives the byte count read so far.
pub fn sha256_reader(mut r: impl Read, progress: &mut dyn FnMut(u64)) -> io::Result<(String, u64)> {
    sha256_tee(&mut r, &mut io::sink(), progress)
}

/// Hash a file, opened read-only.
pub fn sha256_file(path: &Path, progress: &mut dyn FnMut(u64)) -> io::Result<(String, u64)> {
    sha256_reader(File::open(path)?, progress)
}

/// Copy `src` (opened read-only) to a new file `dst`, hashing the bytes as they pass.
/// Fails if `dst` already exists.
pub fn copy_hashed(
    src: &Path,
    dst: &Path,
    progress: &mut dyn FnMut(u64),
) -> io::Result<(String, u64)> {
    let mut out = OpenOptions::new().write(true).create_new(true).open(dst)?;
    let result = sha256_tee(&mut File::open(src)?, &mut out, progress)?;
    out.sync_all()?;
    Ok(result)
}

fn sha256_tee(
    r: &mut dyn Read,
    w: &mut dyn Write,
    progress: &mut dyn FnMut(u64),
) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        w.write_all(&buf[..n])?;
        total += n as u64;
        progress(total);
    }
    Ok((hex::encode(hasher.finalize()), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // FIPS 180-2 test vectors.
        let (h, n) = sha256_reader(&b""[..], &mut |_| {}).unwrap();
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(n, 0);
        let (h, _) = sha256_reader(&b"abc"[..], &mut |_| {}).unwrap();
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn copy_matches_and_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a"), dir.path().join("b"));
        let data: Vec<u8> = (0..3 * BUF + 17).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let (h, n) = copy_hashed(&src, &dst, &mut |_| {}).unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(std::fs::read(&dst).unwrap(), data);
        assert_eq!(h, sha256_file(&src, &mut |_| {}).unwrap().0);
        assert!(copy_hashed(&src, &dst, &mut |_| {}).is_err());
    }
}
