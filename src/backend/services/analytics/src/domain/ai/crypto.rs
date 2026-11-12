//! Sealing for stored Anthropic tokens.
//!
//! AES-256-GCM with a per-write nonce. The associated data is the row's tenant
//! and person, so a ciphertext lifted into another person's row fails to open
//! rather than handing that person someone else's key.

use toolkit_security::crypto::aead::{Aes256Gcm, NONCE_LEN};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config::KEY_BYTES;

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
    let cipher =
        Aes256Gcm::new(key).map_err(|error| anyhow::anyhow!("AES-GCM key rejected: {error}"))?;
    let aad = associated_data(tenant, person);
    let (nonce, ciphertext) = cipher
        .seal(&aad, token.as_bytes())
        .map_err(|error| anyhow::anyhow!("failed to seal the token: {error}"))?;

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
    let nonce = <[u8; NONCE_LEN]>::try_from(sealed.nonce.as_slice()).map_err(|_| {
        anyhow::anyhow!(
            "stored nonce is {} bytes, expected {NONCE_LEN}",
            sealed.nonce.len()
        )
    })?;

    let cipher =
        Aes256Gcm::new(key).map_err(|error| anyhow::anyhow!("AES-GCM key rejected: {error}"))?;
    let aad = associated_data(tenant, person);

    let plaintext = cipher
        .open(&nonce, &aad, &sealed.ciphertext)
        .map_err(|_| anyhow::anyhow!("failed to open the sealed token"))?;

    let token = std::str::from_utf8(&plaintext)
        .map_err(|_| anyhow::anyhow!("sealed token is not valid UTF-8"))?;

    Ok(Zeroizing::new(token.to_owned()))
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

    // A row sealed outside insight (python `cryptography` AESGCM, same key,
    // tenant/person AAD and token as above). Stored rows must keep opening
    // whatever AES-GCM implementation seals new ones.
    #[test]
    fn opens_a_row_sealed_by_another_implementation() -> anyhow::Result<()> {
        let sealed = Sealed {
            nonce: hex::decode("000102030405060708090a0b")?,
            ciphertext: hex::decode(
                "34da5b2da194706f41ac5695ca8cff94d2a0125b14abcd737bf3241ce5316e46480859e347f141eba3c65ad8a85104f08dc59eee79",
            )?,
        };

        assert_eq!(open(&KEY, tenant(), person(), &sealed)?.as_str(), TOKEN);
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
