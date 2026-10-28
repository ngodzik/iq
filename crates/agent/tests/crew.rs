//! The crew against scripted models: no network, no provider.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iq_agent::{Called, Crew, Event, Extension, PERSON, Pattern, RulesFile, Settings};
use iq_core::{
    ChatModel, ChatRequest, ChatResponse, CoreError, Message, ModelId, TokenCount, ToolCall, Usage,
    Usd,
};
use iq_gate::{Gate, Helper};

/// Answers each agent from its own script, the agent known by the name its
/// instructions give it, and keeps every request.
#[derive(Default)]
struct Scripted {
    scripts: Mutex<HashMap<String, VecDeque<ChatResponse>>>,
    requests: Mutex<Vec<(String, ChatRequest)>>,
}

impl Scripted {
    fn new(scripts: &[(&str, Vec<ChatResponse>)]) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(
                scripts
                    .iter()
                    .map(|(name, answers)| (name.to_string(), answers.clone().into()))
                    .collect(),
            ),
            requests: Mutex::default(),
        })
    }

    /// The models `agent`'s calls asked for.
    fn models_of(&self, agent: &str) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == agent)
            .map(|(_, request)| request.model.to_string())
            .collect()
    }

    /// The efforts `agent`'s calls asked for.
    fn efforts_of(&self, agent: &str) -> Vec<Option<iq_core::Effort>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == agent)
            .map(|(_, request)| request.effort)
            .collect()
    }

    /// Every request of `agent`'s, in order.
    fn requests_of(&self, agent: &str) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == agent)
            .map(|(_, request)| request.clone())
            .collect()
    }

    /// The standing instructions of `agent`'s first call.
    fn instructions_of(&self, agent: &str) -> String {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .find(|(name, _)| name == agent)
            .and_then(|(_, request)| match request.messages.first() {
                Some(Message::System(text)) => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Everything `agent` was sent, as one text.
    fn seen_by(&self, agent: &str) -> String {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == agent)
            .map(|(_, request)| format!("{:?}", request.messages))
            .collect()
    }
}

fn speaker(request: &ChatRequest) -> String {
    match request.messages.first() {
        Some(Message::System(text)) => text
            .strip_prefix("You are ")
            .and_then(|rest| rest.split(',').next())
            .unwrap_or("reviewer")
            .to_owned(),
        _ => "?".to_owned(),
    }
}

/// The script, as the crew holds it.
#[derive(Clone)]
struct Model(Arc<Scripted>);

impl std::ops::Deref for Model {
    type Target = Scripted;

    fn deref(&self) -> &Scripted {
        &self.0
    }
}

impl ChatModel for Model {
    type Error = CoreError;

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, CoreError> {
        let name = speaker(request);
        self.requests
            .lock()
            .unwrap()
            .push((name.clone(), request.clone()));
        let next = self
            .scripts
            .lock()
            .unwrap()
            .get_mut(&name)
            .and_then(VecDeque::pop_front);
        Ok(next.unwrap_or_else(|| say(&format!("{name} has nothing more to say"))))
    }
}

fn say(text: &str) -> ChatResponse {
    ChatResponse {
        content: Some(text.to_owned()),
        tool_calls: Vec::new(),
        usage: Usage {
            input: TokenCount(100),
            output: TokenCount(10),
        },
        cost: Some(Usd(0.001)),
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

/// `response`, as a call that read `tokens`.
fn reading(tokens: u64, mut response: ChatResponse) -> ChatResponse {
    response.usage.input = TokenCount(tokens);
    response
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    crew: Crew<Model>,
    model: Arc<Scripted>,
    events: Arc<Mutex<Vec<Event>>>,
}

fn fixture(scripts: &[(&str, Vec<ChatResponse>)], settings: Settings) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let gate = Gate::builder(&root).build().unwrap();
    let model = Scripted::new(scripts);
    let events = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&events);
    let crew = Crew::new(
        Model(Arc::clone(&model)),
        gate,
        // Nothing here runs a command: the helper is never started.
        Helper::new("/nonexistent", Vec::<String>::new()),
        settings,
        Arc::new(move |event| kept.lock().unwrap().push(event)),
    );
    Fixture {
        _dir: dir,
        root,
        crew,
        model,
        events,
    }
}

impl Fixture {
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    async fn settled(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.crew.settled())
            .await
            .expect("the crew never settled");
    }

    /// Waits for the person to be asked something, and gives its number.
    async fn question(&self) -> u64 {
        for _ in 0..200 {
            if let Some(id) = self.events().iter().find_map(|event| match event {
                Event::Asked { id, .. } => Some(*id),
                _ => None,
            }) {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("nothing was asked: {:#?}", self.events());
    }
}

fn model(id: &str) -> ModelId {
    ModelId::new(id).unwrap()
}

#[tokio::test]
async fn an_agent_writes_and_reads_through_the_gate_and_says_what_it_spent() {
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "write_file",
                    serde_json::json!({"path": "src/hello.txt", "content": "hi\n"}),
                ),
                calls("read_file", serde_json::json!({"path": "src/hello.txt"})),
                calls("read_file", serde_json::json!({"path": ".env"})),
                say("Wrote it."),
            ],
        )],
        Settings::default(),
    );
    std::fs::write(f.root.join(".env"), "KEY=secret-value-1234\n").unwrap();
    f.crew
        .start("lead", &model("cheap/model"), "write hello".into(), PERSON);
    f.settled().await;
    assert_eq!(
        std::fs::read_to_string(f.root.join("src/hello.txt")).unwrap(),
        "hi\n"
    );
    let seen = f.model.seen_by("lead");
    assert!(seen.contains("1\\thi"), "{seen}");
    assert!(!seen.contains("secret-value"), "{seen}");
    assert!(
        seen.contains("the gate refused it: it holds secrets"),
        "{seen}"
    );
    assert_refusal_guidance(&seen);
    assert!(seen.contains("nobody can approve it"), "{seen}");
    let events = f.events();
    assert!(events.contains(&Event::Done {
        agent: "lead".into(),
        outcome: Ok("Wrote it.".into())
    }));
    let spent: f64 = events
        .iter()
        .filter_map(|event| match event {
            Event::Spent {
                cost: Some(cost), ..
            } => Some(cost.0),
            _ => None,
        })
        .sum();
    assert!((spent - 0.004).abs() < 1e-9, "{spent}");
    assert!((f.crew.billed().0 - 0.004).abs() < 1e-9);
}

