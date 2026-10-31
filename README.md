# iq

**An experimental harness for coding agents. It is not stable, it comes with no guarantee of any kind, and it changes all the time.**

This is a personal project, not a product. Commands, settings and file formats change without notice, and whatever is described here may be changed or removed at any time. Its security has not been audited. If you want a coding agent to rely on, use an established one instead.

## What it is

iq runs coding agents in the terminal, on your machine. For now, the models answer through Requesty's router alone; other providers are meant to come later. A lead agent takes your request and may start other agents, with the roles, models and effort it chooses; they write to each other and to you on a shared board, and each chat may work in a git worktree of its own. iq is the harness around them: the gate, the sandbox, the board and the costs. The models do the work.

Three principles hold it together:

- **Security is the foundation, not a layer.** Everything an agent does goes through one gate, and every command runs in the operating system's sandbox.
- **Your skills say how the work is done.** iq ships no method of its own. Which team to start for a kind of work, with which roles and models, how its agents work together and review each other: you write it in your skills, and you find your own formula. iq makes it easy to follow: agents that start others, messages between them, a worktree per chat, every step in view.
- **Every cost is in view.** Each call to a model says what it cost, summed by model, by kind of work and over time. iq never holds an agent back to save money: you see what it spends, and you decide.

## Security

This is how iq is designed to hold, and which part of its code holds what: the code is split into crates, libraries each with one responsibility. It is a design, not a guarantee.

- **One gate** (the `iq-gate` crate). Reading or writing a file, running a command, reaching the network: every action an agent asks for is judged there, then allowed, asked of you, or refused. Only the gate can issue the permit an action needs, so the compiler itself refuses a way around it. Every decision is written to an audit log before the action is taken, and the rules that judge commands live in iq's own folder, never in the project, which commands can write.
- **The operating system's sandbox** (the `iq-gate` crate). Commands run confined by the kernel: Landlock and seccomp on Linux, Seatbelt on macOS; other systems refuse commands until their sandbox is written. A command writes only where the work is, always reaches localhost (on Linux, through a network of the sandbox's own), and reaches the internet only when a rule gives it (fetching packages and sources), when you allow it, or once you opened it for every command. A command waiting for an answer nobody can give is stopped, and the git hooks a command leaves behind are put back.
- **Secrets never reach a model.** Files that hold secrets are refused, the secrets of your environment are kept out of commands, and what commands print is cleared of the project's own. API keys come from the environment only.
- **The machine itself.** What needs your machine outside the sandbox (the container daemon, a cluster, a forge's or a cloud's command line, with your credentials) runs there only once approved, as you would run it yourself.
- **What others share waits for you.** Publishing, or anything that changes what other people see, is always yours to approve.

To report a vulnerability, please use the repository's private vulnerability reporting rather than a public issue.

## The autopilot

By default, every question the gate asks comes to you. With the autopilot on, a reviewing model answers in your place, within bounds it cannot widen:

- it answers what stays within the sandbox and, when you let it, what only reaches the internet and what runs on the machine itself;
- it follows a policy you write yourself, as one of your skills: read again at each decision, kept outside the project so that no command can rewrite it, and never shown to the agents it judges;
- it sees the agent's task, your latest request and the agent's last actions, so that each decision rests on what is happening;
- publishing, changing what others share, files outside the project and secrets are never its to allow, and after a few refusals in a row the agent's next action comes back to you.

Rules and the reviewer only spare you questions: they widen nothing you did not allow.

## Rules, skills, notes and sync

- **Rules.** The agents follow the project's own rules (its `AGENTS.md`, and the rule files many projects already keep for their agents) and the rules you give every project.
- **Skills.** Markdown files of yours, each for a kind of work: an agent reads one before the work it serves, and gives the agents it starts what their part needs of it. The autopilot's policy is one of them, kept for the reviewer alone.
- **Notes.** Each project has notes the agents keep for themselves, found again from the same repository on another machine.
- **Sync.** Your settings, your command rules, the rules and skills kept in iq's own folder (the autopilot's policy among them), the projects' notes, and any file of your home you list can be kept in a private git repository of yours, on GitHub or any other git host, and are put back on a new machine. What a sync would send is shown line by line for you to approve, unless you let it go on its own. What holds secrets is never kept, and a new machine takes what is there before it sends anything. Git does it with the machine's own credentials: nothing goes anywhere but the repository you named.
- **What stays on the machine.** The chats, the journal of tasks and costs, the observations and the audit log: they are not synced.

## Plugins

iq can be extended by plugins: programs of their own, started only when your settings name them, which speak with iq over a small protocol of JSON lines (the `iq-plugin` crate). A plugin may add commands, show a line of status, and give the commands run in a place the variables they need there. What only some projects need (containers, a cluster, a cloud) belongs in a plugin, while iq keeps the generic part. A plugin runs on your machine as you and answers to you alone: no agent can start one or speak to it, what it gives is written to the audit log, and the gate refuses the variables its sandbox rests on. No plugin is published yet.

## How it is built

A Rust workspace whose crate graph enforces the separation: the agents never depend on the providers, the interface's state is tested without a terminal, and nothing reaches the machine without the gate's permit. `AGENTS.md` describes each crate and the rules every agent working on iq follows, iq's own among them.

## Usage

```bash
export REQUESTY_API_KEY=...   # Requesty's key, read from the environment only
iq                            # runs in the terminal
iq do "..."                   # hands a task to the agents at once
iq --help
```

## Building

iq runs on Linux and macOS, the systems its sandbox is written for.

```bash
cargo install --path .              # builds the iq binary into ~/.cargo/bin
scripts/check.sh                    # formatting, lints, docs, tests, dependency audit
git config core.hooksPath .githooks # run those checks before every commit
```

## Data from others

None of this data ships with the program: each install fetches it for its own use and says where it comes from.

- **Model lists, prices and context windows** for the provider's models come from the provider itself.

## License

MIT
