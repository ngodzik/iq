//! The observer: as each task ends, the rules place it if they can; the
//! others wait, and once enough wait, a cheap model places them together,
//! in the background, shown the categories and tasks placed before so that
//! it keeps to them. What it spends is counted with the rest, and a day's
//! placing is capped.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iq_agent::Report;
use iq_core::{ChatModel, Delegate, Effort, ModelId};

use crate::categories::Categories;
use crate::labels::{Labels, category_of};
use crate::place;
use crate::task::{self, Task};

/// The most tasks a model places in one call.
const BATCH: usize = 20;

/// The most tasks models place in a day: past it, the rest waits for the
/// next day, so that a burst never spends much.
const MOST_A_DAY: usize = 300;

/// How far back the tasks placed before are looked for, for the model to
/// keep to, in seconds.
const BEFORE: u64 = 30 * 24 * 3600;

/// What the observer is set with.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The model that places what no rule places: a cheap one.
    pub model: ModelId,
    /// How many tasks wait before it is asked, to pay less per task with
    /// a model whose every call costs a lot of context.
    pub every: usize,
    /// The person's categories.
    pub categories: Categories,
}

/// The observer. Cheap to clone: every clone is the same.
pub struct Observer<M, D> {
    inner: Arc<Inner<M, D>>,
}

impl<M, D> Clone for Observer<M, D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<M, D> {
    provider: M,
    agents: D,
    settings: Settings,
    journal: PathBuf,
    labels: Labels,
    costs: Report,
    /// Where an agent with tools of its own asked to place runs, reading.
    directory: PathBuf,
    waiting: Mutex<Vec<Task>>,
    placing: AtomicBool,
    /// The day, in days since 1970, and the tasks placed that day.
    today: Mutex<(u64, usize)>,
}

impl<M, D> Observer<M, D>
where
    M: ChatModel + 'static,
    D: Delegate + 'static,
{
    /// An observer placing the tasks of the journal at `journal`, keeping
    /// what models give in `labels`, its costs told to `costs`.
    pub fn new(
        provider: M,
        agents: D,
        settings: Settings,
        journal: PathBuf,
        labels: Labels,
        costs: Report,
        directory: PathBuf,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                provider,
                agents,
                settings,
                journal,
                labels,
                costs,
                directory,
                waiting: Mutex::new(Vec::new()),
                placing: AtomicBool::new(false),
                today: Mutex::new((0, 0)),
            }),
        }
    }

    /// The categories it places tasks in.
    #[must_use]
    pub fn categories(&self) -> &Categories {
        &self.inner.settings.categories
    }

    /// A task ended: the rules place it, or it waits for the model. Must
    /// be called within the runtime.
    pub fn done(&self, task: Task) {
        let categories = &self.inner.settings.categories;
        if categories.is_empty() || categories.by_rule(&task).is_some() {
            return;
        }
        let waiting = {
            let mut waiting = lock(&self.inner.waiting);
            waiting.push(task);
            waiting.len()
        };
        if waiting >= self.inner.settings.every.max(1) {
            self.place_soon();
        }
    }

    /// Places the tasks of the journal since `since` that neither the rules
    /// nor a model placed yet, as when the categories changed or iq was
    /// away. Must be called within the runtime.
    pub fn catch_up(&self, since: u64) {
        let categories = &self.inner.settings.categories;
        if categories.is_empty() {
            return;
        }
        let labels = self.inner.labels.load();
        let unplaced: Vec<Task> = task::read(&self.inner.journal, since)
            .into_iter()
            .filter(|task| category_of(task, categories, &labels).is_none())
            .collect();
        if unplaced.is_empty() {
            return;
        }
        {
            let mut waiting = lock(&self.inner.waiting);
            for task in unplaced {
                if !waiting.iter().any(|w| w.id() == task.id()) {
                    waiting.push(task);
                }
            }
        }
        self.place_soon();
    }

    /// Has the waiting tasks placed in the background, unless they are.
    fn place_soon(&self) {
        if self.inner.placing.swap(true, Ordering::SeqCst) {
            return;
        }
        let observer = self.clone();
        tokio::spawn(async move {
            observer.place_waiting().await;
            observer.inner.placing.store(false, Ordering::SeqCst);
        });
    }

    async fn place_waiting(&self) {
        loop {
            let batch: Vec<Task> = {
                let mut waiting = lock(&self.inner.waiting);
                let room = self.room_today();
                let take = waiting.len().min(BATCH).min(room);
                waiting.drain(..take).collect()
            };
            if batch.is_empty() {
                return;
            }
            self.place(&batch).await;
        }
    }

    /// How many more tasks a model may place today.
    fn room_today(&self) -> usize {
        let day = now() / 86_400;
        let mut today = lock(&self.inner.today);
        if today.0 != day {
            *today = (day, 0);
        }
        MOST_A_DAY.saturating_sub(today.1)
    }

    /// Asks the model to place `tasks`, and keeps what it gave. A task it
    /// left out, or a call that failed, waits for the next catch-up.
    async fn place(&self, tasks: &[Task]) {
        let inner = &self.inner;
        let categories = &inner.settings.categories;
        let labels = inner.labels.load();
        let before: Vec<(Task, String)> = task::read(&inner.journal, now().saturating_sub(BEFORE))
            .into_iter()
            .rev()
            .filter_map(|task| {
                let name = category_of(&task, categories, &labels)?;
                Some((task, name))
            })
            .collect();
        let prompt = place::prompt(categories, &before, tasks);
        let answer = iq_agent::ask_once(
            &inner.provider,
            &inner.agents,
            &inner.settings.model,
            place::INSTRUCTIONS,
            prompt,
            iq_agent::Asked {
                agent: "observer",
                directory: &inner.directory,
                effort: Effort::Low,
                costs: &inner.costs,
            },
        )
        .await;
        let Ok(answer) = answer else {
            return;
        };
        let placed = place::read(
            &answer,
            tasks,
            categories,
            inner.settings.model.as_str(),
            now(),
        );
        lock(&inner.today).1 += tasks.len();
        let _ = inner.labels.add(&placed);
    }

    /// Waits until no task waits nor is being placed: for tests and for
    /// iq's end.
    pub async fn settled(&self) {
        loop {
            let idle = !self.inner.placing.load(Ordering::SeqCst)
                && lock(&self.inner.waiting).len() < self.inner.settings.every.max(1);
            if idle {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