#[tokio::test]
async fn what_the_gate_asks_waits_for_the_person() {
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "write_file",
                    serde_json::json!({"path": ".git/hooks/pre-commit", "content": "evil"}),
                ),
                say("Could not."),
            ],
        )],
        Settings::default(),
    );
    // Even the autopilot leaves it to the person: it widens the sandbox.
    f.crew.set_autopilot(true);
    f.crew
        .start("lead", &model("cheap/model"), "add a hook".into(), PERSON);
    let id = f.question().await;
    f.crew.answer(id, false);
    f.settled().await;
    assert!(!f.root.join(".git/hooks/pre-commit").exists());
    assert!(f.model.seen_by("lead").contains("the person refused it"));
    assert_refusal_guidance(&f.model.seen_by("lead"));
    assert!(f.events().contains(&Event::Answered {
        id,
        approved: false
    }));
    assert!(f.model.seen_by("reviewer").is_empty(), "no reviewer");
}

struct ReviewPolicy(Result<Option<String>, String>);

impl Extension<Model> for ReviewPolicy {
    fn tools(&self, _agent: &str) -> Vec<iq_core::ToolSpec> {
        Vec::new()
    }

    fn review_instructions(&self) -> Result<Option<String>, String> {
        self.0.clone()
    }

    fn call(
        &self,
        _crew: Crew<Model>,
        _agent: String,
        _name: String,
        _args: serde_json::Value,
    ) -> Called {
        Box::pin(std::future::ready(Err("no tools".into())))
    }
}

