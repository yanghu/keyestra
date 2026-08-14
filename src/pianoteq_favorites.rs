use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct PianoteqFavorites {
    pub full: Vec<String>,
    pub reverb: Vec<String>,
}

impl PianoteqFavorites {
    fn normalized(mut self) -> Self {
        deduplicate(&mut self.full);
        deduplicate(&mut self.reverb);
        self
    }
}

#[derive(Debug)]
pub struct PianoteqFavoriteStore {
    path: PathBuf,
    favorites: Mutex<PianoteqFavorites>,
}

impl PianoteqFavoriteStore {
    pub fn discover() -> Result<Self> {
        let appdata =
            env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
        Ok(Self::at(
            PathBuf::from(appdata)
                .join("keyestra")
                .join("pianoteq-favorites.json"),
        ))
    }

    fn at(path: PathBuf) -> Self {
        let favorites = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<PianoteqFavorites>(&text)
                .map(PianoteqFavorites::normalized)
                .unwrap_or_else(|error| {
                    eprintln!(
                        "Ignoring invalid Pianoteq favorites file {}: {}",
                        path.display(),
                        error
                    );
                    PianoteqFavorites::default()
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                PianoteqFavorites::default()
            }
            Err(error) => {
                eprintln!(
                    "Could not read Pianoteq favorites file {}: {}",
                    path.display(),
                    error
                );
                PianoteqFavorites::default()
            }
        };
        Self {
            path,
            favorites: Mutex::new(favorites),
        }
    }

    pub fn snapshot(&self) -> Result<PianoteqFavorites> {
        self.favorites
            .lock()
            .map(|favorites| favorites.clone())
            .map_err(|_| anyhow::anyhow!("Pianoteq favorites lock poisoned"))
    }

    pub fn set(&self, kind: &str, key: &str, favorite: bool) -> Result<PianoteqFavorites> {
        if key.is_empty() || key.len() > 2048 {
            anyhow::bail!("Missing or invalid Pianoteq favorite key");
        }

        let mut current = self
            .favorites
            .lock()
            .map_err(|_| anyhow::anyhow!("Pianoteq favorites lock poisoned"))?;
        let mut next = current.clone();
        let list = match kind {
            "full" => &mut next.full,
            "reverb" => &mut next.reverb,
            _ => anyhow::bail!("Pianoteq favorite kind must be full or reverb"),
        };

        if favorite {
            if !list.iter().any(|item| item == key) {
                list.push(key.to_string());
            }
        } else {
            list.retain(|item| item != key);
        }

        self.persist(&next)?;
        *current = next.clone();
        Ok(next)
    }

    fn persist(&self, favorites: &PianoteqFavorites) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(favorites)?;
        fs::write(&self.path, format!("{text}\n"))
            .with_context(|| format!("Failed to write {}", self.path.display()))
    }
}

fn deduplicate(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| !item.is_empty() && seen.insert(item.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "keyestra-{name}-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn favorites_persist_for_full_and_reverb_presets() {
        let path = temporary_path("pianoteq-favorites");
        let store = PianoteqFavoriteStore::at(path.clone());
        store.set("full", "My Presets\0My Piano", true).unwrap();
        store.set("reverb", "\0Piano room 2", true).unwrap();

        let reloaded = PianoteqFavoriteStore::at(path.clone()).snapshot().unwrap();
        assert_eq!(reloaded.full, vec!["My Presets\0My Piano"]);
        assert_eq!(reloaded.reverb, vec!["\0Piano room 2"]);

        let store = PianoteqFavoriteStore::at(path.clone());
        store.set("full", "My Presets\0My Piano", false).unwrap();
        assert!(store.snapshot().unwrap().full.is_empty());
        assert_eq!(store.snapshot().unwrap().reverb, vec!["\0Piano room 2"]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn loading_removes_empty_and_duplicate_entries() {
        let path = temporary_path("pianoteq-favorites-normalize");
        fs::write(
            &path,
            r#"{"full":["one","","one","two"],"reverb":["room","room"]}"#,
        )
        .unwrap();

        let favorites = PianoteqFavoriteStore::at(path.clone()).snapshot().unwrap();
        assert_eq!(favorites.full, vec!["one", "two"]);
        assert_eq!(favorites.reverb, vec!["room"]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_kind_does_not_write_a_file() {
        let path = temporary_path("pianoteq-favorites-invalid");
        let store = PianoteqFavoriteStore::at(path.clone());
        assert!(store.set("effect", "preset", true).is_err());
        assert!(!path.exists());
    }
}
