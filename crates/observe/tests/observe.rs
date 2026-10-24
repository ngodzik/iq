//! The observer against a scripted model: no network, no provider.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use iq_agent::Event;
use iq_core::{
    ChatModel, ChatRequest, ChatResponse, CoreError, Delegate, DelegateEvent, DelegateReply,
    DelegateRequest, ModelId, TokenCount, Usage, Usd,
};
use iq_observe::{Categories, Category, Labels, Observer, Settings, Task, category_of};

/// Answers from a script, and keeps what it was asked.
#[derive(Clone, Default)]
struct Scripted {
    answers: Arc<Mutex<VecDeque<String>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl ChatModel for Scripted {
    type Error = CoreError;

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, CoreError> {
        let asked = format!("{:?}", request.messages);
        self.asked.lock().unwrap().push(asked);
        let answer = self.answers.lock().unwrap().pop_front().unwrap_or_default();
        Ok(ChatResponse {
            content: Some(answer),
            tool_calls: Vec::new(),
            usage: Usage {
                input: TokenCount(500),
                output: TokenCount(20),
            },
            cost: Some(Usd(0.0001)),
            cache: None,
        })
    }
}

#[derive(Clone, Default)]
struct NoAgents;

impl Delegate for NoAgents {
    type Error = CoreError;

    async fn run(
        &self,
        _: &DelegateRequest,
        _: &mut (dyn FnMut(DelegateEvent) + Send),
    ) -> Result<DelegateReply, CoreError> {
        Err(CoreError::EmptyModelId)
    }
}

fn task(at: u64, prompt: &str, files: &[&str]) -> Task {
    Task {
        at,
        agent: "lead".into(),
        model: "m".into(),
        prompt: prompt.into(),
        files: files.iter().map(|f| (*f).to_owned()).collect(),
        ..Task::default()
    }
}

fn categories() -> Categories {
    Categories::new(vec![
        Category {
            name: "Python".into(),
            files: vec!["*.py".into()],
            ..Category::default()
        },
        Category {
            name: "Reviews".into(),
            description: "reading others' changes".into(),
            ..Category::default()
        },
    ])
}

#[tokio::test]
async fn the_rules_place_first_and_a_cheap_model_the_rest_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("tasks.jsonl");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let tasks = [
        task(now - 30, "fix the parser", &["parse.py"]),
        task(now - 20, "review the auth PR", &[]),
        task(now - 10, "what does this do?", &[]),
    ];
    let lines: String = tasks
        .iter()
        .map(|t| serde_json::to_string(t).unwrap() + "\n")
        .collect();
    std::fs::write(&journal, lines).unwrap();
    let model = Scripted::default();
    model
        .answers
        .lock()
        .unwrap()
        .push_back(r#"{"categories": {"T1": "Reviews", "T2": "Cooking"}}"#.into());
    let spent = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&spent);
    let labels = Labels::at(dir.path().join("labels.jsonl"));
    let observer = Observer::new(
        model.clone(),
        NoAgents,
        Settings {
            model: ModelId::new("cheap/model").unwrap(),
            every: 2,
            categories: categories(),
        },
        journal.clone(),
        labels.clone(),
        Arc::new(move |event: Event| kept.lock().unwrap().push(event)),
        dir.path().to_path_buf(),
    );
    // The rules place the first: nothing waits.
    observer.done(tasks[0].clone());
    observer.done(tasks[1].clone());
    observer.settled().await;
    assert!(
        model.asked.lock().unwrap().is_empty(),
        "one waits, fewer than the batch"
    );
    observer.done(tasks[2].clone());
    observer.settled().await;
    let asked = model.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "two placed in one call");
    assert!(
        asked[0].contains("review the auth PR")
            && asked[0].contains("Reviews: reading others' changes")
    );
    let kept = labels.load();
    let category = |t: &Task| category_of(t, &categories(), &kept);
    assert_eq!(category(&tasks[0]).as_deref(), Some("Python"));
    assert_eq!(category(&tasks[1]).as_deref(), Some("Reviews"));
    // A category it made up is taken as other.
    assert_eq!(category(&tasks[2]).as_deref(), Some("other"));
    // What it cost is counted with the rest.
    assert!(
        spent
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Spent { agent, .. } if agent == "observer"))
    );

    // Once the categories change, what no longer fits is placed again.
    let renamed = Categories::new(vec![Category {
        name: "Code reviews".into(),
        ..Category::default()
    }]);
    model.answers.lock().unwrap().push_back(
        r#"{"categories": {"T1": "other", "T2": "Code reviews", "T3": "other"}}"#.into(),
    );
    let again = Observer::new(
        model.clone(),
        NoAgents,
        Settings {
            model: ModelId::new("cheap/model").unwrap(),
            every: 1,
            categories: renamed.clone(),
        },
        journal,
        labels.clone(),
        Arc::new(|_| {}),
        dir.path().to_path_buf(),
    );
    again.catch_up(0);
    again.settled().await;
    assert_eq!(model.asked.lock().unwrap().len(), 2);
    assert_eq!(
        category_of(&tasks[1], &renamed, &labels.load()).as_deref(),
        Some("Code reviews")
    );
}

#[tokio::test]
async fn without_categories_nothing_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let model = Scripted::default();
    let observer = Observer::new(
        model.clone(),
        NoAgents,
        Settings {
            model: ModelId::new("cheap/model").unwrap(),
            every: 1,
            categories: Categories::default(),
        },
        dir.path().join("tasks.jsonl"),
        Labels::at(dir.path().join("labels.jsonl")),
        Arc::new(|_| {}),
        dir.path().to_path_buf(),
    );
    observer.done(task(1, "anything", &[]));
    observer.catch_up(0);
    observer.settled().await;
    assert!(model.asked.lock().unwrap().is_empty());
}
