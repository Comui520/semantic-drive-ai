use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM
const SALT_SIZE: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFile {
    pub id: String,
    pub original_name: String,
    pub original_path: String,
    pub encrypted_name: String,
    pub size: u64,
    pub stored_at: String,
}

/// Derive an AES-256 key from a password using Argon2id
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Key derivation failed: {}", e))?;
    Ok(key)
}

/// Generate a random salt
fn generate_salt() -> [u8; SALT_SIZE] {
    let mut salt = [0u8; SALT_SIZE];
    rand::rngs::OsRng.fill(&mut salt);
    salt
}

/// Encrypt data with AES-256-GCM
pub fn encrypt_data(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Cipher init failed: {}", e))?;
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::rngs::OsRng.fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    // Prepend nonce to ciphertext
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt data with AES-256-GCM
pub fn decrypt_data(encrypted: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if encrypted.len() < NONCE_SIZE + 16 {
        return Err("Data too short".to_string());
    }

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Cipher init failed: {}", e))?;
    let nonce = Nonce::from_slice(&encrypted[..NONCE_SIZE]);
    let ciphertext = &encrypted[NONCE_SIZE..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))
}

/// Hash password for storage verification
pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Check if the vault is configured (has a salt file)
pub fn is_vault_configured(vault_dir: &Path) -> bool {
    vault_dir.join("salt").exists()
}

/// Configure a new vault with a password
pub fn configure_vault(vault_dir: &Path, password: &str) -> Result<(), String> {
    fs::create_dir_all(vault_dir)
        .map_err(|e| format!("Cannot create vault dir: {}", e))?;

    let salt = generate_salt();
    let key = derive_key(password, &salt)?;
    let verification = encrypt_data(b"semantic-drive-vault-verification", &key)?;

    fs::write(vault_dir.join("salt"), &salt).map_err(|e| format!("Cannot save salt: {}", e))?;
    fs::write(vault_dir.join("verify"), &verification)
        .map_err(|e| format!("Cannot save verification: {}", e))?;

    // Create encrypted files directory
    fs::create_dir_all(vault_dir.join("files"))
        .map_err(|e| format!("Cannot create vault files dir: {}", e))?;

    Ok(())
}

/// Unlock the vault with a password. Returns the derived key.
pub fn unlock_vault(vault_dir: &Path, password: &str) -> Result<[u8; 32], String> {
    let salt =
        fs::read(vault_dir.join("salt")).map_err(|_| "Vault not configured".to_string())?;
    let verification = fs::read(vault_dir.join("verify"))
        .map_err(|_| "Vault verification data missing".to_string())?;

    let key = derive_key(password, &salt)?;

    // Verify the password by attempting decryption
    decrypt_data(&verification, &key).map_err(|_| "密码错误".to_string())?;

    Ok(key)
}

/// Encrypt a file and store it in the vault
pub fn encrypt_file(
    source_path: &Path,
    vault_dir: &Path,
    key: &[u8; 32],
) -> Result<VaultFile, String> {
    let data = fs::read(source_path)
        .map_err(|e| format!("Cannot read source file: {}", e))?;

    let original_name = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let encrypted = encrypt_data(&data, key)?;
    let id = uuid::Uuid::new_v4().to_string();
    let enc_name = format!("{}.enc", &id);
    let enc_path = vault_dir.join("files").join(&enc_name);

    fs::write(&enc_path, &encrypted)
        .map_err(|e| format!("Cannot write encrypted file: {}", e))?;

    Ok(VaultFile {
        id,
        original_name,
        original_path: source_path.to_string_lossy().to_string(),
        encrypted_name: enc_name,
        size: data.len() as u64,
        stored_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Decrypt a file from the vault and return its contents
pub fn decrypt_file(
    encrypted_name: &str,
    vault_dir: &Path,
    key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    let enc_path = vault_dir.join("files").join(encrypted_name);
    let encrypted = fs::read(&enc_path)
        .map_err(|e| format!("Cannot read encrypted file: {}", e))?;

    decrypt_data(&encrypted, key)
}

/// Remove an encrypted file from the vault
pub fn remove_vault_file(encrypted_name: &str, vault_dir: &Path) -> Result<(), String> {
    let path = vault_dir.join("files").join(encrypted_name);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Cannot remove file: {}", e))?;
    }
    Ok(())
}

/// Get the vault directory path
pub fn get_vault_dir() -> Result<PathBuf, String> {
    crate::scanner::get_scan_root().map(|p| p.join(".semanticdrive").join("vault"))
}