#[tokio::test]
async fn the_reviewer_receives_local_policy_and_errors_go_to_the_person() {
    for policy in [
        Ok(Some("Preserve local database fixtures.".into())),
        Err("review skill unavailable".into()),
    ] {
        let failed = policy.is_err();
        let f = fixture(
            &[
                (
                    "lead",
                    vec![
                        calls("run", serde_json::json!({"command": "git reset --hard"})),
                        say("Blocked."),
                    ],
                ),
                ("reviewer", vec![say("REFUSE: preserve the fixtures.")]),
            ],
            Settings {
                reviewer: Some(model("cheap/reviewer")),
                ..Settings::default()
            },
        );
        f.crew.extend(Arc::new(ReviewPolicy(policy)));
        f.crew.set_autopilot(true);
        f.crew.start(
            "lead",
            &model("cheap/model"),
            "Inspect the project".into(),
            PERSON,
        );
        if failed {
            let id = f.question().await;
            f.crew.answer(id, false);
        }
        f.settled().await;
        let seen = f.model.seen_by("reviewer");
        if failed {
            assert!(seen.is_empty());
            assert!(f.events().iter().any(|event| matches!(event, Event::Notice { text, .. } if text.contains("review skill unavailable"))));
        } else {
            assert!(seen.contains("Preserve local database fixtures."), "{seen}");
            assert!(seen.contains("never yours to approve"), "{seen}");
        }
    }
}

fn assert_refusal_guidance(seen: &str) {
    assert!(seen.contains("Do not get around it"), "{seen}");
    assert!(seen.contains("replacement tool"), "{seen}");
    assert!(seen.contains("tell the person what is blocked"), "{seen}");
}

#[tokio::test]
async fn a_reviewer_refusal_explains_the_reason_and_next_steps() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls("run", serde_json::json!({"command": "git reset --hard"})),
                    say("Blocked."),
                ],
            ),
            ("reviewer", vec![say("REFUSE: it would discard work.")]),
        ],
        Settings {
            reviewer: Some(model("cheap/reviewer")),
            ..Settings::default()
        },
    );
    f.crew.set_autopilot(true);
    f.crew.start(
        "lead",
        &model("cheap/model"),
        "Inspect the project".into(),
        PERSON,
    );
    f.settled().await;
    let seen = f.model.seen_by("lead");
    assert!(
        seen.contains("the reviewer refused it: it would discard work. Do not"),
        "{seen}"
    );
    assert_refusal_guidance(&seen);
}

#[tokio::test]
async fn the_reviewer_answers_within_the_sandbox_when_the_autopilot_is_on() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls("run", serde_json::json!({"command": "git reset --hard"})),
                    say("Reset."),
                ],
            ),
            ("reviewer", vec![say("APPROVE: it is what the task asks.")]),
        ],
        Settings {
            reviewer: Some(model("cheap/reviewer")),
            ..Settings::default()
        },
    );
    f.crew.set_autopilot(true);
    f.crew
        .start("lead", &model("cheap/model"), "start over".into(), PERSON);
    f.settled().await;
    let reviewed = f
        .events()
        .into_iter()
        .find_map(|event| match event {
            Event::Reviewed { approved, why, .. } => Some((approved, why)),
            _ => None,
        })
        .expect("the reviewer answered");
    assert_eq!(reviewed, (true, "it is what the task asks.".to_owned()));
    assert_eq!(f.model.models_of("reviewer"), ["cheap/reviewer"]);
    let asked = f.model.seen_by("reviewer");
    assert!(asked.contains("git reset --hard"), "{asked}");
    assert!(asked.contains("start over"), "{asked}");
    // No question went to the person.
    assert!(!f.events().iter().any(|e| matches!(e, Event::Asked { .. })));
}

#[tokio::test]
async fn agents_hand_work_to_each_other_on_the_board() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls(
                        "start_agent",
                        serde_json::json!({"name": "Helper", "model": "cheap/model", "task": "say hi"}),
                    ),
                    calls("wait_for_messages", serde_json::json!({"seconds": 5})),
                    say("The helper said hi."),
                ],
            ),
            ("helper", vec![say("hi from the helper")]),
        ],
        Settings::default(),
    );
    f.crew
        .start("lead", &model("cheap/model"), "get a hi".into(), PERSON);
    f.settled().await;
    assert!(
        f.model
            .seen_by("lead")
            .contains("Message from helper: hi from the helper"),
        "{}",
        f.model.seen_by("lead")
    );
    assert!(f.model.seen_by("helper").contains("say hi"));
    let listed = f.crew.list();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "helper");
    assert_eq!(listed[0].by, "lead");

    // The person writes to the lead, resting: it wakes and reads it.
    f.crew.tell("lead", "and now?".into()).unwrap();
    f.settled().await;
    assert!(f.model.seen_by("lead").contains("and now?"));
    assert!(f.crew.tell("nobody", "hi".into()).is_err());
}

