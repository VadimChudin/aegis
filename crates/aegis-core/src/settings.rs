//! Settings file (`settings.json` in the app config dir).
//!
//! Every credential field is stored encrypted (AES-256-GCM, `enc:v1:` prefix)
//! with a random key stored beside this file as `settings.key`. On Unix the key
//! file is owner-only; Windows uses the config directory's inherited ACL. This
//! is local-file protection, not an OS key vault, and does not protect against
//! software running as the same user. Secret fields never leave this module
//! towards the window.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    broker::{BrokerId, ConnectReport},
    market::Timeframe,
};

const PREFIX: &str = "enc:v1:";
pub const THEMES: [&str; 3] = ["glass-dark", "glass-light", "glass-blue"];
pub const LANGS: [&str; 3] = ["en", "ru", "kk"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrokerSettings {
    #[serde(default)]
    pub auto_connect: bool,
    /// Field key → encrypted value.
    #[serde(default)]
    fields: BTreeMap<String, String>,
    #[serde(default)]
    pub last_report: Option<ConnectReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub chart_broker: Option<BrokerId>,
    #[serde(default = "default_timeframe")]
    pub timeframe: Timeframe,
    /// Interface language: "en", "ru" or "kk"; empty = follow the system.
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    brokers: BTreeMap<BrokerId, BrokerSettings>,
    /// Strategy id → its settings (sliders and toggles), as JSON.
    #[serde(default)]
    pub strategies: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    ai_key: Option<String>,
}

fn default_theme() -> String {
    THEMES[0].into()
}

fn default_timeframe() -> Timeframe {
    Timeframe::M15
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: default_theme(),
            chart_broker: None,
            timeframe: default_timeframe(),
            lang: String::new(),
            brokers: BTreeMap::new(),
            strategies: BTreeMap::new(),
            ai_key: None,
        }
    }
}

/// What the window may see about one broker.
#[derive(Clone, Debug, Serialize)]
pub struct PublicBroker {
    /// Non-secret fields only (login, server, API key).
    pub values: BTreeMap<String, String>,
    /// Secret fields that are stored; the window shows "Stored".
    pub stored: Vec<String>,
    pub auto_connect: bool,
    pub last_report: Option<ConnectReport>,
    /// A stored value could not be decrypted (file copied from another computer).
    pub unreadable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicSettings {
    pub theme: String,
    pub lang: String,
    pub chart_broker: Option<BrokerId>,
    pub timeframe: Timeframe,
    pub brokers: BTreeMap<BrokerId, PublicBroker>,
}

pub struct SettingsStore {
    path: PathBuf,
    cipher: Aes256Gcm,
    data: Settings,
    key_error: Option<String>,
}

fn machine_key() -> [u8; 32] {
    let host = whoami::fallible::hostname().unwrap_or_default();
    let user = whoami::username();
    let mut h = Sha256::new();
    h.update(b"aegis.settings.v1\0");
    h.update(host.as_bytes());
    h.update(b"\0");
    h.update(user.as_bytes());
    h.finalize().into()
}

impl SettingsStore {
    /// Loads the file; a missing file gives defaults, a corrupt one is kept as `.bad`.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (data, settings_error) = read_settings(&path);
        let settings_existed = path.exists();
        let (key, created, key_error) = match load_or_create_key(&path) {
            Ok((key, created)) => (key, created, None),
            Err(error) => ([0; 32], false, Some(error.to_string())),
        };
        let mut store = SettingsStore {
            path,
            cipher: cipher_for(&key),
            data,
            key_error,
        };

        if settings_existed && created && store.key_error.is_none() {
            let legacy_cipher = cipher_for(&machine_key());
            if store.migrate_legacy(&legacy_cipher).is_err() {
                store.key_error = Some("legacy credentials could not be migrated".into());
            } else if store.has_encrypted_values() {
                if let Err(error) = store.save() {
                    store.key_error = Some(format!("could not save migrated settings: {error}"));
                }
            }
        } else if store.key_error.is_none() && store.has_unreadable_secrets() {
            store.key_error = Some("stored credentials could not be decrypted".into());
        }
        if let Some(error) = settings_error {
            log::warn!("settings: {error}");
        }
        store
    }

