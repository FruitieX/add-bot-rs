use crate::{
    command::RecentFormOptions,
    state::{State, STATE_SCHEMA_VERSION},
    types::Username,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use color_eyre::{eyre::eyre, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, RwLock};

const STATE_FILE_PATH: &str = "state.json";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct SaveFailure {
    pub committed: bool,
    error: std::io::Error,
}
impl std::fmt::Display for SaveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "State save failed (committed={}): {}",
            self.committed, self.error
        )
    }
}
impl std::error::Error for SaveFailure {}

fn save_atomic(path: &Path, bytes: &[u8]) -> std::result::Result<(), SaveFailure> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(
        ".state.{}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut committed = false;
    let mut created = false;
    let result = (|| {
        let directory = std::fs::File::open(parent)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        committed = true;
        directory.sync_all()
    })();
    if created && !committed {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| SaveFailure { committed, error })
}

#[derive(Clone)]
pub struct StateContainer {
    state: Arc<RwLock<State>>,
    path: Arc<PathBuf>,
    pub delivery_lock: Arc<Mutex<()>>,
}

impl StateContainer {
    pub async fn try_read_from_file(tz: Tz) -> Result<Self> {
        Self::load(PathBuf::from(STATE_FILE_PATH), tz, Utc::now()).await
    }

    pub async fn load(path: PathBuf, tz: Tz, now: DateTime<Utc>) -> Result<Self> {
        let state = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                eyre!(
                    "Cannot parse {}: {error}; existing state was not changed",
                    path.display()
                )
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State {
                schema_version: STATE_SCHEMA_VERSION,
                ..Default::default()
            },
            Err(error) => {
                return Err(eyre!(
                    "Cannot read {}: {error}; refusing to start with empty state",
                    path.display()
                ))
            }
        };
        let container = Self {
            state: Arc::new(RwLock::new(state)),
            path: Arc::new(path),
            delivery_lock: Arc::new(Mutex::new(())),
        };
        container
            .transact(move |state| state.migrate(tz, now))
            .await?;
        Ok(container)
    }

    pub async fn read(&self) -> State {
        self.state.read().await.clone()
    }

    /// Edit the latest state under one lock; save before publishing memory.
    /// An owned task completes even if the caller is cancelled during I/O.
    pub async fn transact<R: Send + 'static>(
        &self,
        mutation: impl FnOnce(&mut State) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut current = this.state.write().await;
            let mut next = current.clone();
            let result = mutation(&mut next)?;
            if next == *current {
                return Ok(result);
            }
            let bytes = serde_json::to_vec(&next)?;
            let path = this.path.clone();
            let saved = tokio::task::spawn_blocking(move || save_atomic(&path, &bytes)).await?;
            match saved {
                Ok(()) => {
                    *current = next;
                    Ok(result)
                }
                Err(error) => {
                    if error.committed {
                        *current = next;
                    }
                    Err(error.into())
                }
            }
        })
        .await?
    }

    pub async fn resolve_recent_form_options(
        &self,
        requester: Option<&Username>,
        target: &Username,
        options: RecentFormOptions,
    ) -> Result<RecentFormOptions> {
        let requester = requester.cloned();
        let target = target.clone();
        self.transact(move |state| {
            Ok(state
                .resolve_recent_form_options(requester.as_ref(), &target, options)
                .0)
        })
        .await
    }
}

pub fn save_failure_message(error: &color_eyre::Report) -> &'static str {
    if error
        .downcast_ref::<SaveFailure>()
        .is_some_and(|error| error.committed)
    {
        "This change was saved, but its durability couldn’t be confirmed. Avoid repeating the command; ask an admin to check storage."
    } else {
        "This change couldn’t be saved and wasn’t applied. Try again shortly; ask an admin to check storage if it continues."
    }
}

