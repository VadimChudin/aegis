//! Settings file (`settings.json` in the app config dir).
//!
//! Every credential field is stored encrypted (AES-256-GCM, `enc:v1:` prefix)
//! with a key derived from this computer's host and user name, so a copied
//! file does not open elsewhere. It does not protect against software running
//! as the same user. Secret fields never leave this module towards the window.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
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
        Self::open_with_key(path, machine_key())
    }

    pub fn open_with_key(path: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        let path = path.into();
        let data = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("settings: {e}; starting from defaults");
                let _ = fs::rename(&path, path.with_extension("json.bad"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        SettingsStore { path, cipher, data }
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
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
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
        let blob = hex::decode(stored.strip_prefix(PREFIX)?).ok()?;
        if blob.len() < 12 {
            return None;
        }
        let (nonce, sealed) = blob.split_at(12);
        let plain = self.cipher.decrypt(Nonce::from_slice(nonce), sealed).ok()?;
        String::from_utf8(plain).ok()
    }
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
