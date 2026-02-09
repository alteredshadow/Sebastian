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

    #[test]
    fn test_aes_encrypt_decrypt_roundtrip() {
        let key = b"01234567890123456789012345678901"; // 32 bytes
        let plaintext = b"Hello, Sebastian!";

        let encrypted = aes_encrypt(key, plaintext);
        assert!(!encrypted.is_empty());

        let decrypted = aes_decrypt(key, &encrypted);
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aes_empty_data() {
        let key = b"01234567890123456789012345678901";
        let plaintext = b"";

        let encrypted = aes_encrypt(key, plaintext);
        assert!(!encrypted.is_empty());

        let decrypted = aes_decrypt(key, &encrypted);
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aes_tampered_data() {
        let key = b"01234567890123456789012345678901";
        let plaintext = b"Hello, Sebastian!";

        let mut encrypted = aes_encrypt(key, plaintext);
        // Tamper with ciphertext
        if encrypted.len() > AES_BLOCK_SIZE + 1 {
            encrypted[AES_BLOCK_SIZE + 1] ^= 0xFF;
        }

        let decrypted = aes_decrypt(key, &encrypted);
        assert!(decrypted.is_empty()); // HMAC should fail
    }

    #[test]
    fn test_rsa_keypair_generation() {
        let result = generate_rsa_keypair();
        assert!(result.is_some());
        let (pub_pem, _priv_key) = result.unwrap();
        assert!(pub_pem.starts_with(b"-----BEGIN RSA PUBLIC KEY-----"));
    }

    #[test]
    fn test_aes_large_data() {
        let key = b"01234567890123456789012345678901";
        let plaintext = vec![0xABu8; 1024 * 512]; // 512KB

        let encrypted = aes_encrypt(key, &plaintext);
        let decrypted = aes_decrypt(key, &encrypted);
        assert_eq!(decrypted, plaintext);
    }
}
