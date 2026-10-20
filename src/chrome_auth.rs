use crate::error::{Result, SplunkError};
#[cfg(any(target_os = "macos", test))]
use aes::Aes128;
#[cfg(any(target_os = "macos", test))]
use cbc::Decryptor;
#[cfg(any(target_os = "macos", test))]
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
#[cfg(any(target_os = "macos", test))]
use hmac::Hmac;
#[cfg(any(target_os = "macos", test))]
use pbkdf2::pbkdf2;
#[cfg(target_os = "macos")]
use serde::Deserialize;
#[cfg(any(target_os = "macos", test))]
use sha1::Sha1;
#[cfg(any(target_os = "macos", test))]
use sha2::{Digest, Sha256};
use std::process::Command;
use url::Url;

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
struct ChromeCookie {
    name: String,
    value: String,
    encrypted_value: String,
    path: String,
    expires_utc: i64,
}

/// Read only cookies scoped to the configured Splunk host from a Chrome profile.
/// Cookie values are held in memory and never written to the CLI config.
#[cfg(target_os = "macos")]
pub fn read_chrome_auth(base_url: &str, profile: &str) -> Result<(String, String)> {
    if profile.is_empty()
        || profile.contains('/')
        || profile.contains('\\')
        || profile == "."
        || profile == ".."
    {
        return Err(SplunkError::Config("invalid Chrome profile name".into()));
    }

    let splunk_url = Url::parse(base_url)
        .map_err(|_| SplunkError::Config("invalid configured Splunk base URL".into()))?;
    if splunk_url.scheme() != "https" {
        return Err(SplunkError::Config(
            "Chrome cookie authentication requires an HTTPS Splunk base URL".into(),
        ));
    }
    let host = splunk_url
        .host_str()
        .ok_or_else(|| SplunkError::Config("configured Splunk base URL has no host".into()))?
        .to_ascii_lowercase();
    let chrome_dir = dirs::home_dir()
        .ok_or_else(|| SplunkError::Config("could not determine user home directory".into()))?
        .join("Library/Application Support/Google/Chrome")
        .join(profile);
    let cookie_db = [
        chrome_dir.join("Cookies"),
        chrome_dir.join("Network/Cookies"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| {
        SplunkError::Config(format!(
            "Chrome cookie database not found for profile '{profile}'"
        ))
    })?;

    let domain_filter = cookie_domains(&host)
        .iter()
        .map(|domain| format!("'{domain}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT name, value, hex(encrypted_value) AS encrypted_value, path, expires_utc FROM cookies WHERE host_key IN ({domain_filter}) AND name IN ('splunkd_8000', 'splunkweb_csrf_token_8000')"
    );
    let output = Command::new("sqlite3")
        .args(["-readonly", "-json"])
        .arg(&cookie_db)
        .arg(sql)
        .output()
        .map_err(|e| {
            SplunkError::Config(format!("could not run sqlite3 to read Chrome cookies: {e}"))
        })?;
    if !output.status.success() {
        return Err(SplunkError::Config(
            "could not read Chrome cookies (the profile may be locked or unsupported)".into(),
        ));
    }
    let rows: Vec<ChromeCookie> = serde_json::from_slice(&output.stdout)
        .map_err(|_| SplunkError::Config("could not parse Chrome cookie data".into()))?;

    let safe_storage_key = chrome_safe_storage_key()?;
    let mut cookie_pairs = Vec::new();
    let mut form_key = None;
    for row in rows {
        if row.expires_utc != 0 && chrome_time_now_micros() >= row.expires_utc
            || !cookie_path_matches(
                &row.path,
                "/en-GB/splunkd/__raw/services/search/jobs/export",
            )
        {
            continue;
        }
        let value = if !row.value.is_empty() {
            row.value
        } else if !row.encrypted_value.is_empty() {
            let encrypted = decode_hex(&row.encrypted_value)?;
            decrypt_chrome_value(&encrypted, &safe_storage_key, &host)?
        } else {
            continue;
        };
        if row.name == "splunkweb_csrf_token_8000" {
            form_key = Some(value.clone());
        }
        cookie_pairs.push(format!("{}={value}", row.name));
    }

    if cookie_pairs.is_empty() {
        return Err(SplunkError::Config(format!(
            "no cookies found for Splunk host '{host}' in Chrome profile '{profile}'"
        )));
    }
    let form_key = form_key.ok_or_else(|| {
        SplunkError::Config(format!(
            "Splunk CSRF cookie not found for host '{host}'; sign in to Splunk in Chrome first"
        ))
    })?;
    Ok((cookie_pairs.join("; "), form_key))
}

fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    request_path == cookie_path
        || request_path
            .strip_prefix(cookie_path)
            .is_some_and(|remainder| cookie_path.ends_with('/') || remainder.starts_with('/'))
}

fn chrome_time_now_micros() -> i64 {
    const WINDOWS_TO_UNIX_EPOCH_MICROS: i64 = 11_644_473_600 * 1_000_000;
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros() as i64 + WINDOWS_TO_UNIX_EPOCH_MICROS)
        .unwrap_or(WINDOWS_TO_UNIX_EPOCH_MICROS)
}