    pub fn open_with_key(path: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        let path = path.into();
        let (data, _) = read_settings(&path);
        SettingsStore {
            path,
            cipher: cipher_for(&key),
            data,
            key_error: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn settings(&self) -> &Settings {
        &self.data
    }

    pub fn set_theme(&mut self, theme: &str) {
        if THEMES.contains(&theme) {
            self.data.theme = theme.into();
        }
    }

    pub fn set_lang(&mut self, lang: &str) {
        if LANGS.contains(&lang) {
            self.data.lang = lang.into();
        }
    }

    pub fn set_strategy(&mut self, id: &str, value: serde_json::Value) {
        self.data.strategies.insert(id.to_string(), value);
    }

    pub fn strategy(&self, id: &str) -> Option<&serde_json::Value> {
        self.data.strategies.get(id)
    }

    pub fn ai_key(&self) -> Option<String> {
        self.data.ai_key.as_deref().and_then(|value| self.decrypt(value))
    }

    pub fn set_ai_key(&mut self, value: &str) {
        self.data.ai_key = Some(self.encrypt(value));
    }

    pub fn forget_ai_key(&mut self) {
        self.data.ai_key = None;
    }

    pub fn set_chart(&mut self, broker: Option<BrokerId>, timeframe: Timeframe) {
        self.data.chart_broker = broker;
        self.data.timeframe = timeframe;
    }

    pub fn broker(&self, id: BrokerId) -> Option<&BrokerSettings> {
        self.data.brokers.get(&id)
    }

    fn entry(&mut self, id: BrokerId) -> &mut BrokerSettings {
        self.data.brokers.entry(id).or_default()
    }

    pub fn has_credentials(&self, id: BrokerId) -> bool {
        self.broker(id).is_some_and(|b| !b.fields.is_empty())
    }

    /// Decrypted fields. Values that cannot be decrypted are left out.
    pub fn credentials(&self, id: BrokerId) -> BTreeMap<String, String> {
        let Some(b) = self.broker(id) else {
            return BTreeMap::new();
        };
        b.fields
            .iter()
            .filter_map(|(k, v)| self.decrypt(v).map(|v| (k.clone(), v)))
            .collect()
    }

    /// Merges a submitted form into the stored fields: a filled field replaces
    /// the stored value, an empty secret keeps it, an empty plain field clears it.
    /// Returns the merged, decrypted fields without saving them.
    pub fn merged_form(&self, id: BrokerId, form: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut merged = self.credentials(id);
        for f in id.info().fields {
            let value = form.get(f.key).map(|v| v.trim()).unwrap_or("");
            if !value.is_empty() {
                merged.insert(f.key.into(), value.into());
            } else if !f.secret {
                merged.remove(f.key);
            }
        }
        merged
    }

    /// Returns the merged, decrypted fields and saves them.
    pub fn apply_form(&mut self, id: BrokerId, form: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let merged = self.merged_form(id, form);
        let encrypted = merged.iter().map(|(k, v)| (k.clone(), self.encrypt(v))).collect();
        self.entry(id).fields = encrypted;
        merged
    }

    pub fn set_auto_connect(&mut self, id: BrokerId, on: bool) {
        self.entry(id).auto_connect = on;
    }

    pub fn set_report(&mut self, id: BrokerId, report: ConnectReport) {
        self.entry(id).last_report = Some(report);
    }

    /// Removes the stored credentials, the last report and auto-connect.
    pub fn forget(&mut self, id: BrokerId) {
        self.data.brokers.remove(&id);
        if self.data.chart_broker == Some(id) {
            self.data.chart_broker = None;
        }
    }

    pub fn public(&self) -> PublicSettings {
        let brokers = BrokerId::ALL
            .into_iter()
            .map(|id| {
                let stored = self.broker(id).cloned().unwrap_or_default();
                let plain = self.credentials(id);
                let mut values = BTreeMap::new();
                let mut secrets = Vec::new();
                for (k, v) in &plain {
                    if id.is_secret(k) {
                        secrets.push(k.clone());
                    } else {
                        values.insert(k.clone(), v.clone());
                    }
                }
                let broker = PublicBroker {
                    values,
                    stored: secrets,
                    auto_connect: stored.auto_connect,
                    last_report: stored.last_report,
                    unreadable: plain.len() < stored.fields.len(),
                };
                (id, broker)
            })
            .collect();
        PublicSettings {
            theme: self.data.theme.clone(),
            lang: self.data.lang.clone(),
            chart_broker: self.data.chart_broker,
            timeframe: self.data.timeframe,
            brokers,
        }
    }

    /// Atomic write (temp file + rename), readable by the owner only.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(error) = &self.key_error {
            return Err(io::Error::new(io::ErrorKind::InvalidData, error.clone()));
        }
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(serde_json::to_string_pretty(&self.data)?.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)
    }