#[tokio::test]
async fn the_autopilot_gives_the_internet_when_let_and_asks_the_person_otherwise() {
    let script = || {
        vec![
            calls(
                "run",
                serde_json::json!({"command": "curl -s https://example.com", "network": true}),
            ),
            say("Fetched."),
        ]
    };
    let settings = || Settings {
        reviewer: Some(model("cheap/reviewer")),
        ..Settings::default()
    };
    let approves = || ("reviewer", vec![say("APPROVE: the task needs the page.")]);
    // Let: the reviewer answers, the person is not asked.
    let f = fixture(&[("lead", script()), approves()], settings());
    f.crew.set_autopilot(true);
    f.crew.set_autopilot_internet(true);
    f.crew.start(
        "lead",
        &model("cheap/model"),
        "read the page".into(),
        PERSON,
    );
    f.settled().await;
    assert!(
        f.events()
            .iter()
            .any(|e| matches!(e, Event::Reviewed { approved: true, .. }))
    );
    assert!(!f.events().iter().any(|e| matches!(e, Event::Asked { .. })));
    // Not let: the person is asked, as without the autopilot.
    let f = fixture(&[("lead", script()), approves()], settings());
    f.crew.set_autopilot(true);
    f.crew.start(
        "lead",
        &model("cheap/model"),
        "read the page".into(),
        PERSON,
    );
    let id = f.question().await;
    f.crew.answer(id, false);
    f.settled().await;
    assert!(
        !f.events()
            .iter()
            .any(|e| matches!(e, Event::Reviewed { .. }))
    );
}

#[tokio::test]
async fn the_lead_follows_the_rules_and_gives_its_agents_what_they_need_of_them() {
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "start_agent",
                    serde_json::json!({"name": "helper", "model": "cheap/model", "task": "fix it, with no dashes"}),
                ),
                say("Started."),
            ],
        )],
        Settings {
            rules: vec![RulesFile {
                from: "~/.claude/CLAUDE.md".into(),
                text: "Answer in French.".into(),
            }],
            ..Settings::default()
        },
    );
    std::fs::write(f.root.join("AGENTS.md"), "# Rules\n\nNo dashes in prose.\n").unwrap();
    std::fs::write(f.root.join("CLAUDE.md"), "@AGENTS.md\nTests first.\n").unwrap();
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    let lead = f.model.instructions_of("lead");
    assert!(lead.contains("from their ~/.claude/CLAUDE.md") && lead.contains("Answer in French."));
    assert!(lead.contains("from its AGENTS.md") && lead.contains("from its CLAUDE.md"));
    assert!(lead.contains("Tests first."));
    // AGENTS.md, imported by CLAUDE.md, is given once.
    assert_eq!(lead.matches("No dashes in prose.").count(), 1);
    // The agent it starts reads only what it was given.
    let helper = f.model.seen_by("helper");
    assert!(helper.contains("fix it, with no dashes"));
    assert!(!helper.contains("Answer in French.") && !helper.contains("Tests first."));
    assert!(!helper.contains("rules for agents"));

    // A `CLAUDE.md` alone, importing another.
    let g = fixture(&[], Settings::default());
    std::fs::write(g.root.join("CLAUDE.md"), "Read these:\n@docs/rules.md\n").unwrap();
    std::fs::create_dir_all(g.root.join("docs")).unwrap();
    std::fs::write(g.root.join("docs/rules.md"), "Tests first.\n").unwrap();
    g.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    g.settled().await;
    let seen = g.model.seen_by("lead");
    assert!(seen.contains("from its CLAUDE.md") && seen.contains("Tests first."));
    assert!(!seen.contains("@docs/rules.md"));

    // None: nothing is said of rules.
    let h = fixture(&[], Settings::default());
    h.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    h.settled().await;
    assert!(!h.model.seen_by("lead").contains("rules for agents"));
}

