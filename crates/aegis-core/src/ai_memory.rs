use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_ENTRIES: usize = 2_000;
const MAX_RETRIEVAL: usize = 8;
static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Experience {
    pub id: u64,
    pub strategy: String,
    pub observation: String,
    pub lesson: String,
    pub outcome_r: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoryData {
    next_id: u64,
    experiences: Vec<Experience>,
}

pub struct MemoryStore {
    path: PathBuf,
    data: MemoryData,
}

impl MemoryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let data = match fs::read(&path) {
            Ok(bytes) => {
                let data: MemoryData = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid memory store {}: {e}", path.display()))?;
                validate_data(&data)?;
                data
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => MemoryData {
                next_id: 1,
                experiences: Vec::new(),
            },
            Err(error) => return Err(format!("cannot read memory store {}: {error}", path.display())),
        };
        Ok(Self { path, data })
    }

    pub fn list(&self) -> &[Experience] {
        &self.data.experiences
    }

    pub fn add(
        &mut self,
        strategy: &str,
        observation: &str,
        lesson: &str,
        outcome_r: Option<f64>,
    ) -> Result<Experience, String> {
        let strategy = validate_text("strategy", strategy, 80)?;
        let observation = validate_text("observation", observation, 2_000)?;
        let lesson = validate_text("lesson", lesson, 2_000)?;
        validate_outcome(outcome_r)?;

        let id = self.data.next_id;
        let next_id = id
            .checked_add(1)
            .ok_or_else(|| "experience id space exhausted".to_string())?;
        let experience = Experience {
            id,
            strategy,
            observation,
            lesson,
            outcome_r,
        };
        let mut data = MemoryData {
            next_id,
            experiences: self.data.experiences.clone(),
        };
        data.experiences.push(experience.clone());
        if data.experiences.len() > MAX_ENTRIES {
            data.experiences.remove(0);
        }
        self.commit(data)?;
        Ok(experience)
    }

    pub fn delete(&mut self, id: u64) -> Result<(), String> {
        let mut data = MemoryData {
            next_id: self.data.next_id,
            experiences: self.data.experiences.clone(),
        };
        let Some(index) = data.experiences.iter().position(|entry| entry.id == id) else {
            return Err(format!("experience {id} not found"));
        };
        data.experiences.remove(index);
        self.commit(data)
    }

    pub fn clear(&mut self) -> Result<(), String> {
        self.commit(MemoryData {
            next_id: self.data.next_id,
            experiences: Vec::new(),
        })
    }

    pub fn relevant(&self, strategy: &str, query: &str, limit: usize) -> Vec<Experience> {
        let strategy = strategy.trim().to_lowercase();
        let query_tokens = tokens(query);
        let mut ranked: Vec<_> = self
            .data
            .experiences
            .iter()
            .map(|experience| {
                let same_strategy = experience.strategy.to_lowercase() == strategy;
                let overlap = token_overlap(
                    &query_tokens,
                    &tokens(&format!("{} {}", experience.observation, experience.lesson)),
                );
                (experience, same_strategy, overlap)
            })
            .filter(|(_, same_strategy, overlap)| *same_strategy || *overlap > 0)
            .collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| b.0.id.cmp(&a.0.id))
        });
        ranked
            .into_iter()
            .take(limit.min(MAX_RETRIEVAL))
            .map(|(experience, _, _)| experience.clone())
            .collect()
    }

    pub fn export_jsonl(&self) -> String {
        self.data
            .experiences
            .iter()
            .map(|experience| {
                serde_json::json!({
                    "instruction": "Use this recorded experience as context; it is not a guarantee of future outcomes.",
                    "input": {
                        "strategy": experience.strategy,
                        "observation": experience.observation,
                        "outcome_r": experience.outcome_r,
                    },
                    "output": experience.lesson,
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn commit(&mut self, data: MemoryData) -> Result<(), String> {
        persist(&self.path, &data)?;
        self.data = data;
        Ok(())
    }
}

fn validate_text(field: &str, value: &str, max_chars: usize) -> Result<String, String> {
    let value = value.trim();
    let length = value.chars().count();
    if length == 0 {
        return Err(format!("{field} must not be empty"));
    }
    if length > max_chars {
        return Err(format!("{field} must be at most {max_chars} characters"));
    }
    Ok(value.to_string())
}

fn validate_outcome(outcome_r: Option<f64>) -> Result<(), String> {
    if let Some(value) = outcome_r {
        if !value.is_finite() || !(-1_000.0..=1_000.0).contains(&value) {
            return Err("outcome_r must be finite and between -1000 and 1000".to_string());
        }
    }
    Ok(())
}

fn validate_data(data: &MemoryData) -> Result<(), String> {
    if data.experiences.len() > MAX_ENTRIES {
        return Err(format!("memory store exceeds {MAX_ENTRIES} experiences"));
    }
    let mut previous_id = 0;
    for experience in &data.experiences {
        if experience.id == 0 || experience.id <= previous_id {
            return Err("memory store experience ids are invalid or out of order".to_string());
        }
        validate_text("strategy", &experience.strategy, 80)?;
        validate_text("observation", &experience.observation, 2_000)?;
        validate_text("lesson", &experience.lesson, 2_000)?;
        validate_outcome(experience.outcome_r)?;
        previous_id = experience.id;
    }
    if data.next_id == 0 || data.next_id <= previous_id {
        return Err("memory store next id is invalid".to_string());
    }
    Ok(())
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn token_overlap(query: &HashSet<String>, content: &HashSet<String>) -> usize {
    query.intersection(content).count()
}

fn persist(path: &Path, data: &MemoryData) -> Result<(), String> {
    let bytes = serde_json::to_vec(data).map_err(|e| format!("cannot encode memory store: {e}"))?;
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create memory store directory: {e}"))?;
    }
    let temp_path = temporary_path(path);
    let write_result =
        write_temp(&temp_path, &bytes).and_then(|()| fs::rename(&temp_path, path).map_err(|e| e.to_string()));
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("cannot persist memory store {}: {error}", path.display()));
    }
    if let Some(parent) = parent {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn temporary_path(path: &Path) -> PathBuf {
    let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}-{id}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestPath(PathBuf);

    impl TestPath {
        fn new() -> Self {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            Self(std::env::temp_dir().join(format!("aegis-ai-memory-{}-{unique}.json", std::process::id())))
        }

        fn as_path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn persists_and_reopens_experiences() {
        let path = TestPath::new();
        let mut store = MemoryStore::open(path.as_path()).unwrap();
        let experience = store
            .add(
                "mean reversion",
                "  price returned to range  ",
                "wait for confirmation",
                Some(1.5),
            )
            .unwrap();
        assert_eq!(experience.id, 1);
        let reopened = MemoryStore::open(path.as_path()).unwrap();
        assert_eq!(reopened.list(), &[experience]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path.as_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn corrupt_store_is_an_error() {
        let path = TestPath::new();
        fs::write(path.as_path(), b"not json").unwrap();
        assert!(MemoryStore::open(path.as_path()).is_err());
    }

    #[test]
    fn validates_and_trims_new_entries() {
        let path = TestPath::new();
        let mut store = MemoryStore::open(path.as_path()).unwrap();
        let experience = store.add(" trend ", " setup ", " lesson ", None).unwrap();
        assert_eq!(experience.strategy, "trend");
        assert_eq!(experience.observation, "setup");
        assert!(store.add(" ", "setup", "lesson", None).is_err());
        assert!(store.add("trend", "setup", "lesson", Some(f64::NAN)).is_err());
        assert!(store.add("trend", "setup", "lesson", Some(1_000.1)).is_err());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn deletes_clears_and_keeps_ids_monotonic() {
        let path = TestPath::new();
        let mut store = MemoryStore::open(path.as_path()).unwrap();
        let first = store.add("trend", "a", "b", None).unwrap();
        store.delete(first.id).unwrap();
        assert!(store.delete(first.id).is_err());
        let second = store.add("trend", "c", "d", None).unwrap();
        assert!(second.id > first.id);
        store.clear().unwrap();
        assert!(store.list().is_empty());
        assert!(store.add("trend", "e", "f", None).unwrap().id > second.id);
    }

    #[test]
    fn bounds_store_and_discards_oldest_entry() {
        let path = TestPath::new();
        let data = MemoryData {
            next_id: MAX_ENTRIES as u64 + 1,
            experiences: (1..=MAX_ENTRIES as u64)
                .map(|id| Experience {
                    id,
                    strategy: "trend".to_string(),
                    observation: format!("observation {id}"),
                    lesson: "lesson".to_string(),
                    outcome_r: None,
                })
                .collect(),
        };
        fs::write(path.as_path(), serde_json::to_vec(&data).unwrap()).unwrap();

        let mut store = MemoryStore::open(path.as_path()).unwrap();
        let added = store.add("trend", "new", "new lesson", None).unwrap();
        assert_eq!(added.id, MAX_ENTRIES as u64 + 1);
        assert_eq!(store.list().len(), MAX_ENTRIES);
        assert_eq!(store.list()[0].id, 2);
        assert_eq!(store.list().last().unwrap().id, added.id);
    }

    #[test]
    fn retrieval_ranks_strategy_and_overlap_and_export_is_jsonl() {
        let path = TestPath::new();
        let mut store = MemoryStore::open(path.as_path()).unwrap();
        let other = store.add("breakout", "volume spike", "avoid chasing", None).unwrap();
        let matching = store
            .add("trend", "momentum confirmation", "enter after pullback", Some(0.5))
            .unwrap();
        let overlap = store
            .add("range", "momentum confirmation", "wait for pullback", None)
            .unwrap();
        let relevant = store.relevant("trend", "momentum pullback", 20);
        assert_eq!(relevant.len(), 2);
        assert_eq!(relevant[0].id, matching.id);
        assert_eq!(relevant[1].id, overlap.id);
        assert!(!relevant.iter().any(|e| e.id == other.id));
        assert!(store.relevant("unknown", "unrelated", 3).is_empty());
        assert_eq!(store.relevant("trend", "momentum", 1)[0].id, matching.id);

        let lines = store.export_jsonl();
        let records: Vec<serde_json::Value> = lines.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[1]["input"]["strategy"], "trend");
        assert_eq!(records[1]["output"], "enter after pullback");
        assert!(records[1]["instruction"].as_str().unwrap().contains("not a guarantee"));
    }
}