#[cfg(not(target_os = "macos"))]
pub fn read_chrome_auth(_base_url: &str, _profile: &str) -> Result<(String, String)> {
    Err(SplunkError::Config(
        "Chrome cookie authentication is currently supported only on macOS".into(),
    ))
}

#[cfg(target_os = "macos")]
fn chrome_safe_storage_key() -> Result<Vec<u8>> {
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            "Chrome Safe Storage",
            "-a",
            "Chrome",
        ])
        .output()
        .map_err(|e| {
            SplunkError::Config(format!(
                "could not access Chrome Safe Storage in Keychain: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(SplunkError::Config(
            "could not access Chrome Safe Storage in Keychain; allow the Keychain prompt and try again".into(),
        ));
    }
    let password = String::from_utf8(output.stdout)
        .map_err(|_| SplunkError::Config("Chrome Safe Storage key was not valid text".into()))?;
    Ok(password.trim_end_matches(['\r', '\n']).as_bytes().to_vec())
}

#[cfg(any(target_os = "macos", test))]
fn decrypt_chrome_value(encrypted: &[u8], safe_storage_key: &[u8], host: &str) -> Result<String> {
    if encrypted.len() < 3 + 16 || !(encrypted.starts_with(b"v10") || encrypted.starts_with(b"v11"))
    {
        return Err(SplunkError::Config(
            "Chrome cookie uses an unsupported encryption format".into(),
        ));
    }

    let mut key = [0u8; 16];
    pbkdf2::<Hmac<Sha1>>(safe_storage_key, b"saltysalt", 1003, &mut key)
        .map_err(|_| SplunkError::Config("could not derive Chrome cookie decryption key".into()))?;
    let iv = [b' '; 16];
    let mut data = encrypted[3..].to_vec();
    let decrypted = Decryptor::<Aes128>::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut data)
        .map_err(|_| {
            SplunkError::Config("could not decrypt Chrome cookie; sign in again in Chrome".into())
        })?;
    let plaintext =
        if decrypted.len() > 32 && decrypted[..32] == Sha256::digest(host.as_bytes())[..] {
            &decrypted[32..]
        } else {
            decrypted
        };
    String::from_utf8(plaintext.to_vec())
        .map_err(|_| SplunkError::Config("Chrome cookie was not valid text".into()))
}

#[cfg(any(target_os = "macos", test))]
fn decode_hex(encoded: &str) -> Result<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return Err(SplunkError::Config(
            "invalid encrypted Chrome cookie data".into(),
        ));
    }
    encoded
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn hex_digit(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(SplunkError::Config(
            "invalid encrypted Chrome cookie data".into(),
        )),
    }
}

#[cfg(target_os = "macos")]
fn cookie_domains(host: &str) -> Vec<String> {
    let labels: Vec<_> = host.split('.').collect();
    let mut domains = vec![host.to_string(), format!(".{host}")];
    for start in 1..labels.len().saturating_sub(1) {
        domains.push(format!(".{}", labels[start..].join(".")));
    }
    domains
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::Encryptor;
    use cbc::cipher::BlockEncryptMut;

    #[test]
    fn hex_decoding_is_case_insensitive() {
        assert_eq!(decode_hex("00aBcD").unwrap(), [0, 0xab, 0xcd]);
    }

    #[test]
    fn invalid_hex_is_rejected() {
        assert!(decode_hex("0xz").is_err());
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn cookie_path_must_match_request_path() {
        assert!(cookie_path_matches("/", "/en-GB/splunkd/__raw/search"));
        assert!(cookie_path_matches(
            "/en-GB/",
            "/en-GB/splunkd/__raw/search"
        ));
        assert!(!cookie_path_matches(
            "/en-GB/splunkd/__rawhide",
            "/en-GB/splunkd/__raw/search"
        ));
        assert!(!cookie_path_matches(
            "/other/",
            "/en-GB/splunkd/__raw/search"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cookie_domain_filter_includes_parent_domains_but_not_siblings() {
        assert_eq!(
            cookie_domains("splunk-operations.ocset.net"),
            [
                "splunk-operations.ocset.net",
                ".splunk-operations.ocset.net",
                ".ocset.net"
            ]
        );
    }

    #[test]
    fn rejects_non_chrome_encryption_formats() {
        assert!(
            decrypt_chrome_value(b"v20not-supported", b"safe storage", "splunk.example").is_err()
        );
    }

    #[test]
    fn decrypts_chrome_safe_storage_cookie_values() {
        let safe_storage_key = b"test safe storage key";
        let mut key = [0u8; 16];
        pbkdf2::<Hmac<Sha1>>(safe_storage_key, b"saltysalt", 1003, &mut key).unwrap();
        let iv = [b' '; 16];
        let mut encrypted = Sha256::digest(b"splunk.example").to_vec();
        encrypted.extend_from_slice(b"splunk-session-cookie");
        let encrypted_len = encrypted.len();
        encrypted.resize(encrypted_len + 16, 0);
        let ciphertext = Encryptor::<Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut encrypted, encrypted_len)
            .unwrap()
            .to_vec();
        let mut chrome_value = b"v10".to_vec();
        chrome_value.extend(ciphertext);

        assert_eq!(
            decrypt_chrome_value(&chrome_value, safe_storage_key, "splunk.example").unwrap(),
            "splunk-session-cookie"
        );
    }
}