/// Tools for leads alone, as the memory's and the skills' are.
struct ForLeads;

impl<M> Extension<M> for ForLeads {
    fn tools(&self, _agent: &str) -> Vec<iq_core::ToolSpec> {
        vec![iq_core::ToolSpec {
            name: "recall".into(),
            description: "Reads a note.".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]
    }

    fn leads_only(&self) -> bool {
        true
    }

    fn instructions(&self, _agent: &str) -> Option<String> {
        Some("Your notes: none yet.".into())
    }

    fn call(
        &self,
        _crew: Crew<M>,
        _agent: String,
        _name: String,
        _args: serde_json::Value,
    ) -> Called {
        Box::pin(std::future::ready(Ok("A note.".to_owned())))
    }
}

#[tokio::test]
async fn what_serves_leads_alone_is_not_the_agents_they_start() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls(
                        "start_agent",
                        serde_json::json!({"name": "helper", "model": "cheap/model", "task": "help"}),
                    ),
                    say("Started."),
                ],
            ),
            (
                "helper",
                vec![calls("recall", serde_json::json!({})), say("Done.")],
            ),
        ],
        Settings::default(),
    );
    f.crew.extend(Arc::new(ForLeads));
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    assert!(
        f.model
            .instructions_of("lead")
            .contains("Your notes: none yet.")
    );
    assert!(f.crew.tools_for("lead").iter().any(|t| t.name == "recall"));
    assert!(!f.model.instructions_of("helper").contains("Your notes"));
    assert!(
        f.crew
            .tools_for("helper")
            .iter()
            .all(|t| t.name != "recall")
    );
    assert!(
        f.model
            .seen_by("helper")
            .contains("there is no tool named \\\"recall\\\"")
    );
}

#[tokio::test]
async fn a_team_works_as_the_roles_it_was_given_say_and_answers_who_started_it() {
    use iq_core::Effort;
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls(
                        "start_team",
                        serde_json::json!({
                            "name": "code",
                            "task": "Add a greeting to hello.txt.",
                            "members": [
                                {"role": "writer", "model": "lab/a", "effort": "high", "does": "writes the change, then has it reviewed"},
                                {"role": "reviewer", "model": "lab/b", "does": "reads the diff; APPROVE or what to change, two rounds at most"},
                            ],
                        }),
                    ),
                    say("Started."),
                    say("Thanks."),
                ],
            ),
            (
                "code-writer",
                vec![
                    calls(
                        "send_message",
                        serde_json::json!({"to": "code-reviewer", "text": "Review my change."}),
                    ),
                    calls("wait_for_messages", serde_json::json!({"seconds": 5})),
                    say("Written and approved."),
                ],
            ),
            ("code-reviewer", vec![say("APPROVE")]),
        ],
        Settings::default(),
    );
    f.crew
        .start("lead", &model("cheap/model"), "greet".into(), PERSON);
    f.settled().await;
    let listed = f.crew.list();
    let member = |name: &str| listed.iter().find(|a| a.name == name).unwrap().clone();
    assert_eq!(
        (
            member("code-writer").model.as_str(),
            member("code-writer").by.as_str()
        ),
        ("lab/a", "lead")
    );
    assert_eq!(
        (
            member("code-reviewer").model.as_str(),
            member("code-reviewer").by.as_str()
        ),
        ("lab/b", "code-writer")
    );
    assert!(
        f.model
            .efforts_of("code-writer")
            .iter()
            .all(|e| *e == Some(Effort::High))
    );
    assert_eq!(f.model.efforts_of("code-reviewer"), [None]);
    // The reviewer waited for the writer, its brief and its team before it.
    let reviewer = f.model.seen_by("code-reviewer");
    assert!(reviewer.contains("Add a greeting to hello.txt."));
    assert!(reviewer.contains("You are code-reviewer") && reviewer.contains("goes to code-writer"));
    assert!(reviewer.contains("Review my change."));
    assert!(f.model.seen_by("code-writer").contains("APPROVE"));
    // Only the writer's answer reached the lead.
    let lead = f.model.seen_by("lead");
    assert!(lead.contains("The team code is started") && lead.contains("Written and approved."));
    assert!(!lead.contains("Message from code-reviewer"));
}

