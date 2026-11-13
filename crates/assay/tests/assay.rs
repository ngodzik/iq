//! The assay against scripted models: candidates in folders of their own,
//! their checks run for real in the sandbox, and a grader that grades them
//! blind, the reference among them. No network, no provider.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iq_agent::Event;
use iq_assay::{
    Allowed, Assay, AssayError, Ending, Kind, Offer, Phase, Planned, Progress, Roster, Runner,
    Settings, State, Store,
};
use iq_core::{
    ChatModel, ChatRequest, ChatResponse, CoreError, ModelId, TokenCount, ToolCall, Usage, Usd,
};
use iq_gate::Helper;

/// The grader every test but one grades with.
const GRADER: &str = "lab/sage";

/// Answers each model from its own script, by the model asked, after a
/// while when it is given one, as a provider takes.
#[derive(Clone, Default)]
struct Scripted {
    scripts: Arc<Mutex<HashMap<String, VecDeque<ChatResponse>>>>,
    takes: Duration,
}

impl Scripted {
    fn new(scripts: Vec<(&str, Vec<ChatResponse>)>) -> Self {
        Self {
            scripts: Arc::new(Mutex::new(
                scripts
                    .into_iter()
                    .map(|(model, answers)| (model.to_owned(), answers.into()))
                    .collect(),
            )),
            takes: Duration::ZERO,
        }
    }

    /// The same, each answer taking `takes`.
    fn taking(self, takes: Duration) -> Self {
        Self { takes, ..self }
    }
}

impl ChatModel for Scripted {
    type Error = CoreError;

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, CoreError> {
        tokio::time::sleep(self.takes).await;
        let next = self
            .scripts
            .lock()
            .unwrap()
            .get_mut(request.model.as_str())
            .and_then(VecDeque::pop_front);
        Ok(next.unwrap_or_else(|| say("nothing more")))
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
        reasoning_signature: None,
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

/// Writes `mul` in calc.py, as a candidate that does the task.
fn writes_mul() -> ChatResponse {
    calls(
        "write_file",
        serde_json::json!({"path": "calc.py", "content": MUL}),
    )
}

/// calc.py with `mul`.
const MUL: &str = "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n";

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
    assay: Assay<Scripted>,
    costs: Arc<Mutex<Vec<Event>>>,
    watched: Arc<Mutex<Vec<Progress>>>,
}

fn settings(allowed: &[&str], grader: Option<&str>) -> Settings {
    Settings {
        grader: grader.map(|g| ModelId::new(g).unwrap()),
        allowed: Allowed::from_words(&allowed.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>()),
        reference: Some(ModelId::new("lab/quill").unwrap()),
        parallel: 2,
        ..Settings::default()
    }
}

/// The assay of a store of its own, its settings `settings`, the provider
/// listing the fixtures' models.
fn fixture(model: Scripted, settings: Settings) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("assay")).unwrap();
    let (assay, costs, watched) = assay_of(&store, model, settings);
    Fixture {
        _dir: dir,
        store,
        assay,
        costs,
        watched,
    }
}

type Assayed = (
    Assay<Scripted>,
    Arc<Mutex<Vec<Event>>>,
    Arc<Mutex<Vec<Progress>>>,
);

fn assay_of(store: &Store, model: Scripted, settings: Settings) -> Assayed {
    let costs = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&costs);
    let watched = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&watched);
    let runner = Runner {
        provider: model,
        provider_name: Some("Scripted".into()),
        helper: helper(),
        audit: Some(store.root().join("audit.log")),
        rules: None,
        effort: None,
    };
    let assay = Assay::new(
        store.clone(),
        runner,
        settings,
        Arc::new(move |event| kept.lock().unwrap().push(event)),
        Arc::new(move |progress| seen.lock().unwrap().push(progress)),
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
            offer("lab/quill"),
            offer("cloud/coder-5"),
            offer("lab/reed"),
            offer(GRADER),
            offer("lab/sage-2"),
        ],
        missing: None,
    });
    (assay, costs, watched)
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

