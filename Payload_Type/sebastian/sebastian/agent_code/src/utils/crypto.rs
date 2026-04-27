use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rsa::{
    pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey},
    Oaep, RsaPrivateKey, RsaPublicKey,
};
use sha1::Sha1;
use sha2::Sha256;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

const AES_BLOCK_SIZE: usize = 16;
const HMAC_SIZE: usize = 32;
const RSA_KEY_BITS: usize = 4096;

/// Generate a 4096-bit RSA key pair
/// Returns (PEM-encoded public key bytes, private key)
pub fn generate_rsa_keypair() -> Option<(Vec<u8>, RsaPrivateKey)> {
    let mut rng = rand::thread_rng();
    let private_key = match RsaPrivateKey::new(&mut rng, RSA_KEY_BITS) {
        Ok(key) => key,
        Err(e) => {
            log::error!("Failed to generate RSA key pair: {}", e);
            return None;
        }
    };

    let public_key = RsaPublicKey::from(&private_key);
    let pub_pem = match public_key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF) {
        Ok(pem) => pem,
        Err(e) => {
            log::error!("Failed to encode public key to PEM: {}", e);
            return None;
        }
    };

    Some((pub_pem.as_bytes().to_vec(), private_key))
}

/// RSA-OAEP decrypt with SHA1
pub fn rsa_decrypt_cipher_bytes(encrypted_data: &[u8], private_key: &RsaPrivateKey) -> Vec<u8> {
    let padding = Oaep::new::<Sha1>();
    match private_key.decrypt(padding, encrypted_data) {
        Ok(decrypted) => decrypted,
        Err(e) => {
            log::debug!("Failed to RSA decrypt: {}", e);
            Vec::new()
        }
    }
}

/// RSA-OAEP encrypt with SHA1
pub fn rsa_encrypt_bytes(plain_bytes: &[u8], public_key_der: &[u8]) -> Vec<u8> {
    let pub_key = match RsaPublicKey::from_pkcs1_der(public_key_der) {
        Ok(key) => key,
        Err(e) => {
            log::debug!("Error parsing public key: {}", e);
            return Vec::new();
        }
    };

    let padding = Oaep::new::<Sha1>();
    let mut rng = rand::thread_rng();
    match pub_key.encrypt(&mut rng, padding, plain_bytes) {
        Ok(encrypted) => encrypted,
        Err(e) => {
            log::debug!("Unable to encrypt: {}", e);
            Vec::new()
        }
    }
}

/// AES-256-CBC encrypt with PKCS7 padding + HMAC-SHA256
/// Returns: IV (16 bytes) || ciphertext || HMAC (32 bytes)
pub fn aes_encrypt(key: &[u8], plain_bytes: &[u8]) -> Vec<u8> {
    if key.len() != 32 {
        log::error!("AES key must be 32 bytes, got {}", key.len());
        return Vec::new();
    }

    // Generate random IV
    let mut iv = [0u8; AES_BLOCK_SIZE];
    rand::thread_rng().fill_bytes(&mut iv);

    // Encrypt with PKCS7 padding (in-place)
    let enc = Aes256CbcEnc::new_from_slices(key, &iv).expect("Invalid key/IV length");
    // Allocate buffer: plaintext + up to one block of padding
    let padded_len = ((plain_bytes.len() / AES_BLOCK_SIZE) + 1) * AES_BLOCK_SIZE;
    let mut buf = vec![0u8; padded_len];
    buf[..plain_bytes.len()].copy_from_slice(plain_bytes);
    let enc_bytes = enc
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain_bytes.len())
        .expect("Encryption buffer too small");

    // Build: IV + ciphertext
    let mut result = Vec::with_capacity(AES_BLOCK_SIZE + enc_bytes.len() + HMAC_SIZE);
    result.extend_from_slice(&iv);
    result.extend_from_slice(enc_bytes);

    // Compute HMAC-SHA256 over IV + ciphertext
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length error");
    mac.update(&result);
    let hmac_result = mac.finalize().into_bytes();

    // Append HMAC
    result.extend_from_slice(&hmac_result);

    result
}