    fn encrypt(&self, plain: &str) -> String {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let sealed = self
            .cipher
            .encrypt(&nonce, plain.as_bytes())
            .expect("AES-GCM encryption does not fail");
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&sealed);
        format!("{PREFIX}{}", hex::encode(blob))
    }

    fn decrypt(&self, stored: &str) -> Option<String> {
        decrypt_with(&self.cipher, stored)
    }

    fn has_encrypted_values(&self) -> bool {
        self.data
            .brokers
            .values()
            .any(|broker| broker.fields.values().any(|value| value.starts_with(PREFIX)))
            || self
                .data
                .ai_key
                .as_deref()
                .is_some_and(|value| value.starts_with(PREFIX))
    }

    fn has_unreadable_secrets(&self) -> bool {
        self.data
            .brokers
            .values()
            .any(|broker| broker.fields.values().any(|value| self.decrypt(value).is_none()))
            || self
                .data
                .ai_key
                .as_deref()
                .is_some_and(|value| self.decrypt(value).is_none())
    }

    fn migrate_legacy(&mut self, legacy_cipher: &Aes256Gcm) -> Result<(), ()> {
        let mut migrated = self.data.clone();
        for broker in migrated.brokers.values_mut() {
            for value in broker.fields.values_mut() {
                let plain = if value.starts_with(PREFIX) {
                    decrypt_with(legacy_cipher, value).ok_or(())?
                } else {
                    value.clone()
                };
                *value = encrypt_with(&self.cipher, &plain);
            }
        }
        if let Some(value) = migrated.ai_key.as_mut() {
            let plain = if value.starts_with(PREFIX) {
                decrypt_with(legacy_cipher, value).ok_or(())?
            } else {
                value.clone()
            };
            *value = encrypt_with(&self.cipher, &plain);
        }
        self.data = migrated;
        Ok(())
    }
}

fn read_settings(path: &Path) -> (Settings, Option<String>) {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(data) => (data, None),
            Err(error) => {
                let _ = fs::rename(path, path.with_extension("json.bad"));
                (Settings::default(), Some(format!("{error}; starting from defaults")))
            }
        },
        Err(_) => (Settings::default(), None),
    }
}

fn load_or_create_key(settings_path: &Path) -> io::Result<([u8; 32], bool)> {
    let path = settings_path.with_file_name("settings.key");
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let key: [u8; 32] = Aes256Gcm::generate_key(&mut OsRng).into();
            file.write_all(&key)?;
            file.sync_all()?;
            if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                if let Ok(dir) = fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
            Ok((key, true))
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            for attempt in 0..10 {
                let read_key = (|| {
                    let mut bytes = Vec::new();
                    fs::File::open(&path)?.read_to_end(&mut bytes)?;
                    if bytes.len() != 32 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "settings key must be 32 bytes",
                        ));
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                    }
                    let mut key = [0; 32];
                    key.copy_from_slice(&bytes);
                    Ok(key)
                })();
                match read_key {
                    Ok(key) => return Ok((key, false)),
                    Err(error) if error.kind() == io::ErrorKind::InvalidData && attempt < 9 => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => return Err(error),
                }
            }
            unreachable!()
        }
        Err(error) => Err(error),
    }
}

fn cipher_for(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
}