#[tokio::test]
async fn an_agent_given_a_role_is_told_to_keep_to_it() {
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "start_agent",
                    serde_json::json!({"name": "checker", "model": "cheap/model", "task": "check", "role": "tests"}),
                ),
                say("Started."),
            ],
        )],
        Settings::default(),
    );
    f.crew.start(
        "lead",
        &model("cheap/model"),
        "split the work".into(),
        PERSON,
    );
    f.settled().await;
    assert!(
        f.model
            .seen_by("checker")
            .contains("Your role in the crew: tests")
    );
    let listed = f.crew.list();
    let checker = listed.iter().find(|a| a.name == "checker").unwrap();
    assert_eq!(checker.role.as_deref(), Some("tests"));
}

#[tokio::test]
async fn a_writer_and_its_reviewer_go_round_until_it_approves() {
    let f = fixture(
        &[
            (
                "writer",
                vec![say("first try"), say("second try, with a test")],
            ),
            (
                "review",
                vec![
                    say("CHANGES: add a test"),
                    say("APPROVE: it has its test now"),
                ],
            ),
        ],
        Settings::default(),
    );
    f.crew.launch(
        Pattern::Pair,
        "writer",
        &[model("cheap/model"), model("cheap/other")],
        "make it work".into(),
        None,
        None,
    );
    for _ in 0..4 {
        f.settled().await;
    }
    assert!(
        f.model
            .seen_by("review")
            .contains("Review what writer changed")
    );
    assert!(f.model.seen_by("writer").contains("CHANGES: add a test"));
    let to_person: Vec<String> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Posted { from, to, text } if from == "review" && to == PERSON => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(to_person, vec!["APPROVE: it has its test now".to_owned()]);
    assert!(f.events().iter().any(
        |e| matches!(e, Event::Notice { text, .. } if text.contains("approved it after 2 round"))
    ));
}

#[tokio::test]
async fn debaters_answer_alone_then_read_each_other_and_the_first_sums_up() {
    let f = fixture(
        &[
            (
                "think",
                vec![
                    say("use a map"),
                    say("a map, still"),
                    say("we agree on a map"),
                ],
            ),
            ("think-2", vec![say("use a list"), say("a map is better")]),
        ],
        Settings::default(),
    );
    let first = f.crew.launch(
        Pattern::Debate,
        "think",
        &[model("cheap/model"), model("cheap/other")],
        "how to store it".into(),
        None,
        None,
    );
    assert_eq!(first, "think");
    for _ in 0..4 {
        f.settled().await;
    }
    assert!(f.model.seen_by("think-2").contains("use a map"));
    assert!(f.model.seen_by("think").contains("a map is better"));
    let done: Vec<String> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Done {
                agent,
                outcome: Ok(text),
            } if agent == "think" => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(done.last().map(String::as_str), Some("we agree on a map"));
}

#[tokio::test]
async fn an_answer_written_back_is_not_sent_twice() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls(
                        "start_agent",
                        serde_json::json!({"name": "helper", "model": "cheap/model", "task": "count"}),
                    ),
                    calls("wait_for_messages", serde_json::json!({"seconds": 5})),
                    say("Counted."),
                ],
            ),
            (
                "helper",
                vec![
                    calls(
                        "send_message",
                        serde_json::json!({"to": "lead", "text": "three"}),
                    ),
                    say("three"),
                ],
            ),
        ],
        Settings::default(),
    );
    f.crew.start(
        "lead",
        &model("cheap/model"),
        "count with a helper".into(),
        PERSON,
    );
    f.settled().await;
    let posted = f
        .events()
        .into_iter()
        .filter(|e| matches!(e, Event::Posted { from, to, .. } if from == "helper" && to == "lead"))
        .count();
    assert_eq!(posted, 1);
}

