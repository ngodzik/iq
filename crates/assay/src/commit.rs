//! An exercise made from a commit of the person's: the folder as it was
//! before it, for candidates to start from, and what the commit changed,
//! beside, for the judge to write the task and pick the tests. git does
//! it, read only, once the gate let the judge read the repository.

use std::path::Path;

use tokio::process::Command;

/// What git is run with: nothing of the repository's settings runs a
/// program of its own while iq reads it.
const QUIET: &[&str] = &["-c", "core.fsmonitor=", "-c", "diff.external="];

/// Runs git in `repository` with `args`, and gives what it printed.
async fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(QUIET)
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git could not be run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

async fn git_text(repository: &Path, args: &[&str]) -> Result<String, String> {
    git(repository, args)
        .await
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Writes the tree of `revision`, or only `paths` of it, into `into`.
async fn export(
    repository: &Path,
    revision: &str,
    paths: &[&str],
    into: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let archive = into.with_extension("tar");
    let file = archive.to_string_lossy().into_owned();
    let mut args = vec!["archive", "--format=tar", "-o", file.as_str(), revision];
    if !paths.is_empty() {
        args.push("--");
        args.extend(paths);
    }
    git(repository, &args).await?;
    let untar = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(into)
        .output()
        .await
        .map_err(|e| format!("tar could not be run: {e}"));
    let _ = std::fs::remove_file(&archive);
    let untar = untar?;
    if !untar.status.success() {
        return Err(format!(
            "tar failed: {}",
            String::from_utf8_lossy(&untar.stderr).trim()
        ));
    }
    Ok(())
}

/// Makes the draft exercise `folder` from `commit` of `repository`, a
/// folder before it of `most` files at most. Returns what it made, for the
/// judge.
pub(crate) async fn exercise(
    repository: &Path,
    commit: &str,
    folder: &Path,
    most: usize,
) -> Result<String, String> {
    let sha = git_text(
        repository,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .await?
    .trim()
    .to_owned();
    let parent = git_text(repository, &["rev-parse", "--verify", &format!("{sha}^")])
        .await
        .map_err(|_| format!("{commit} has no parent: there is no folder before it"))?
        .trim()
        .to_owned();
    let before = git_text(repository, &["ls-tree", "-r", "--name-only", &parent]).await?;
    let count = before.lines().count();
    if count > most {
        return Err(format!(
            "the folder before {commit} holds {count} files, more than the {most} an exercise may: take a commit of a smaller project"
        ));
    }
    let changed = git_text(
        repository,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            &parent,
            &sha,
        ],
    )
    .await?;
    let changed: Vec<&str> = changed.lines().filter(|l| !l.is_empty()).collect();
    export(repository, &parent, &[], &folder.join("start")).await?;
    let present = git_text(repository, &["ls-tree", "-r", "--name-only", &sha])
        .await?
        .lines()
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    let after: Vec<&str> = changed
        .iter()
        .copied()
        .filter(|path| present.contains(*path))
        .collect();
    if !after.is_empty() {
        export(repository, &sha, &after, &folder.join("after")).await?;
    }
    let diff = git(
        repository,
        &["diff", "--no-ext-diff", "--no-textconv", &parent, &sha],
    )
    .await?;
    write(&folder.join("reference.diff"), &diff)?;
    let message = git_text(repository, &["log", "-1", "--format=%B", &sha]).await?;
    let title = message.lines().next().unwrap_or("").trim().to_owned();
    write(
        &folder.join("source.md"),
        format!(
            "Made from commit {sha} of {}, as the folder was before it.\n\nIts message:\n\n{}\n",
            repository.display(),
            message.trim()
        )
        .as_bytes(),
    )?;
    let quoted = |text: &str| toml::Value::String(text.to_owned()).to_string();
    write(
        &folder.join("exercise.toml"),
        format!(
            "title = {}\ncategories = []\n# check = \"the command that runs the hidden tests in checks/\"\nminutes = 15\nsource = {}\n",
            quoted(&title),
            quoted(&format!("commit {sha} of {}", repository.display()))
        )
        .as_bytes(),
    )?;
    let tests: Vec<&str> = changed
        .iter()
        .copied()
        .filter(|p| {
            let p = p.to_lowercase();
            p.contains("test") || p.contains("spec")
        })
        .collect();
    let listed: Vec<&str> = changed.iter().copied().take(60).collect();
    Ok(format!(
        "Made the draft {}: start/ ({count} files, the folder before the commit), reference.diff, after/ (the {} files it changed, as after it), source.md, and exercise.toml to finish.\n\
         It changed: {}{}\n\
         Of these, tests at a guess: {}.\n\
         Now write task.md (what to do, as the person would ask it, nothing of the solution), rubric.md, put the tests that decide it in checks/ (from after/), set `check`, `categories` and `minutes` in exercise.toml, then ask to freeze it.",
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        after.len(),
        listed.join(", "),
        if changed.len() > listed.len() {
            format!(" and {} more", changed.len() - listed.len())
        } else {
            String::new()
        },
        if tests.is_empty() {
            "none".to_owned()
        } else {
            tests.join(", ")
        }
    ))
}

fn write(file: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(file, bytes).map_err(|e| format!("{}: {e}", file.display()))
}