fn encrypt_with(cipher: &Aes256Gcm, plain: &str) -> String {
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(&nonce, plain.as_bytes())
        .expect("AES-GCM encryption does not fail");
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&sealed);
    format!("{PREFIX}{}", hex::encode(blob))
}

fn decrypt_with(cipher: &Aes256Gcm, stored: &str) -> Option<String> {
    let blob = hex::decode(stored.strip_prefix(PREFIX)?).ok()?;
    if blob.len() < 12 {
        return None;
    }
    let (nonce, sealed) = blob.split_at(12);
    let plain = cipher.decrypt(Nonce::from_slice(nonce), sealed).ok()?;
    String::from_utf8(plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aegis-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn ai_key_is_encrypted_private_and_forgettable() {
        let path = tmp("ai-key");
        let mut store = SettingsStore::open_with_key(&path, [11; 32]);
        store.set_ai_key("example-not-a-real-provider-key");
        store.save().unwrap();
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("example-not-a-real-provider-key"));
        assert!(!serde_json::to_string(&store.public())
            .unwrap()
            .contains("example-not-a-real-provider-key"));
        assert_eq!(
            SettingsStore::open_with_key(&path, [11; 32]).ai_key().as_deref(),
            Some("example-not-a-real-provider-key")
        );
        assert!(SettingsStore::open_with_key(&path, [12; 32]).ai_key().is_none());
        store.forget_ai_key();
        store.save().unwrap();
        assert!(SettingsStore::open_with_key(&path, [11; 32]).ai_key().is_none());
    }

    #[test]
    fn credentials_round_trip_encrypted_on_disk() {
        let path = tmp("roundtrip");
        let mut s = SettingsStore::open_with_key(&path, [7; 32]);
        s.apply_form(
            BrokerId::Binance,
            &form(&[("api_key", "KEY-abc"), ("api_secret", "SECRET-xyz")]),
        );
        s.set_auto_connect(BrokerId::Binance, true);
        s.set_chart(Some(BrokerId::Binance), Timeframe::H1);
        s.save().unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("KEY-abc") && !text.contains("SECRET-xyz"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }

        let again = SettingsStore::open_with_key(&path, [7; 32]);
        assert_eq!(again.credentials(BrokerId::Binance)["api_secret"], "SECRET-xyz");
        assert_eq!(again.settings().timeframe, Timeframe::H1);
        assert!(again.broker(BrokerId::Binance).unwrap().auto_connect);
    }

    #[test]
    fn public_view_never_contains_secrets() {
        let mut s = SettingsStore::open_with_key(tmp("public"), [1; 32]);
        s.apply_form(BrokerId::Bybit, &form(&[("api_key", "K"), ("api_secret", "TOPSECRET")]));
        let p = s.public();
        let bybit = &p.brokers[&BrokerId::Bybit];
        assert_eq!(bybit.values.get("api_key").map(String::as_str), Some("K"));
        assert_eq!(bybit.stored, vec!["api_secret".to_string()]);
        assert!(!serde_json::to_string(&p).unwrap().contains("TOPSECRET"));
        assert!(p.brokers[&BrokerId::Roboforex].values.is_empty());
    }

    #[test]
    fn empty_secret_keeps_stored_value_empty_plain_field_clears() {
        let mut s = SettingsStore::open_with_key(tmp("merge"), [2; 32]);
        s.apply_form(
            BrokerId::Roboforex,
            &form(&[
                ("login", "1"),
                ("password", "p"),
                ("server", "RoboForex-ECN"),
                ("python", "py"),
            ]),
        );
        let merged = s.apply_form(
            BrokerId::Roboforex,
            &form(&[("login", "2"), ("password", ""), ("server", "S")]),
        );
        assert_eq!(merged["login"], "2");
        assert_eq!(merged["password"], "p");
        assert!(!merged.contains_key("python"));
    }

    #[test]
    fn other_machine_key_cannot_read_and_is_flagged() {
        let path = tmp("otherkey");
        let mut s = SettingsStore::open_with_key(&path, [3; 32]);
        s.apply_form(BrokerId::Binance, &form(&[("api_key", "K"), ("api_secret", "S")]));
        s.save().unwrap();
        let other = SettingsStore::open_with_key(&path, [4; 32]);
        assert!(other.credentials(BrokerId::Binance).is_empty());
        assert!(other.public().brokers[&BrokerId::Binance].unreadable);
    }

    #[test]
    fn open_uses_persistent_random_local_key() {
        let path = tmp("local-key");
        let other_path = tmp("local-key-other");
        let mut store = SettingsStore::open(&path);
        store.apply_form(BrokerId::Binance, &form(&[("api_key", "local-key-value")]));
        store.set_ai_key("local-ai-key");
        store.save().unwrap();

        let key_path = path.with_file_name("settings.key");
        let key = fs::read(&key_path).unwrap();
        assert_eq!(key.len(), 32);
        assert_ne!(key, machine_key());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&key_path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let reopened = SettingsStore::open(&path);
        assert_eq!(reopened.credentials(BrokerId::Binance)["api_key"], "local-key-value");
        assert_eq!(reopened.ai_key().as_deref(), Some("local-ai-key"));

        SettingsStore::open(&other_path);
        assert_ne!(key, fs::read(other_path.with_file_name("settings.key")).unwrap());
    }

    #[test]
    fn missing_local_key_migrates_legacy_machine_encrypted_credentials() {
        let path = tmp("legacy-key");
        let mut legacy = SettingsStore::open_with_key(&path, machine_key());
        legacy.apply_form(BrokerId::Binance, &form(&[("api_key", "legacy-api-key")]));
        legacy.set_ai_key("legacy-ai-key");
        legacy.save().unwrap();
        let old_text = fs::read_to_string(&path).unwrap();

        let migrated = SettingsStore::open(&path);
        assert_eq!(migrated.credentials(BrokerId::Binance)["api_key"], "legacy-api-key");
        assert_eq!(migrated.ai_key().as_deref(), Some("legacy-ai-key"));
        assert_ne!(fs::read_to_string(&path).unwrap(), old_text);
        assert_eq!(
            SettingsStore::open(&path).credentials(BrokerId::Binance)["api_key"],
            "legacy-api-key"
        );
    }

    #[test]
    fn corrupt_local_key_marks_credentials_unreadable_and_blocks_save() {
        let path = tmp("corrupt-key");
        let mut original = SettingsStore::open_with_key(&path, [9; 32]);
        original.apply_form(BrokerId::Binance, &form(&[("api_key", "keep-this-ciphertext")]));
        original.save().unwrap();
        let settings_before = fs::read(&path).unwrap();
        let key_path = path.with_file_name("settings.key");
        fs::write(&key_path, [0; 32]).unwrap();

        let broken = SettingsStore::open(&path);
        assert!(broken.credentials(BrokerId::Binance).is_empty());
        assert!(broken.public().brokers[&BrokerId::Binance].unreadable);
        assert!(broken.save().is_err());
        assert_eq!(fs::read(&path).unwrap(), settings_before);
        assert_eq!(fs::read(&key_path).unwrap(), [0; 32]);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults_and_is_kept() {
        let path = tmp("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        let s = SettingsStore::open_with_key(&path, [5; 32]);
        assert_eq!(s.settings().theme, "glass-dark");
        assert!(path.with_extension("json.bad").exists());
    }

    #[test]
    fn forget_clears_chart_broker() {
        let mut s = SettingsStore::open_with_key(tmp("forget"), [6; 32]);
        s.apply_form(BrokerId::Bybit, &form(&[("api_key", "K"), ("api_secret", "S")]));
        s.set_chart(Some(BrokerId::Bybit), Timeframe::M5);
        s.forget(BrokerId::Bybit);
        assert!(!s.has_credentials(BrokerId::Bybit));
        assert_eq!(s.settings().chart_broker, None);
    }

    #[test]
    fn unknown_theme_is_ignored() {
        let mut s = SettingsStore::open_with_key(tmp("theme"), [8; 32]);
        s.set_theme("glass-blue");
        s.set_theme("neon");
        assert_eq!(s.settings().theme, "glass-blue");
    }
}
