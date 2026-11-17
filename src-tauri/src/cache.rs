//! Small on-disk TTL cache shared by the resolver.
//!
//! Search rows, author pages and detail summaries are immutable for a short
//! while, so a repeated resolve or a mirror hand-off should not spawn the
//! crawler or refetch a page. Entries store their own key so a hash collision
//! can never return another query's rows.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct Envelope<T> {
    key: String,
    fetched_at: u64,
    value: T,
}

/// FNV-1a keeps cache filenames stable across releases without adding a
/// hashing dependency.
pub(crate) fn key_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{:016x}.json", key_hash(key)))
}

/// Resolve the cache directory from an environment override, otherwise the
/// app data directory. `leaf` names the cache (for example `search-cache`).
pub fn directory(env_key: &str, leaf: &str) -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os(env_key) {
        return Some(PathBuf::from(configured));
    }
    dirs::data_dir().map(|base| base.join("novelforge").join(leaf))
}

/// Read a positive TTL override from the environment.
pub fn ttl_seconds(env_key: &str, default: u64) -> u64 {
    std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

pub fn read_at<T: DeserializeOwned>(
    directory: &Path,
    key: &str,
    now: u64,
    ttl_seconds: u64,
) -> Option<(T, u64)> {
    let bytes = std::fs::read(path(directory, key)).ok()?;
    let entry: Envelope<T> = serde_json::from_slice(&bytes).ok()?;
    if entry.key != key {
        return None;
    }
    let age = now.saturating_sub(entry.fetched_at);
    if age > ttl_seconds {
        return None;
    }
    Some((entry.value, age))
}

pub fn write_at<T: Serialize>(directory: &Path, key: &str, value: &T, now: u64) {
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    let entry = Envelope {
        key: key.to_string(),
        fetched_at: now,
        value,
    };
    let Ok(json) = serde_json::to_vec(&entry) else {
        return;
    };
    let target = path(directory, key);
    let temp = target.with_extension("tmp");
    if std::fs::write(&temp, json).is_ok() {
        let _ = std::fs::rename(&temp, &target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("novelforge-cache-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn round_trips_and_expires() {
        let directory = temp_dir();
        let key = "search:novelfire.net::reverend insanity";
        write_at(
            &directory,
            key,
            &vec!["Reverend Insanity".to_string()],
            1_000,
        );

        let (value, age) =
            read_at::<Vec<String>>(&directory, key, 1_030, 3_600).expect("fresh entry");
        assert_eq!(value, vec!["Reverend Insanity".to_string()]);
        assert_eq!(age, 30);

        assert!(read_at::<Vec<String>>(&directory, key, 4_601, 3_600).is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_stored_key_never_answers_for_another_key() {
        let directory = temp_dir();
        write_at(&directory, "first", &"one".to_string(), 1_000);
        assert!(read_at::<String>(&directory, "second", 1_010, 3_600).is_none());
        let _ = std::fs::remove_dir_all(directory);
    }
}
