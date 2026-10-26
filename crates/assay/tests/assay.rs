//! The assay against scripted models: a chat that asks for a run, two
//! candidates in folders of their own, their checks run for real in the
//! sandbox, and a grader that grades them blind, the reference among them.
//! No network, no provider.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iq_agent::{Crew, Event, Extension, Settings as CrewSettings};
use iq_assay::{Allowed, Assay, Ending, Offer, Roster, Runner, Settings, State, Store, Told};
use iq_core::{
    ChatModel, ChatRequest, ChatResponse, CoreError, Delegate, DelegateEvent, DelegateReply,
    DelegateRequest, ModelId, TokenCount, ToolCall, Usage, Usd,
};
use iq_gate::{Gate, Helper};

/// Answers each model from its own script, by the model asked.
#[derive(Clone, Default)]
struct Scripted(Arc<Mutex<HashMap<String, VecDeque<ChatResponse>>>>);

impl Scripted {
    fn new(scripts: Vec<(&str, Vec<ChatResponse>)>) -> Self {
        Self(Arc::new(Mutex::new(
            scripts
                .into_iter()
                .map(|(model, answers)| (model.to_owned(), answers.into()))
                .collect(),
        )))
    }
}

impl ChatModel for Scripted {
    type Error = CoreError;

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, CoreError> {
        let next = self
            .0
            .lock()
            .unwrap()
            .get_mut(request.model.as_str())
            .and_then(VecDeque::pop_front);
        Ok(next.unwrap_or_else(|| say("nothing more")))
    }
}

/// No agent with tools of its own is used here.
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

fn say(text: &str) -> ChatResponse {
    ChatResponse {
        content: Some(text.to_owned()),
        tool_calls: Vec::new(),
        usage: Usage {
            input: TokenCount(1_000),
            output: TokenCount(100),
        },
        cost: Some(Usd(0.002)),
        cache: None,
    }
}

fn calls(tool: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call-{tool}"),
            name: tool.to_owned(),
            arguments: arguments.to_string(),
        }],
        ..say("")
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let at = root.join(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
}

fn helper() -> Helper {
    Helper::new(env!("CARGO_BIN_EXE_iq-assay-sandbox"), Vec::<String>::new())
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    assay: Assay<Scripted, NoAgents>,
    crew: Crew<Scripted, NoAgents>,
    events: Arc<Mutex<Vec<Event>>>,
    costs: Arc<Mutex<Vec<Event>>>,
    told: Arc<Mutex<Vec<Told>>>,
}

/// The chat's crew in the store, the assay serving the chat, every
/// question the person is asked answered yes.
fn fixture(model: Scripted) -> Fixture {
    fixture_allowing(model, &["models"])
}

