//! Bounded HLS AES-128-CBC decryption using RustCrypto, with strict PKCS#7.
use crate::{Error, Result};
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};

pub(crate) struct Decryptor {
    cipher: cbc::Decryptor<aes::Aes128>,
    pending: [u8; 16],
    used: usize,
}
impl Decryptor {
    pub(crate) fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
        Self {
            cipher: cbc::Decryptor::new(key.into(), iv.into()),
            pending: [0; 16],
            used: 0,
        }
    }
    /// Retain the last ciphertext block until EOF so padding never reaches output.
    /// Memory is proportional to the caller's bounded input chunk, not the segment.
    pub(crate) fn update(&mut self, mut bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len() + 16);
        while !bytes.is_empty() {
            if self.used == 16 {
                let mut block = self.pending.into();
                self.cipher.decrypt_block(&mut block);
                output.extend_from_slice(&block);
                self.used = 0;
            }
            let take = bytes.len().min(16 - self.used);
            self.pending[self.used..self.used + take].copy_from_slice(&bytes[..take]);
            self.used += take;
            bytes = &bytes[take..];
        }
        output
    }
    pub(crate) fn finish(mut self) -> Result<Vec<u8>> {
        if self.used != 16 {
            return Err(invalid());
        }
        self.cipher
            .decrypt_padded::<Pkcs7>(&mut self.pending)
            .map(|plain| plain.to_vec())
            .map_err(|_| invalid())
    }
}
fn invalid() -> Error {
    Error::UnsupportedManifest("invalid AES-128 ciphertext length or PKCS#7 padding".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn nist_sp800_38a_cbc_vector_matches_across_odd_chunk_boundaries() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let cipher = hex(
            "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7",
        );
        let plain = hex(
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
        );
        let mut decryptor = Decryptor::new(&key, &iv);
        let mut output = Vec::new();
        for chunk in cipher.chunks(7) {
            output.extend(decryptor.update(chunk));
        }
        // NIST's vector has no padding; a following block flushes its final block.
        output.extend(decryptor.update(&[0; 16]));
        assert_eq!(output, plain);
        assert!(decryptor.finish().is_err());
    }
    #[test]
    fn pinned_upstream_cbc_regression_vector() {
        // yt-dlp 897341241f2bfb2439fe0367f4c7c9ccd29a3828,
        // test/test_aes.py::TestAES::test_cbc_decrypt.
        let mut key = [0; 16];
        key[..2].copy_from_slice(&[0x20, 0x15]);
        let cipher = hex("97922be50bc318916b79396d26b3b540e627c2962ec87588ab392d5b9e7cf1cd");
        let mut decryptor = Decryptor::new(&key, &key);
        let mut plain = decryptor.update(&cipher);
        plain.extend(decryptor.finish().unwrap());
        assert_eq!(plain, b"Secret message goes here");
    }

    #[test]
    fn rustcrypto_padded_vector_and_invalid_lengths() {
        // Published RustCrypto CBC example vector, independently fixed ciphertext.
        let cipher = hex(
            "c7fe247ef97b21f07cbdd26cb5d346bfd27867cb00d9486723e159978fb9a5f914cfb228a710de4171e396e7b6cf859e",
        );
        let mut decryptor = Decryptor::new(&[0x42; 16], &[0x24; 16]);
        let mut plain = Vec::new();
        for byte in &cipher {
            plain.extend(decryptor.update(&[*byte]));
        }
        plain.extend(decryptor.finish().unwrap());
        assert_eq!(plain, b"hello world! this is my plaintext.");
        for length in [0, 1, 15, 17, 31] {
            let mut decryptor = Decryptor::new(&[0; 16], &[0; 16]);
            decryptor.update(&vec![0; length]);
            assert!(decryptor.finish().is_err());
        }
        let mut corrupted = cipher;
        corrupted[31] ^= 1;
        let mut decryptor = Decryptor::new(&[0x42; 16], &[0x24; 16]);
        decryptor.update(&corrupted);
        assert!(decryptor.finish().is_err());
    }
}
