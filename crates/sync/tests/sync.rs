//! Two machines syncing through a repository on disk: no network, git
//! alone.

use std::path::{Path, PathBuf};

use iq_sync::Sync;

struct World {
    _dir: tempfile::TempDir,
    remote: String,
    root: PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let remote = root.join("remote.git");
    let status = std::process::Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&remote)
        .status()
        .unwrap();
    assert!(status.success());
    World {
        remote: remote.display().to_string(),
        root,
        _dir: dir,
    }
}

impl World {
    fn machine(&self, name: &str) -> Sync {
        self.keeping(name, &[])
    }

    fn keeping(&self, name: &str, kept: &[&str]) -> Sync {
        Sync::new(self.root.join(name).join("data"), &self.remote, name)
            .mirror(
                self.root.join(name).join("home/settings.toml"),
                "config/settings.toml",
            )
            .keeping(
                self.home(name),
                kept.iter().map(|k| (*k).to_owned()).collect(),
            )
    }

    fn home(&self, name: &str) -> PathBuf {
        self.root.join(name).join("home")
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let at = root.join(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
}

fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

#[tokio::test]
async fn a_new_machine_takes_what_is_there_before_it_sends_anything() {
    let w = world();
    let a = w.machine("a");
    write(a.root(), "assay/README.md", "the assay\n");
    write(a.root(), "memory/notes.md", "kept on a\n");
    write(&w.root, "a/home/settings.toml", "theme = \"ink\"\n");
    let first = a.run().await.unwrap();
    assert!(first.fresh);
    assert!(
        first.sent.contains(&"config/settings.toml".to_owned()),
        "{first:?}"
    );

    // b had a README of its own, a note a never saw, and its own settings.
    let b = w.machine("b");
    write(b.root(), "assay/README.md", "made fresh on b\n");
    write(b.root(), "memory/only-b.md", "b's\n");
    write(&w.root, "b/home/settings.toml", "theme = \"porcelain\"\n");
    let second = b.run().await.unwrap();
    assert!(second.fresh);
    assert_eq!(
        read(b.root(), "assay/README.md").as_deref(),
        Some("the assay\n")
    );
    assert_eq!(
        read(b.root(), "memory/notes.md").as_deref(),
        Some("kept on a\n")
    );
    // What it had and the repository did not is sent, not lost.
    assert_eq!(second.sent, ["memory/only-b.md"]);
    // Its settings are the synced ones, its own kept beside.
    assert_eq!(
        read(&w.root, "b/home/settings.toml").as_deref(),
        Some("theme = \"ink\"\n")
    );
    assert_eq!(
        read(&w.root, "b/home/settings.toml.before-sync").as_deref(),
        Some("theme = \"porcelain\"\n")
    );
    let back = a.run().await.unwrap();
    assert_eq!(back.taken, ["memory/only-b.md"]);
}

#[tokio::test]
async fn changes_on_each_side_meet_and_this_machine_wins_where_they_overlap() {
    let w = world();
    let a = w.machine("a");
    write(a.root(), "shared.md", "one\ntwo\nthree\n");
    a.run().await.unwrap();
    let b = w.machine("b");
    b.run().await.unwrap();

    // Apart: both are kept.
    write(a.root(), "from-a.md", "a\n");
    write(b.root(), "from-b.md", "b\n");
    a.run().await.unwrap();
    let merged = b.run().await.unwrap();
    assert_eq!(merged.taken, ["from-a.md"]);
    a.run().await.unwrap();
    assert_eq!(read(a.root(), "from-b.md").as_deref(), Some("b\n"));

    // The same line: the machine that syncs keeps its own.
    write(a.root(), "shared.md", "one\nTWO from a\nthree\n");
    write(b.root(), "shared.md", "one\nTWO from b\nthree\n");
    a.run().await.unwrap();
    let overlap = b.run().await.unwrap();
    assert_eq!(overlap.kept_here, ["shared.md"]);
    assert_eq!(
        read(b.root(), "shared.md").as_deref(),
        Some("one\nTWO from b\nthree\n")
    );
    let taken = a.run().await.unwrap();
    assert_eq!(taken.taken, ["shared.md"]);
    assert_eq!(
        read(a.root(), "shared.md").as_deref(),
        Some("one\nTWO from b\nthree\n")
    );

    // Removed on one side, changed on the other: the side that syncs stands.
    std::fs::remove_file(a.root().join("from-a.md")).unwrap();
    write(b.root(), "from-a.md", "a, changed on b\n");
    a.run().await.unwrap();
    b.run().await.unwrap();
    assert_eq!(
        read(b.root(), "from-a.md").as_deref(),
        Some("a, changed on b\n")
    );
    a.run().await.unwrap();
    assert_eq!(
        read(a.root(), "from-a.md").as_deref(),
        Some("a, changed on b\n")
    );
}

#[tokio::test]
async fn what_only_this_machine_needs_stays_here() {
    let w = world();
    let a = w.machine("a");
    write(a.root(), "assay/runs/add/20261024-1/A/run.json", "{}\n");
    write(
        a.root(),
        "assay/runs/add/20261024-1/A/work/node_modules/x.js",
        "big\n",
    );
    write(a.root(), "assay/judging/add-1/A.diff", "+x\n");
    let outcome = a.run().await.unwrap();
    assert!(
        outcome
            .sent
            .contains(&"assay/runs/add/20261024-1/A/run.json".to_owned())
    );
    assert!(
        outcome
            .sent
            .iter()
            .all(|f| !f.contains("/work/") && !f.contains("judging")),
        "{outcome:?}"
    );
    let b = w.machine("b");
    b.run().await.unwrap();
    assert!(read(b.root(), "assay/runs/add/20261024-1/A/run.json").is_some());
    assert!(
        read(
            b.root(),
            "assay/runs/add/20261024-1/A/work/node_modules/x.js"
        )
        .is_none()
    );
    // Nothing changed: nothing is committed or sent.
    assert!(!a.pending().await);
    assert!(a.run().await.unwrap().sent.is_empty());
    write(a.root(), "memory/new.md", "x\n");
    assert!(a.pending().await);
    write(&w.root, "a/home/settings.toml", "changed\n");
    a.run().await.unwrap();
    assert!(!a.pending().await);
    write(&w.root, "a/home/settings.toml", "changed again\n");
    assert!(a.pending().await);
}

#[tokio::test]
async fn a_repository_out_of_reach_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let lost = Sync::new(
        dir.path().join("data"),
        dir.path().join("nowhere.git").display().to_string(),
        "a",
    );
    let error = lost.run().await.unwrap_err().to_string();
    assert!(error.contains("fetch"), "{error}");
}

#[tokio::test]
async fn what_the_person_keeps_comes_back_on_a_new_machine_without_its_secrets() {
    let w = world();
    let listed = [
        "~/notes",
        "~/.zshrc",
        "~/.ssh/config",
        "~/../elsewhere",
        "notes",
        "~/.iq",
    ];
    let a = w.keeping("a", &listed);
    let home = w.home("a");
    write(&home, "notes/todo.md", "buy milk\n");
    write(&home, "notes/ideas/iq.md", "an assay\n");
    write(&home, "notes/.env", "TOKEN=abc\n");
    write(&home, "notes/node_modules/x.js", "rebuilt\n");
    write(&home, ".zshrc", "alias iq=...\n");
    write(&home, ".ssh/config", "Host x\n");
    assert_eq!(a.kept(), ["~/notes", "~/.zshrc"]);
    let refused = a.refused().join("\n");
    // iq's own folder is among those the gate holds as secret.
    assert_eq!(a.refused().len(), 4, "{refused}");
    for why in ["holds secrets", "plainly", "write it as ~/"] {
        assert!(refused.contains(why), "{refused}");
    }
    let sent = a.run().await.unwrap().sent;
    assert!(
        sent.contains(&"home/notes/ideas/iq.md".to_owned()),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .all(|f| !f.contains(".env") && !f.contains("node_modules") && !f.contains(".ssh")),
        "{sent:?}"
    );

    // A new machine, its home empty but for an older .zshrc of its own.
    let b = w.keeping("b", &listed);
    write(&w.home("b"), ".zshrc", "old\n");
    b.run().await.unwrap();
    assert_eq!(
        read(&w.home("b"), "notes/ideas/iq.md").as_deref(),
        Some("an assay\n")
    );
    assert_eq!(
        read(&w.home("b"), ".zshrc").as_deref(),
        Some("alias iq=...\n")
    );
    assert_eq!(
        read(&w.home("b"), ".zshrc.before-sync").as_deref(),
        Some("old\n")
    );
    assert!(read(&w.home("b"), "notes/.env").is_none());

    // A file removed here goes from the repository, not from the others' homes.
    std::fs::remove_file(home.join("notes/todo.md")).unwrap();
    assert!(a.pending().await);
    a.run().await.unwrap();
    b.run().await.unwrap();
    assert!(read(b.root(), "home/notes/todo.md").is_none());
    assert_eq!(
        read(&w.home("b"), "notes/todo.md").as_deref(),
        Some("buy milk\n")
    );
}

#[tokio::test]
async fn rules_and_notes_are_kept_from_a_folder_of_credentials_and_a_folder_mirrored_whole() {
    let w = world();
    let listed = [
        "~/.claude/CLAUDE.md",
        "~/.claude/projects/app/memory",
        "~/.claude/.credentials.json",
    ];
    let skills = |name: &str| w.root.join(name).join("home/.iq/skills");
    let a = w.keeping("a", &listed).mirror(skills("a"), "config/skills");
    let home = w.home("a");
    write(&home, ".claude/CLAUDE.md", "Answer in French.\n");
    write(&home, ".claude/.credentials.json", "{}\n");
    write(
        &home,
        ".claude/projects/app/memory/deploy.md",
        "Tag first.\n",
    );
    write(&home, ".claude/projects/app/memory/log.jsonl", "{}\n");
    write(
        &home,
        ".iq/skills/teams/SKILL.md",
        "A writer, a reviewer.\n",
    );
    assert_eq!(
        a.kept(),
        ["~/.claude/CLAUDE.md", "~/.claude/projects/app/memory"]
    );
    assert!(a.refused()[0].contains("holds secrets"));
    // Never synced: it takes first, nothing is asked.
    assert!(a.changes().await.is_empty() && a.pending().await);
    let sent = a.run().await.unwrap().sent;
    for file in [
        "home/.claude/CLAUDE.md",
        "home/.claude/projects/app/memory/deploy.md",
        "config/skills/teams/SKILL.md",
    ] {
        assert!(sent.contains(&file.to_owned()), "{sent:?}");
    }
    assert!(
        sent.iter()
            .all(|f| !f.contains("credentials") && !f.ends_with(".jsonl"))
    );
    assert!(a.changes().await.is_empty());

    // What changed is listed before anything is sent.
    write(&home, ".claude/CLAUDE.md", "Answer in English.\n");
    write(&home, ".iq/skills/compact.md", "Between tasks.\n");
    std::fs::remove_file(home.join(".iq/skills/teams/SKILL.md")).unwrap();
    assert_eq!(
        a.changes().await,
        [
            "config/skills/compact.md",
            "config/skills/teams/SKILL.md",
            "home/.claude/CLAUDE.md"
        ]
    );
    a.run().await.unwrap();

    // Another machine finds the skills in its own iq folder.
    let b = w.keeping("b", &listed).mirror(skills("b"), "config/skills");
    b.run().await.unwrap();
    assert_eq!(
        read(&skills("b"), "compact.md").as_deref(),
        Some("Between tasks.\n")
    );
    assert!(read(&skills("b"), "teams/SKILL.md").is_none());
    assert_eq!(
        read(&w.home("b"), ".claude/CLAUDE.md").as_deref(),
        Some("Answer in English.\n")
    );
}