fn fixture_allowing(model: Scripted, allowed: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("assay")).unwrap();
    let costs = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&costs);
    let told = Arc::new(Mutex::new(Vec::new()));
    let heard = Arc::clone(&told);
    let runner = Runner {
        provider: model.clone(),
        provider_name: Some("Scripted".into()),
        agents: NoAgents,
        helper: helper(),
        gate: Arc::new(|folder: &Path| Gate::builder(folder).build().map_err(|e| e.to_string())),
        reach: None,
        effort: None,
    };
    let assay = Assay::new(
        store.clone(),
        runner,
        Settings {
            grader: ModelId::new("grader/model").unwrap(),
            allowed: Allowed::from_words(
                &allowed.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>(),
            ),
            reference: ModelId::new("acme/good-coder").unwrap(),
            parallel: 2,
            mirrors: Vec::new(),
            servers: Vec::new(),
            journal: None,
            labels: None,
            categories: iq_observe::Categories::default(),
            notices: Vec::new(),
        },
        Arc::new(move |event| kept.lock().unwrap().push(event)),
        Arc::new(move |t| heard.lock().unwrap().push(t)),
    );
    let offer = |id: &str| Offer {
        id: id.into(),
        canonical: None,
        input: Some(1e-6),
        output: Some(2e-6),
        context: None,
        tools: Some(true),
        description: None,
    };
    assay.set_roster(Roster {
        provider: Some("Scripted".into()),
        models: vec![
            offer("acme/good-coder"),
            offer("acme/bad-coder"),
            offer("acme/reviewer"),
            offer("grader/model"),
        ],
        harnesses: Vec::new(),
        missing: None,
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    let (questions, mut asked) = tokio::sync::mpsc::unbounded_channel();
    let crew = Crew::new(
        model,
        NoAgents,
        Gate::builder(store.root()).build().unwrap(),
        helper(),
        CrewSettings {
            patience: Duration::from_secs(20),
            ..CrewSettings::default()
        },
        Arc::new(move |event: Event| {
            if let Event::Asked { id, .. } = &event {
                let _ = questions.send(*id);
            }
            seen.lock().unwrap().push(event);
        }),
    );
    {
        let crew = crew.clone();
        tokio::spawn(async move {
            while let Some(id) = asked.recv().await {
                crew.answer(id, true);
            }
        });
    }
    assay.serve(iq_assay::CHAT);
    crew.extend(Arc::new(assay.clone()));
    Fixture {
        _dir: dir,
        store,
        assay,
        crew,
        events,
        costs,
        told,
    }
}

/// The exercise: add `mul` to calc.py, checked by grep.
fn exercise(store: &Store) -> PathBuf {
    let folder = store.root().join("exercises/add-mul");
    write(
        &folder,
        "exercise.toml",
        "title = \"Add mul\"\ncategories = [\"python\"]\ncheck = \"grep -q 'def mul' calc.py && test -f hidden_check.txt\"\nminutes = 2\ndollars = 0.5\n",
    );
    write(&folder, "task.md", "Add a mul function to calc.py.\n");
    write(&folder, "rubric.md", "Short and correct.\n");
    write(
        &folder,
        "start/calc.py",
        "def add(a, b):\n    return a + b\n",
    );
    write(
        &folder,
        "checks/hidden_check.txt",
        "put in after the candidate\n",
    );
    folder
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..600 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what} never happened");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_asked_in_the_chat_is_checked_graded_and_told_back() {
    let grades = r#"{"grades": [{"label": "A", "score": 8, "notes": "fine"}, {"label": "B", "score": 2, "notes": "nothing done"}]}"#;
    let model = Scripted::new(vec![
        (
            "talk/model",
            vec![
                calls(
                    "assay_run",
                    serde_json::json!({"exercise": "add-mul", "candidates": ["acme/good-coder", "acme/bad-coder"]}),
                ),
                say("Started; I will tell you."),
                say("acme/good-coder passed, acme/bad-coder did nothing."),
            ],
        ),
        (
            "acme/good-coder",
            vec![
                calls(
                    "write_file",
                    serde_json::json!({"path": "calc.py", "content": "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n"}),
                ),
                say("Added mul."),
            ],
        ),
        ("acme/bad-coder", vec![say("Done, I think.")]),
        (
            "grader/model",
            vec![say(&format!("Here are my grades.\n{grades}"))],
        ),
    ]);
    let f = fixture(model);
    exercise(&f.store);
    f.store.freeze("add-mul", 1).unwrap();
    assert_eq!(f.store.exercise("add-mul").unwrap().state, State::Frozen(1));
    f.crew.start(
        iq_assay::CHAT,
        &ModelId::new("talk/model").unwrap(),
        "Run add-mul on both.".into(),
        iq_agent::PERSON,
    );
    let events = Arc::clone(&f.events);
    until("the results told to the chat", || {
        events.lock().unwrap().iter().any(|e| {
            matches!(e, Event::Posted { from, to, .. } if from == "results" && to == iq_assay::CHAT)
        })
    })
    .await;
    f.crew.settled().await;

    let events = f.events.lock().unwrap().clone();
    // The person was asked, with what it should cost.
    let asked = events
        .iter()
        .find_map(|e| match e {
            Event::Asked { action, reason, .. } => Some(format!("{action}: {reason}")),
            _ => None,
        })
        .unwrap();
    assert!(
        asked.contains("run the exercise add-mul on 2 candidates"),
        "{asked}"
    );
    assert!(
        asked.contains("billed") && asked.contains("grader/model"),
        "{asked}"
    );
    let told = events
        .iter()
        .find_map(|e| match e {
            Event::Posted { from, text, .. } if from == "results" => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    assert!(told.contains("The runs of add-mul"), "{told}");
    assert!(told.contains("Models:"), "{told}");

    let records = f.store.records("add-mul");
    assert_eq!(records.len(), 2);
    let good = records
        .iter()
        .find(|r| r.model == "goodcoder" && r.candidate.to_string() == "acme/good-coder")
        .unwrap();
    let bad = records
        .iter()
        .find(|r| r.candidate.to_string() == "acme/bad-coder")
        .unwrap();
    assert_eq!(good.ending, Ending::Answered);
    assert!(good.passed(), "{:?}", good.checked);
    assert_eq!(good.changed, (1, 4, 0));
    assert!(!bad.passed());
    assert_eq!(bad.changed.0, 0);
    // Graded blind, by letter.
    for record in &records {
        assert_eq!(record.verdicts.len(), 1, "{record:?}");
        assert_eq!(record.verdicts[0].judge, "grader/model");
        assert_eq!(record.verdicts[0].key(), "grader/model");
        assert_eq!(record.provider.as_deref(), Some("Scripted"));
    }
    let folder = f
        .store
        .root()
        .join("runs/add-mul")
        .join(&good.batch)
        .join(&good.label);
    assert!(
        std::fs::read_to_string(folder.join("changes.diff"))
            .unwrap()
            .contains("+def mul(a, b):")
    );
    assert!(
        std::fs::read_to_string(folder.join("transcript.md"))
            .unwrap()
            .contains("write_file")
    );
    // The work the grader reads names nobody.
    let blind = f
        .store
        .root()
        .join("judging")
        .join(format!("add-mul-{}", good.batch));
    assert!(blind.join(format!("{}.diff", good.label)).exists());
    assert!(!blind.join("run.json").exists());
    // Every cost was told, the runs' and the grader's.
    let costs = f.costs.lock().unwrap();
    assert!(
        costs
            .iter()
            .any(|e| matches!(e, Event::Spent { agent, .. } if agent.starts_with("assay add-mul")))
    );
    assert!(
        costs
            .iter()
            .any(|e| matches!(e, Event::Spent { agent, .. } if agent == "assay grader"))
    );
    assert!(
        f.told
            .lock()
            .unwrap()
            .iter()
            .any(|t| matches!(t, Told::Running(r) if !r.is_empty()))
    );
    let overview = f.assay.overview();
    assert_eq!(overview.tables.len(), 1);
    assert_eq!(overview.tables[0].rows.len(), 2);
    assert_eq!(overview.categories[0].0, "python");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_runs_as_its_pattern_says_and_is_graded_again_with_its_reference() {
    // Grades for every letter a batch of one and its reference may take.
    let grades = |a: u8, b: u8| {
        say(&format!(
            r#"{{"grades": [{{"label": "A", "score": {a}, "notes": "one"}}, {{"label": "B", "score": {b}, "notes": "two"}}]}}"#
        ))
    };
    let model = Scripted::new(vec![
        (
            "acme/good-coder",
            vec![
                calls(
                    "write_file",
                    serde_json::json!({"path": "calc.py", "content": "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n"}),
                ),
                say("Added mul."),
            ],
        ),
        ("acme/reviewer", vec![say("APPROVE: mul is right.")]),
        ("grader/model", vec![grades(7, 7), grades(9, 9)]),
    ]);
    let f = fixture_allowing(model, &["teams"]);
    let folder = exercise(&f.store);
    write(
        &folder,
        "reference/calc.py",
        "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n",
    );
    f.store.freeze("add-mul", 1).unwrap();
    let call = |name: &str, args: serde_json::Value| {
        f.assay
            .call(f.crew.clone(), iq_assay::CHAT.into(), name.into(), args)
    };
    let started = call(
        "assay_run",
        serde_json::json!({"exercise": "add-mul", "candidates": ["pair: acme/good-coder + acme/reviewer"]}),
    )
    .await
    .unwrap();
    assert!(started.starts_with("Started: 1 run"), "{started}");
    let store = f.store.clone();
    until("the team's run graded", || {
        store
            .records("add-mul")
            .first()
            .is_some_and(|r| !r.verdicts.is_empty())
    })
    .await;
    let record = &f.store.records("add-mul")[0];
    assert_eq!(record.kind, iq_assay::Kind::Team);
    assert!(record.passed());
    // What the team told the person is its answer: the review.
    assert!(record.answer.starts_with("APPROVE"), "{}", record.answer);
    // The reference was graded beside it, blind.
    let reference = f
        .store
        .root()
        .join("runs/add-mul")
        .join(&record.batch)
        .join("reference.json");
    assert!(reference.exists());
    let overview = f.assay.overview();
    assert_eq!(overview.exercises[0].reference_grade, Some(7.0));
    // Graded by its grader, there is nothing left to grade, unless asked.
    let again = call("assay_grade", serde_json::json!({"exercise": "add-mul"}))
        .await
        .unwrap();
    assert!(again.contains("is graded by grader/model"), "{again}");
    let regraded = call(
        "assay_grade",
        serde_json::json!({"exercise": "add-mul", "all": true}),
    )
    .await
    .unwrap();
    assert!(regraded.contains("Grading add-mul"), "{regraded}");
    until("the grade given again", || {
        store.records("add-mul")[0].score() == Some(9.0)
    })
    .await;
    // The same grader's grade replaces its own.
    assert_eq!(f.store.records("add-mul")[0].verdicts.len(), 1);
    let results = call(
        "assay_results",
        serde_json::json!({"exercise": "add-mul", "providers": true}),
    )
    .await
    .unwrap();
    assert!(
        results.starts_with("Scores are grader/model's grades."),
        "{results}"
    );
    assert!(
        results.contains("Teams:\n- pair: acme/good-coder + acme/reviewer [Scripted]: 1/1 passed"),
        "{results}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_freeze_runs_the_check_on_the_start_and_on_the_reference_first() {
    let f = fixture(Scripted::default());
    let folder = exercise(&f.store);
    let planned =
        |assay: Assay<Scripted, NoAgents>| async move { assay.plan_freeze("add-mul").await };
    // Without a reference, it fails on the start, and says nothing shows it
    // can pass.
    let plan = planned(f.assay.clone()).await.unwrap();
    assert!(
        matches!(&plan, iq_assay::Planned::Ready(p) if p.reason.contains("with no reference/")),
        "{plan:?}"
    );
    // A reference that does not pass it is refused.
    write(&folder, "reference/notes.txt", "not the solution\n");
    let wrong = planned(f.assay.clone()).await.unwrap_err();
    assert!(wrong.contains("fails on the reference"), "{wrong}");
    // One that does, and the exercise is ready, its checks' result kept.
    write(
        &folder,
        "reference/calc.py",
        "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n",
    );
    let plan = planned(f.assay.clone()).await.unwrap();
    assert!(
        matches!(&plan, iq_assay::Planned::Ready(p) if p.reason.contains("fails on start/ and passes on the reference")),
        "{plan:?}"
    );
    assert!(folder.join("reference.json").exists());
    // Checks that pass on the start test nothing.
    write(
        &folder,
        "start/calc.py",
        "def mul(a, b):\n    return a * b\n",
    );
    let empty = planned(f.assay.clone()).await.unwrap_err();
    assert!(empty.contains("passes on start/ already"), "{empty}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_assays_tools_refuse_what_is_not_ready_or_allowed() {
    let f = fixture(Scripted::default());
    exercise(&f.store);
    let call = |name: &str, args: serde_json::Value| {
        f.assay
            .call(f.crew.clone(), iq_assay::CHAT.into(), name.into(), args)
    };
    let draft = call(
        "assay_run",
        serde_json::json!({"exercise": "add-mul", "candidates": ["acme/good-coder"]}),
    )
    .await
    .unwrap_err();
    assert!(draft.contains("draft"), "{draft}");
    f.store.freeze("add-mul", 1).unwrap();
    let harness = call(
        "assay_run",
        serde_json::json!({"exercise": "add-mul", "candidates": ["claude-code/haiku"]}),
    )
    .await
    .unwrap_err();
    assert!(harness.contains("do not allow"), "{harness}");
    let unknown = call(
        "assay_run",
        serde_json::json!({"exercise": "add-mul", "candidates": ["made/up"]}),
    )
    .await
    .unwrap_err();
    assert!(unknown.contains("assay_roster"), "{unknown}");
    let listed = call("assay_exercises", serde_json::json!({}))
        .await
        .unwrap();
    assert!(
        listed.contains("add-mul (Add mul): frozen, version 1"),
        "{listed}"
    );
    let roster = call("assay_roster", serde_json::json!({"search": "good"}))
        .await
        .unwrap();
    assert!(
        roster.contains("- acme/good-coder: 1.00 / 2.00"),
        "{roster}"
    );
    assert!(
        !roster.contains("acme/bad-coder") && !roster.contains("- grader/model"),
        "{roster}"
    );
    // Another agent has none of the assay's tools.
    assert!(f.assay.tools("lead").is_empty());
    assert_eq!(f.crew.tools_for(iq_assay::CHAT).len(), 4 + 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exercise_is_made_from_a_commit_once_the_person_allows_it() {
    let f = fixture(Scripted::default());
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
    };
    git(&["init", "-q"]);
    write(root, "calc.py", "def add(a, b):\n    return a + b\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "Start"]);
    write(
        root,
        "calc.py",
        "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n",
    );
    write(
        root,
        "tests/test_calc.py",
        "from calc import mul\n\n\ndef test_mul():\n    assert mul(2, 3) == 6\n",
    );
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "Add mul, with its test"]);
    let made = f
        .assay
        .call(
            f.crew.clone(),
            iq_assay::CHAT.into(),
            "assay_from_commit".into(),
            serde_json::json!({"repository": root.to_str().unwrap(), "commit": "HEAD", "exercise": "mul"}),
        )
        .await
        .unwrap();
    assert!(made.contains("tests/test_calc.py"), "{made}");
    // The gate asked the person to let the chat read the repository.
    assert!(f.events.lock().unwrap().iter().any(
        |e| matches!(e, Event::Asked { reason, .. } if reason.contains("outside the workspace"))
    ));
    let folder = f.store.root().join("exercises/mul");
    assert_eq!(
        std::fs::read_to_string(folder.join("start/calc.py")).unwrap(),
        "def add(a, b):\n    return a + b\n"
    );
    assert!(!folder.join("start/tests").exists());
    assert!(folder.join("after/tests/test_calc.py").exists());
    assert!(
        std::fs::read_to_string(folder.join("reference.diff"))
            .unwrap()
            .contains("+def mul")
    );
    let exercise = f.store.exercise("mul").unwrap();
    assert_eq!(exercise.title, "Add mul, with its test");
    assert_eq!(exercise.state, State::Draft);
    // A name that leaves the store, or an option for git, is refused.
    for (commit, id) in [("HEAD", "../x"), ("--output=/tmp/x", "y")] {
        let refused = f
            .assay
            .call(
                f.crew.clone(),
                iq_assay::CHAT.into(),
                "assay_from_commit".into(),
                serde_json::json!({"repository": root.to_str().unwrap(), "commit": commit, "exercise": id}),
            )
            .await;
        assert!(refused.is_err());
    }
}