/// Grades for every letter a batch of `scores` may take, from A.
fn grades(scores: &[u8]) -> ChatResponse {
    let grades: Vec<String> = scores
        .iter()
        .enumerate()
        .map(|(at, score)| {
            format!(
                r#"{{"label": "{}", "score": {score}, "notes": "graded {score}"}}"#,
                char::from(b'A' + at as u8)
            )
        })
        .collect();
    say(&format!(
        "Here are my grades.\n{{\"grades\": [{}]}}",
        grades.join(", ")
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_asked_with_its_cost_then_checked_graded_and_said() {
    let model = Scripted::new(vec![
        ("lab/quill", vec![writes_mul(), say("Added mul.")]),
        ("cloud/coder-5", vec![say("Done, I think.")]),
        (GRADER, vec![grades(&[8, 2])]),
    ]);
    let f = fixture(model, settings(&["models"], Some(GRADER)));
    exercise(&f.store);
    f.store.freeze("add-mul", 1).unwrap();
    assert_eq!(f.store.exercise("add-mul").unwrap().state, State::Frozen(1));
    let plan = f
        .assay
        .plan_run("add-mul", &["lab/quill", "cloud/coder-5"], 1)
        .unwrap();
    // What the person is asked, with what it should cost.
    assert_eq!(plan.plan.action, "run the exercise add-mul on 2 candidates");
    assert!(
        plan.plan.reason.starts_with("it spends about")
            && plan.plan.reason.contains("billed")
            && plan.plan.reason.contains("the grader (lab/sage)"),
        "{}",
        plan.plan.reason
    );
    assert_eq!(plan.runs(), 2);
    let said = f.assay.run_now(plan).await;
    assert!(said.contains("The runs of add-mul"), "{said}");
    assert!(said.contains("Models:"), "{said}");

    let records = f.store.records("add-mul");
    assert_eq!(records.len(), 2);
    let good = records
        .iter()
        .find(|r| r.model == "quill" && r.candidate.to_string() == "lab/quill")
        .unwrap();
    let bad = records
        .iter()
        .find(|r| r.candidate.to_string() == "cloud/coder-5")
        .unwrap();
    assert_eq!(good.ending, Ending::Answered);
    assert!(good.passed(), "{:?}", good.checked);
    assert_eq!(good.changed, (1, 4, 0));
    assert!(!bad.passed());
    assert_eq!(bad.changed.0, 0);
    // Graded blind, by letter.
    for record in &records {
        assert_eq!(record.verdicts.len(), 1, "{record:?}");
        assert_eq!(record.verdicts[0].key(), GRADER);
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
    let transcript = std::fs::read_to_string(folder.join("transcript.md")).unwrap();
    assert!(transcript.contains("`write_file`"), "{transcript}");
    assert!(
        transcript.ends_with("### The run ends: answered\n"),
        "{transcript}"
    );
    // What the agents kept on the side went with the run.
    assert!(!folder.join("scratch").exists());
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
    // The gates of the runs wrote to the assay's own audit log.
    assert!(
        std::fs::read_to_string(f.store.root().join("audit.log"))
            .unwrap()
            .contains("lab/quill")
    );
    // Each run was followed through its phases.
    let watched = f.watched.lock().unwrap();
    for phase in [
        Phase::Waiting,
        Phase::Working,
        Phase::Checking,
        Phase::Grading,
        Phase::Done(true),
    ] {
        assert!(watched.iter().any(|p| p.phase == phase), "{phase:?}");
    }
    let overview = f.assay.overview();
    assert_eq!(overview.exercises.len(), 1);
    assert_eq!(overview.exercises[0].waiting, 0);
    assert_eq!(overview.exercises[0].table.as_ref().unwrap().rows.len(), 2);
    assert_eq!(overview.categories[0].0, "python");
    let shown = overview.to_string();
    assert!(
        shown.contains("add-mul (Add mul): frozen, version 1. Kinds of task: python."),
        "{shown}"
    );
    // The letters are shuffled by batch: who got which grade is not known
    // here, only that each was graded.
    assert!(
        shown.contains("- quill [Scripted]: 1/1 passed, score ")
            && shown.contains("- coder5 [Scripted]: 0/1 passed, score "),
        "{shown}"
    );
    assert!(
        shown.contains("The reference (quill) is the cheapest that does the work"),
        "{shown}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_runs_as_its_pattern_says_and_a_new_grader_grades_it_again() {
    let model = Scripted::new(vec![
        ("lab/quill", vec![writes_mul(), say("Added mul.")]),
        ("lab/reed", vec![say("APPROVE: mul is right.")]),
        (GRADER, vec![grades(&[7, 7])]),
        ("lab/sage-2", vec![grades(&[9, 9])]),
    ]);
    let f = fixture(model.clone(), settings(&["teams"], Some(GRADER)));
    let folder = exercise(&f.store);
    write(&folder, "reference/calc.py", MUL);
    f.store.freeze("add-mul", 1).unwrap();
    let plan = f
        .assay
        .plan_run("add-mul", &["pair: lab/quill + lab/reed"], 1)
        .unwrap();
    f.assay.run_now(plan).await;
    let record = &f.store.records("add-mul")[0];
    assert_eq!(record.kind, Kind::Team);
    assert!(record.passed());
    // What the team told the person is its answer: the review.
    assert!(record.answer.starts_with("APPROVE"), "{}", record.answer);
    assert_eq!(record.score(), Some(7.0));
    // The reference was graded beside it, blind.
    let reference = f
        .store
        .root()
        .join("runs/add-mul")
        .join(&record.batch)
        .join("reference.json");
    assert!(reference.exists());
    assert_eq!(f.assay.overview().exercises[0].reference_grade, Some(7.0));
    // Graded by its grader, there is nothing left to grade.
    assert!(f.assay.plan_grade("add-mul").unwrap().is_none());

    // Another grader set, the run waits for its grade, and the first
    // grader's is left out of what is summed.
    let (again, _, _) = assay_of(&f.store, model, settings(&["teams"], Some("lab/sage-2")));
    let overview = again.overview();
    assert_eq!(overview.exercises[0].waiting, 1);
    assert!(
        overview.to_string().contains(
            "1 run of its latest version waits for lab/sage-2's grade: iq assay grade add-mul."
        ),
        "{overview}"
    );
    let plan = again.plan_grade("add-mul").unwrap().unwrap();
    assert_eq!(plan.runs(), 1);
    assert_eq!(plan.plan.action, "have lab/sage-2 grade 1 run of add-mul");
    assert!(
        plan.plan.reason.starts_with("it spends about"),
        "{}",
        plan.plan.reason
    );
    let said = again.grade_now(plan).await;
    assert!(
        said.starts_with("The grading of add-mul is done."),
        "{said}"
    );
    assert!(
        said.contains("Teams:\n- pair: lab/quill + lab/reed [Scripted]: 1/1 passed"),
        "{said}"
    );
    // Each grader's grade kept apart, the new one's summed.
    let record = &f.store.records("add-mul")[0];
    assert_eq!(record.verdicts.len(), 2);
    let overview = again.overview();
    assert_eq!(overview.exercises[0].waiting, 0);
    assert_eq!(overview.exercises[0].reference_grade, Some(9.0));
    let rows = &overview.exercises[0].table.as_ref().unwrap().rows;
    assert_eq!(rows[0].score, Some(9.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_freeze_runs_the_check_on_the_start_and_on_the_reference_first() {
    let f = fixture(Scripted::default(), settings(&["models"], Some(GRADER)));
    let folder = exercise(&f.store);
    // Without a reference, it fails on the start, and says nothing shows it
    // can pass.
    let plan = f.assay.plan_freeze("add-mul").await.unwrap();
    assert!(
        matches!(&plan, Planned::Ready(p) if p.reason.contains("with no reference/")),
        "{plan:?}"
    );
    // A reference that does not pass it is refused.
    write(&folder, "reference/notes.txt", "not the solution\n");
    let wrong = f.assay.plan_freeze("add-mul").await.unwrap_err();
    assert!(
        wrong.to_string().contains("fails on the reference"),
        "{wrong}"
    );
    // One that does, and the exercise is ready, its checks' result kept.
    write(&folder, "reference/calc.py", MUL);
    let plan = f.assay.plan_freeze("add-mul").await.unwrap();
    let Planned::Ready(ready) = &plan else {
        panic!("{plan:?}");
    };
    assert!(
        ready
            .reason
            .contains("fails on start/ and passes on the reference"),
        "{plan:?}"
    );
    assert_eq!(
        ready.action,
        "freeze the exercise add-mul (Add mul) as version 1"
    );
    assert!(folder.join("reference.json").exists());
    // Planning froze nothing: the person agrees first.
    assert_eq!(f.store.exercise("add-mul").unwrap().state, State::Draft);
    assert!(
        f.assay
            .freeze_now("add-mul")
            .unwrap()
            .starts_with("add-mul is frozen as version 1")
    );
    assert!(matches!(
        f.assay.plan_freeze("add-mul").await.unwrap(),
        Planned::Done(_)
    ));
    // Checks that pass on the start test nothing.
    write(&folder, "start/calc.py", MUL);
    let empty = f.assay.plan_freeze("add-mul").await.unwrap_err();
    assert!(
        empty.to_string().contains("passes on start/ already"),
        "{empty}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_not_ready_or_allowed_is_refused_before_anything_is_spent() {
    let f = fixture(Scripted::default(), settings(&["models"], Some(GRADER)));
    exercise(&f.store);
    let refused = |result: Result<iq_assay::RunPlan, AssayError>| match result {
        Ok(plan) => panic!("{:?}", plan.plan),
        Err(error) => error.to_string(),
    };
    let draft = refused(f.assay.plan_run("add-mul", &["lab/quill"], 1));
    assert!(draft.contains("draft"), "{draft}");
    f.store.freeze("add-mul", 1).unwrap();
    let team = refused(
        f.assay
            .plan_run("add-mul", &["pair: lab/quill + lab/reed"], 1),
    );
    assert!(team.contains("do not allow"), "{team}");
    let unknown = refused(f.assay.plan_run("add-mul", &["made/up"], 1));
    assert!(unknown.contains("iq models"), "{unknown}");
    // Without a grader, nothing runs, nor is graded.
    let (ungraded, _, _) = assay_of(&f.store, Scripted::default(), settings(&["models"], None));
    let none = refused(ungraded.plan_run("add-mul", &["lab/quill"], 1));
    assert!(none.contains("no grader is set"), "{none}");
    assert!(ungraded.plan_grade("add-mul").is_err());
    assert!(
        ungraded
            .overview()
            .to_string()
            .contains("No grader is set: `grader` in"),
    );
    // Exercises are chosen by name, by kind of task, or all.
    assert_eq!(f.assay.select("add-mul", true).unwrap(), ["add-mul"]);
    assert_eq!(f.assay.select("Python", true).unwrap(), ["add-mul"]);
    assert_eq!(f.assay.select("all", true).unwrap(), ["add-mul"]);
    let nothing = f.assay.select("rust", true).unwrap_err().to_string();
    assert!(nothing.contains("the exercises are add-mul"), "{nothing}");
    // A draft runs nowhere.
    write(
        &f.store.root().join("exercises/draft"),
        "exercise.toml",
        "title = \"Later\"\n",
    );
    assert_eq!(f.assay.select("all", true).unwrap(), ["add-mul"]);
    assert_eq!(f.assay.select("all", false).unwrap(), ["add-mul", "draft"]);
    let shown = f.assay.overview().to_string();
    assert!(
        shown.contains("draft (Later): draft. Kinds of task: none.")
            && shown
                .contains("  To fix: task.md, what every candidate is told, is missing or empty."),
        "{shown}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_candidate_that_spends_what_it_is_allowed_is_stopped() {
    let model = Scripted::new(vec![
        (
            "lab/quill",
            vec![
                calls("list_agents", serde_json::json!({})),
                calls("list_agents", serde_json::json!({})),
                calls("list_agents", serde_json::json!({})),
                writes_mul(),
                say("Added mul."),
            ],
        ),
        (GRADER, vec![grades(&[3])]),
    ])
    .taking(Duration::from_millis(200));
    let f = fixture(model, settings(&["models"], Some(GRADER)));
    let folder = exercise(&f.store);
    // Two calls at $0.002 each spend it.
    write(
        &folder,
        "exercise.toml",
        "title = \"Add mul\"\ncategories = [\"python\"]\ncheck = \"grep -q 'def mul' calc.py\"\nminutes = 2\ndollars = 0.003\n",
    );
    f.store.freeze("add-mul", 1).unwrap();
    let plan = f.assay.plan_run("add-mul", &["lab/quill"], 1).unwrap();
    tokio::time::timeout(Duration::from_secs(60), f.assay.run_now(plan))
        .await
        .unwrap();
    let record = &f.store.records("add-mul")[0];
    assert!(
        matches!(&record.ending, Ending::Failed(why) if why.contains("it was allowed")),
        "{:?}",
        record.ending
    );
    // Stopped as its third call was under way, it never wrote mul.
    assert!(!record.passed(), "{:?}", record.checked);
    assert_eq!(record.spent.calls, 2, "{:?}", record.spent);
}