#[tokio::test]
async fn agents_wake_each_other_a_few_times_then_wait_for_the_person() {
    // Two agents the person started, writing to each other at each turn.
    let ping = |to: &str| {
        (0..10)
            .flat_map(|_| {
                [
                    calls(
                        "send_message",
                        serde_json::json!({"to": to, "text": "your turn"}),
                    ),
                    say("over"),
                ]
            })
            .collect::<Vec<_>>()
    };
    let f = fixture(
        &[("alpha", ping("beta")), ("beta", ping("alpha"))],
        Settings::default(),
    );
    f.crew
        .start("beta", &model("cheap/model"), "wait".into(), PERSON);
    f.settled().await;
    f.crew.start(
        "alpha",
        &model("cheap/model"),
        "start the talk".into(),
        PERSON,
    );
    f.settled().await;
    let turns = |agent: &str| {
        f.events()
            .iter()
            .filter(|e| matches!(e, Event::Working { agent: a, .. } if a == agent))
            .count()
    };
    // Each started once by the person, then woken by the other, at most
    // four times between them.
    assert!(
        turns("alpha") + turns("beta") <= 2 + 4,
        "{} {}",
        turns("alpha"),
        turns("beta")
    );
    assert!(turns("alpha") + turns("beta") > 2);
    assert!(
        f.events()
            .iter()
            .any(|e| matches!(e, Event::Notice { text, .. } if text.contains("woke each other")))
    );
}

#[tokio::test]
async fn a_reply_of_nothing_is_urged_on_once_then_fails_the_turn() {
    let nothing = || ChatResponse {
        content: Some("  ".into()),
        ..calls("none", serde_json::json!({}))
    };
    let empty = || ChatResponse {
        tool_calls: Vec::new(),
        ..nothing()
    };
    let f = fixture(
        &[
            ("lead", vec![empty(), say("Here it is.")]),
            ("idle", vec![empty(), empty(), say("never said")]),
        ],
        Settings::default(),
    );
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.crew
        .start("idle", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    let events = f.events();
    assert!(events.contains(&Event::Done {
        agent: "lead".into(),
        outcome: Ok("Here it is.".into())
    }));
    assert!(
        f.model
            .seen_by("lead")
            .contains("neither text nor a tool call")
    );
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Done { agent, outcome: Err(why) } if agent == "idle" && why.contains("nothing")
    )));
}

#[tokio::test]
async fn the_lead_starts_its_agents_on_the_models_it_chooses() {
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "start_agent",
                    serde_json::json!({"name": "helper", "model": "cheap/other", "task": "fix calc.py"}),
                ),
                say("Started."),
            ],
        )],
        Settings::default(),
    );
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    // iq holds it to nothing: what it starts is its choice.
    assert_eq!(f.model.models_of("helper"), ["cheap/other"]);
}

#[tokio::test]
async fn the_lead_chooses_how_hard_its_agents_think_and_the_person_its_own() {
    use iq_core::Effort;
    let f = fixture(
        &[(
            "lead",
            vec![
                calls(
                    "start_agent",
                    serde_json::json!({"name": "helper", "model": "cheap/other", "task": "rename it", "effort": "low"}),
                ),
                say("Started."),
                say("Again."),
            ],
        )],
        Settings {
            effort: Some(Effort::High),
            ..Settings::default()
        },
    );
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    assert_eq!(f.model.efforts_of("helper"), [Some(Effort::Low)]);
    // The lead, as the settings say, its turns woken by the helper too.
    assert!(
        f.model
            .efforts_of("lead")
            .iter()
            .all(|e| *e == Some(Effort::High))
    );
    f.crew.set_effort("lead", Effort::Max).unwrap();
    assert!(f.events().contains(&Event::Effort {
        agent: "lead".into(),
        effort: Some(Effort::Max)
    }));
    f.crew.tell("lead", "once more".into()).unwrap();
    f.settled().await;
    assert_eq!(f.model.efforts_of("lead").last(), Some(&Some(Effort::Max)));
    assert!(f.crew.set_effort("nobody", Effort::Low).is_err());
}