/// AES-256-CBC decrypt: verify HMAC-SHA256, then decrypt with PKCS7 unpadding
/// Input: IV (16 bytes) || ciphertext || HMAC (32 bytes)
pub fn aes_decrypt(key: &[u8], encrypted_bytes: &[u8]) -> Vec<u8> {
    if key.len() != 32 {
        log::error!("AES key must be 32 bytes, got {}", key.len());
        return Vec::new();
    }

    if encrypted_bytes.len() < AES_BLOCK_SIZE + HMAC_SIZE {
        log::error!("Ciphertext too short");
        return Vec::new();
    }

    // Split: IV | encrypted_portion | hmac_hash
    let iv = &encrypted_bytes[..AES_BLOCK_SIZE];
    let hmac_hash = &encrypted_bytes[encrypted_bytes.len() - HMAC_SIZE..];
    let encrypted_portion = &encrypted_bytes[AES_BLOCK_SIZE..encrypted_bytes.len() - HMAC_SIZE];

    // Verify HMAC over IV + ciphertext
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length error");
    mac.update(&encrypted_bytes[..encrypted_bytes.len() - HMAC_SIZE]);
    if mac.verify_slice(hmac_hash).is_err() {
        log::error!("HMAC verification failed");
        return Vec::new();
    }

    // Check alignment
    if encrypted_portion.len() % AES_BLOCK_SIZE != 0 {
        log::error!("Ciphertext not a multiple of the block size");
        return Vec::new();
    }

    // Decrypt
    let dec = Aes256CbcDec::new_from_slices(key, iv).expect("Invalid key/IV length");
    let mut ct_buf = encrypted_portion.to_vec();
    match dec.decrypt_padded_mut::<Pkcs7>(&mut ct_buf) {
        Ok(plain) => plain.to_vec(),
        Err(e) => {
            log::error!("Padding error: {}", e);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8; 32] = b"01234567890123456789012345678901";

    // -------------------------------------------------------------------------
    // AES
    // -------------------------------------------------------------------------

    #[test]
    fn test_aes_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, Sebastian!";
        let encrypted = aes_encrypt(KEY, plaintext);
        assert!(!encrypted.is_empty());
        assert_eq!(aes_decrypt(KEY, &encrypted), plaintext);
    }

    #[test]
    fn test_aes_empty_data() {
        let encrypted = aes_encrypt(KEY, b"");
        assert!(!encrypted.is_empty());
        assert_eq!(aes_decrypt(KEY, &encrypted), b"");
    }

    #[test]
    fn test_aes_tampered_ciphertext_fails_hmac() {
        let plaintext = b"Hello, Sebastian!";
        let mut encrypted = aes_encrypt(KEY, plaintext);
        // Flip a byte in the ciphertext region (after IV, before HMAC)
        if encrypted.len() > AES_BLOCK_SIZE + 1 {
            encrypted[AES_BLOCK_SIZE + 1] ^= 0xFF;
        }
        assert!(aes_decrypt(KEY, &encrypted).is_empty());
    }

    #[test]
    fn test_aes_tampered_hmac_fails() {
        let plaintext = b"tamper the mac";
        let mut encrypted = aes_encrypt(KEY, plaintext);
        // Flip the last byte of the HMAC
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0x01;
        assert!(aes_decrypt(KEY, &encrypted).is_empty());
    }

    #[test]
    fn test_aes_wrong_key_fails_hmac() {
        let encrypted = aes_encrypt(KEY, b"secret");
        let wrong_key = b"99999999999999999999999999999999";
        assert!(aes_decrypt(wrong_key, &encrypted).is_empty());
    }

    #[test]
    fn test_aes_short_key_encrypt_returns_empty() {
        let short_key = b"tooshort";
        assert!(aes_encrypt(short_key, b"data").is_empty());
    }

    #[test]
    fn test_aes_short_key_decrypt_returns_empty() {
        let short_key = b"tooshort";
        assert!(aes_decrypt(short_key, &[0u8; 64]).is_empty());
    }

    #[test]
    fn test_aes_ciphertext_too_short_returns_empty() {
        // Minimum valid ciphertext is IV(16) + one block(16) + HMAC(32) = 64 bytes.
        // Passing 47 bytes must fail gracefully.
        assert!(aes_decrypt(KEY, &[0u8; 47]).is_empty());
    }

    #[test]
    fn test_aes_block_aligned_data() {
        // 16 bytes — exactly one AES block. PKCS7 adds a full padding block.
        let plaintext = b"0123456789ABCDEF";
        let encrypted = aes_encrypt(KEY, plaintext);
        assert_eq!(aes_decrypt(KEY, &encrypted), plaintext);
    }

    #[test]
    fn test_aes_large_data() {
        let plaintext = vec![0xABu8; 1024 * 512]; // 512 KB
        let encrypted = aes_encrypt(KEY, &plaintext);
        assert_eq!(aes_decrypt(KEY, &encrypted), plaintext);
    }

    #[test]
    fn test_aes_iv_randomness() {
        // Two encryptions of the same plaintext must produce different ciphertexts
        // (different random IVs).
        let ct1 = aes_encrypt(KEY, b"same plaintext");
        let ct2 = aes_encrypt(KEY, b"same plaintext");
        assert_ne!(ct1, ct2);
    }

    // -------------------------------------------------------------------------
    // RSA
    // -------------------------------------------------------------------------

    #[test]
    fn test_rsa_keypair_generation() {
        let result = generate_rsa_keypair();
        assert!(result.is_some());
        let (pub_pem, _priv_key) = result.unwrap();
        assert!(pub_pem.starts_with(b"-----BEGIN RSA PUBLIC KEY-----"));
    }

    #[test]
    fn test_rsa_encrypt_decrypt_roundtrip() {
        let (pub_pem, priv_key) = generate_rsa_keypair().unwrap();

        // Parse the PEM public key back to DER for rsa_encrypt_bytes
        let pem_str = std::str::from_utf8(&pub_pem).unwrap();
        use rsa::pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey};
        let pub_key = rsa::RsaPublicKey::from_pkcs1_pem(pem_str).unwrap();
        let pub_der = pub_key.to_pkcs1_der().unwrap().to_vec();

        let plaintext = b"RSA round-trip test";
        let ciphertext = rsa_encrypt_bytes(plaintext, &pub_der);
        assert!(!ciphertext.is_empty());

        let decrypted = rsa_decrypt_cipher_bytes(&ciphertext, &priv_key);
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_rsa_decrypt_wrong_key_returns_empty() {
        let (pub_pem, _priv_key) = generate_rsa_keypair().unwrap();
        let (_, wrong_priv_key) = generate_rsa_keypair().unwrap();

        let pem_str = std::str::from_utf8(&pub_pem).unwrap();
        use rsa::pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey};
        let pub_key = rsa::RsaPublicKey::from_pkcs1_pem(pem_str).unwrap();
        let pub_der = pub_key.to_pkcs1_der().unwrap().to_vec();

        let ciphertext = rsa_encrypt_bytes(b"secret", &pub_der);
        let result = rsa_decrypt_cipher_bytes(&ciphertext, &wrong_priv_key);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rsa_encrypt_bad_public_key_returns_empty() {
        // Garbage DER bytes must not panic
        let result = rsa_encrypt_bytes(b"data", &[0u8; 32]);
        assert!(result.is_empty());
    }
}