#[cfg(test)]
pub struct TestDirectory(pub PathBuf);
#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[cfg(test)]
pub async fn test_store() -> (StateContainer, TestDirectory) {
    let directory = std::env::temp_dir().join(format!(
        "add-bot-state-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    let store = StateContainer::load(directory.join("state.json"), chrono_tz::UTC, Utc::now())
        .await
        .unwrap();
    (store, TestDirectory(directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{command::RecentFormStyle, state::Queue, types::QueueId};
    use teloxide::types::ChatId;

    #[tokio::test]
    async fn concurrent_queue_and_preference_changes_survive_in_memory_and_on_disk() {
        let (sc, directory) = test_store().await;
        let mut tasks = Vec::new();
        for index in 0..40 {
            let sc = sc.clone();
            tasks.push(tokio::spawn(async move {
                sc.transact(move |state| {
                    let (next, _, _) = state.add_remove_player(
                        &ChatId(1),
                        &QueueId::new("20:00".into()),
                        "/2000".into(),
                        chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
                        Username::new(format!("Player{index}")),
                    );
                    *state = next;
                    Ok(())
                })
                .await
                .unwrap();
            }));
        }
        for index in 0..20 {
            let sc = sc.clone();
            tasks.push(tokio::spawn(async move {
                let user = Username::new(format!("User{index}"));
                sc.resolve_recent_form_options(
                    Some(&user),
                    &user,
                    RecentFormOptions {
                        style: RecentFormStyle::Moon,
                        explicit_style: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let state = sc.read().await;
        assert_eq!(
            state.chats[&ChatId(1)].queues[&QueueId::new("20:00".into())].num_players(),
            40
        );
        assert_eq!(state.recent_form_styles.len(), 20);
        let disk: State =
            serde_json::from_slice(&std::fs::read(directory.0.join("state.json")).unwrap())
                .unwrap();
        assert!(disk == state);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn failed_save_rolls_back_mutation_and_preserves_existing_file() {
        let (sc, directory) = test_store().await;
        sc.transact(|state| {
            state.next_notification_id = 8;
            Ok(())
        })
        .await
        .unwrap();
        let original = std::fs::read(directory.0.join("state.json")).unwrap();
        let failing = StateContainer {
            state: sc.state.clone(),
            path: Arc::new(directory.0.clone()),
            delivery_lock: sc.delivery_lock.clone(),
        };
        let error = failing
            .transact(|state| {
                state.next_notification_id = 9;
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(!error.downcast_ref::<SaveFailure>().unwrap().committed);
        assert_eq!(sc.read().await.next_notification_id, 8);
        assert_eq!(
            std::fs::read(directory.0.join("state.json")).unwrap(),
            original
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn malformed_unreadable_and_future_state_are_not_silently_discarded() {
        let (_, directory) = test_store().await;
        let path = directory.0.join("state.json");
        for bytes in [
            b"{broken".as_slice(),
            br#"{"chats":{},"schema_version":99}"#.as_slice(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(
                StateContainer::load(path.clone(), chrono_tz::UTC, Utc::now())
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        assert!(
            StateContainer::load(directory.0.clone(), chrono_tz::UTC, Utc::now())
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_caller_does_not_interrupt_a_started_transaction() {
        let (sc, directory) = test_store().await;
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = started.clone();
        let store = sc.clone();
        let task = tokio::spawn(async move {
            store
                .transact(move |state| {
                    signal.store(true, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    state.next_notification_id = 17;
                    Ok(())
                })
                .await
        });
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        task.abort();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if sc.read().await.next_notification_id == 17 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let disk: State =
            serde_json::from_slice(&std::fs::read(directory.0.join("state.json")).unwrap())
                .unwrap();
        assert_eq!(disk.next_notification_id, 17);
    }

    #[tokio::test]
    async fn legacy_state_migration_preserves_players_and_persists_absolute_deadlines() {
        let (_, directory) = test_store().await;
        let path = directory.0.join("state.json");
        let mut state = State::default();
        let mut queue = Queue::new(
            chrono::NaiveTime::from_hms_opt(19, 30, 0).unwrap(),
            "/1930".into(),
        );
        queue.insert_player(Username::new("Alice".into()));
        state
            .chats
            .entry(ChatId(1))
            .or_default()
            .queues
            .insert(QueueId::new("19:30".into()), queue.clone());
        state
            .chats
            .get_mut(&ChatId(1))
            .unwrap()
            .queues
            .insert(QueueId::new("".into()), queue);
        std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-30T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let sc = StateContainer::load(path.clone(), chrono_tz::Europe::Helsinki, now)
            .await
            .unwrap();
        let state = sc.read().await;
        assert_eq!(state.schema_version, 1);
        let chat = &state.chats[&ChatId(1)];
        assert_eq!(
            chat.queues[&QueueId::new("".into())].expires_at,
            Some(now + chrono::Duration::minutes(30))
        );
        assert_eq!(
            chat.queues[&QueueId::new("19:30".into())]
                .expires_at
                .unwrap()
                .to_rfc3339(),
            "2026-10-01T16:30:00+00:00"
        );
        assert_eq!(chat.queues[&QueueId::new("19:30".into())].num_players(), 1);
        let disk: State = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(disk == state);
    }
}