#[tokio::test]
async fn an_agent_is_told_its_conversation_grows_and_compacts_it_once_its_request_is_done() {
    let f = fixture(
        &[(
            "lead",
            vec![
                reading(30_000, calls("list_agents", serde_json::json!({}))),
                reading(
                    31_000,
                    calls(
                        "compact",
                        serde_json::json!({"summary": "Listed the agents; the tests remain."}),
                    ),
                ),
                reading(32_000, say("Done.")),
                say("On it."),
            ],
        )],
        Settings::default(),
    );
    f.crew
        .start("lead", &model("cheap/model"), "list them".into(), PERSON);
    f.settled().await;
    let requests = f.model.requests_of("lead");
    // Told once, as its conversation passed the step.
    assert!(!format!("{:?}", requests[0].messages).contains("(iq: your conversation"));
    assert!(
        format!("{:?}", requests[1].messages)
            .contains("(iq: your conversation is 30.0k tokens now, read again on each call.)")
    );
    assert_eq!(
        format!("{:?}", requests[2].messages)
            .matches("(iq: your conversation")
            .count(),
        1
    );
    assert!(f.events().iter().any(|event| matches!(
        event,
        Event::Notice { agent, text } if agent == "lead" && text.starts_with("compacted: its calls read 32.0k tokens")
    )));
    // Its request done, its conversation is its task, its summary and its
    // answer.
    f.crew.tell("lead", "now the tests".into()).unwrap();
    f.settled().await;
    let last = f.model.requests_of("lead").pop().unwrap();
    assert_eq!(last.messages.len(), 4, "{:?}", last.messages);
    let seen = format!("{:?}", last.messages);
    assert!(seen.contains("list them") && seen.contains("Listed the agents; the tests remain."));
    assert!(seen.contains("Done.") && seen.contains("now the tests"));
    assert!(!seen.contains("call-list_agents"));
}

#[tokio::test]
async fn an_agent_is_given_the_rules_for_the_files_it_touches_once() {
    let f = fixture(
        &[
            (
                "lead",
                vec![
                    calls(
                        "start_agent",
                        serde_json::json!({"name": "helper", "model": "cheap/model", "task": "fix the api"}),
                    ),
                    say("Started."),
                ],
            ),
            (
                "helper",
                vec![
                    calls(
                        "read_file",
                        serde_json::json!({"path": "src/api/routes.ts"}),
                    ),
                    calls(
                        "read_file",
                        serde_json::json!({"path": "./src/api/models.ts"}),
                    ),
                    calls("read_file", serde_json::json!({"path": "src/ui/page.tsx"})),
                    say("Fixed."),
                ],
            ),
        ],
        Settings::default(),
    );
    std::fs::create_dir_all(f.root.join(".claude/rules/web")).unwrap();
    std::fs::write(
        f.root.join(".claude/rules/api.md"),
        "---\npaths:\n  - \"src/api/**\"\n---\nValidate every input at the edge.\n",
    )
    .unwrap();
    std::fs::write(
        f.root.join(".claude/rules/web/ui.md"),
        "---\npaths: src/ui/**/*.{ts,tsx}\n---\nNo inline styles.\n",
    )
    .unwrap();
    std::fs::write(f.root.join(".claude/rules/style.md"), "Short functions.\n").unwrap();
    std::fs::create_dir_all(f.root.join("src/api")).unwrap();
    std::fs::create_dir_all(f.root.join("src/ui")).unwrap();
    for file in ["src/api/routes.ts", "src/api/models.ts", "src/ui/page.tsx"] {
        std::fs::write(f.root.join(file), "export {}\n").unwrap();
    }
    f.crew
        .start("lead", &model("cheap/model"), "go".into(), PERSON);
    f.settled().await;
    // The lead reads the rules for the whole project, and is told of the
    // others and their files.
    let lead = f.model.instructions_of("lead");
    assert!(lead.contains("from its .claude/rules/style.md") && lead.contains("Short functions."));
    assert!(lead.contains("- .claude/rules/api.md: src/api/**"));
    assert!(lead.contains("- .claude/rules/web/ui.md: src/ui/**/*.{ts,tsx}"));
    assert!(!lead.contains("Validate every input"));
    // The helper, as it first touches their files, once each in its
    // conversation.
    let last = f.model.requests_of("helper").pop().unwrap();
    let helper = format!("{:?}", last.messages);
    assert_eq!(
        helper.matches("Validate every input at the edge.").count(),
        1
    );
    assert!(helper.contains("No inline styles."));
    assert!(!helper.contains("Short functions."));
}
