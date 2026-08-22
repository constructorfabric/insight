//! Sealing for stored Anthropic tokens.
//!
//! AES-256-GCM with a per-write nonce. The associated data is the row's tenant
//! and person, so a ciphertext lifted into another person's row fails to open
//! rather than handing that person someone else's key.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config::KEY_BYTES;

const NONCE_BYTES: usize = 12;

/// A sealed token as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Seal `token` for exactly this tenant and person.
///
/// # Errors
///
/// Returns an error when the AEAD refuses to encrypt.
pub fn seal(
    key: &[u8; KEY_BYTES],
    tenant: Uuid,
    person: Uuid,
    token: &str,
) -> anyhow::Result<Sealed> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let aad = associated_data(tenant, person);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: token.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("failed to seal the token"))?;

    Ok(Sealed {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

/// Open a sealed token stored against this tenant and person.
///
/// # Errors
///
/// Returns an error when the nonce is the wrong length, the ciphertext was
/// tampered with, the key is wrong, or the row belongs to someone else.
pub fn open(
    key: &[u8; KEY_BYTES],
    tenant: Uuid,
    person: Uuid,
    sealed: &Sealed,
) -> anyhow::Result<Zeroizing<String>> {
    if sealed.nonce.len() != NONCE_BYTES {
        anyhow::bail!(
            "stored nonce is {} bytes, expected {NONCE_BYTES}",
            sealed.nonce.len()
        );
    }

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let aad = associated_data(tenant, person);

    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&sealed.nonce),
            Payload {
                msg: &sealed.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("failed to open the sealed token"))?;

    let token = String::from_utf8(plaintext)
        .map_err(|_| anyhow::anyhow!("sealed token is not valid UTF-8"))?;

    Ok(Zeroizing::new(token))
}

/// The last four characters of a token — all the UI ever learns about it.
pub fn hint(token: &str) -> String {
    let trimmed = token.trim();
    let count = trimmed.chars().count();
    trimmed.chars().skip(count.saturating_sub(4)).collect()
}

fn associated_data(tenant: Uuid, person: Uuid) -> [u8; 32] {
    let mut aad = [0_u8; 32];
    aad[..16].copy_from_slice(tenant.as_bytes());
    aad[16..].copy_from_slice(person.as_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_BYTES] = [3_u8; KEY_BYTES];
    const TOKEN: &str = "sk-ant-api03-example-token-value-wxyz";

    fn tenant() -> Uuid {
        Uuid::from_u128(1)
    }

    fn person() -> Uuid {
        Uuid::from_u128(2)
    }

    #[test]
    fn seal_then_open_round_trips() -> anyhow::Result<()> {
        let sealed = seal(&KEY, tenant(), person(), TOKEN)?;

        assert_eq!(open(&KEY, tenant(), person(), &sealed)?.as_str(), TOKEN);
        Ok(())
    }

    #[test]
    fn sealing_the_same_token_twice_gives_different_ciphertext() -> anyhow::Result<()> {
        let first = seal(&KEY, tenant(), person(), TOKEN)?;
        let second = seal(&KEY, tenant(), person(), TOKEN)?;

        assert_ne!(first.ciphertext, second.ciphertext);
        Ok(())
    }

    #[test]
    fn open_rejects_another_persons_row() -> anyhow::Result<()> {
        let sealed = seal(&KEY, tenant(), person(), TOKEN)?;

        assert!(open(&KEY, tenant(), Uuid::from_u128(99), &sealed).is_err());
        Ok(())
    }

    #[test]
    fn open_rejects_another_tenants_row() -> anyhow::Result<()> {
        let sealed = seal(&KEY, tenant(), person(), TOKEN)?;

        assert!(open(&KEY, Uuid::from_u128(99), person(), &sealed).is_err());
        Ok(())
    }

    #[test]
    fn open_rejects_tampered_ciphertext() -> anyhow::Result<()> {
        let mut sealed = seal(&KEY, tenant(), person(), TOKEN)?;
        sealed.ciphertext[0] ^= 0xff;

        assert!(open(&KEY, tenant(), person(), &sealed).is_err());
        Ok(())
    }

    #[test]
    fn open_rejects_another_key() -> anyhow::Result<()> {
        let sealed = seal(&KEY, tenant(), person(), TOKEN)?;

        assert!(open(&[9_u8; KEY_BYTES], tenant(), person(), &sealed).is_err());
        Ok(())
    }

    #[test]
    fn open_rejects_a_nonce_of_the_wrong_length() -> anyhow::Result<()> {
        let mut sealed = seal(&KEY, tenant(), person(), TOKEN)?;
        sealed.nonce.pop();

        assert!(open(&KEY, tenant(), person(), &sealed).is_err());
        Ok(())
    }

    #[test]
    fn hint_is_the_last_four_characters() {
        assert_eq!(hint(TOKEN), "wxyz");
    }

    #[test]
    fn hint_of_a_short_token_is_the_whole_token() {
        assert_eq!(hint("ab"), "ab");
    }
}
