# Harness hooks

A managed directory: updated by the template in all projects, not edited by hand. The project puts its own hooks in `.harness/local/hooks/<slot>/`.

The agent doesn't call hooks one by one but a dispatcher: `dispatch.sh <slot>` with the event JSON on stdin. In Claude Code the wiring is in `.claude/settings.json`; another agent's shim calls the same dispatcher with the same input.

## Codex adapter

Codex loads `.codex/hooks.json` after the project and its hooks are trusted. Bash events and executor `SubagentStop` events use the dispatcher contract directly. The adapter reads the file directives in an `apply_patch` event, runs the edit slots for each affected path, and returns one response. It runs the `post-edit-async` slot synchronously and translates its stderr notification into tool-response context. The `pre-handback` slot is specific to Claude Code.

## Slots

| Slot | Claude Code event | Shared hooks in order |
|---|---|---|
| `session-start` | `SessionStart` | `session-tree` — where the session works, other sessions' worktrees, broken git hooks, overdue questions, leftovers |
| `session-end` | `SessionEnd` | `session-leftovers` — session leftovers to the human on exit |
| `prompt` | `UserPromptSubmit` | `knowledge-due` — open questions that are due, before `/opsx:propose` |
| `pre-bash` | `PreToolUse`, `Bash` | `guard-staging` — rejects bulk staging in the shared directory |
| `post-bash` | `PostToolUse`, `Bash` | `bash-edit` — a reminder that a file was edited by a shell command |
| `post-edit` | `PostToolUse`, `Write\|Edit`, synchronous | `rust-fmt` (rust module), `js-fmt` (frontend module) |
| `post-edit-async` | `PostToolUse`, `Write\|Edit`, `async` + `asyncRewake` | `rust-clippy` (rust module), `js-typecheck` (frontend module) |
| `subagent-stop` | `SubagentStop`, `*-developer` agents | `agent-checks` — an executor does not finish while rustfmt, clippy (rust module) and tsc (frontend module) are red for the touched packages |
| `pre-handback` | `PreToolUse`, `SubagentHandback` | `agent-checks` — the same for an executor's report in auto mode |

A module hook that the project doesn't have is skipped. After a slot's shared hooks come `.harness/local/hooks/<slot>/*.sh` in alphabetical order (byte sort, `LC_ALL=C`).

### Executor checks

Asynchronous checks don't wake a subagent on its edits: the executor follows TDD, and red between its edits is a normal step. The check sits at the exit — `agent-checks.sh`:

- **Who.** An executor is an agent named `*-developer` (`rust-developer`, `frontend-developer` and project executors of the same kind). Reviewers, built-in agents and the main session are not checked. In Claude Code the `subagent-stop` slot's `matcher` also filters: Claude Code's internal agents (input suggestions, `/btw`) also send `SubagentStop`, and without filtering the dispatcher would run on every turn.
- **What.** Files changed in the worktree relative to HEAD, plus untracked ones, mapped to crates and packages: `rustfmt --check` on changed `.rs`, clippy per crate, `tsc --noEmit` per package — the same commands and the same path resolution as `rust-clippy` and `js-typecheck` (`lib/rust-crate.sh`, `lib/js-package.sh`). Nothing changed — the hook is silent.
- **Response.** Red — a refusal with a reason: what to do and condensed check output (up to 30 lines per check, up to 90 per reason). On `SubagentStop` — `decision: "block"`: the agent continues working with the reason as its next instruction. On `SubagentHandback` — `permissionDecision: "deny"`: the report doesn't go out, the agent gets the reason.
- **Once.** `stop_hook_active` in the `SubagentStop` input — the agent is already continuing after a refusal, and the hook is silent. A `SubagentHandback` report is refused once per agent: a marker in the worktree's git directory, `harness/agent-checks/<agent_id>`. Claude Code asks the agent to retry a refused report up to three times, then finishes it without a report — so the second report goes through. Stopping an agent whose report was already checked doesn't repeat the checks.
- **Why `pre-handback`.** In auto mode a subagent hands in its report with the `SubagentHandback` tool before stopping (Claude Code 2.1.271+), and `SubagentStop` happens when the report is already with the main session. The report is what must be held. Without auto mode the report is the agent's last message, and `subagent-stop` holds it.

The template doesn't wire `Stop` (end of a main session turn): the main session's edits are checked by `post-edit-async` with rewake, before a PR — a full run, and a check on every turn would cost time. An executor running as a separate session rather than a subagent gets `Stop`, not `SubagentStop`. A reliable sign of such a session in Claude Code is `agent_type` in the input, if the session was started with `claude --agent <executor>`; an orchestrator that runs an executor as a regular session gives no such sign. A project that runs executors via `--agent` needs one line in `.claude/settings.json` or `.claude/settings.local.json`: event `Stop` → `dispatch.sh subagent-stop` — the hook picks out the executor by `agent_type` itself.

## Hook contract

**Input.** The event JSON on stdin, in Claude Code form. Hooks read from it:

- `tool_input.command` — the command (`pre-bash`, `post-bash`);
- `tool_input.file_path`, `tool_response.filePath` — the written file (`post-edit`, `post-edit-async`);
- `prompt` — the message text (`prompt`);
- `agent_id` — present only in subagent events;
- `agent_type` — the agent name: of a subagent or of a session started with `--agent`;
- `hook_event_name`, `stop_hook_active`, `tool_name`, `cwd` — which event, whether the agent is already continuing after a stop refusal, which tool, the agent's directory (`subagent-stop`, `pre-handback`).

The working directory is the session's directory. The project root is `$CLAUDE_PROJECT_DIR` if set, otherwise `git rev-parse --show-toplevel`.

**Output.**

- Exit 0 and empty stdout — the hook has nothing to say.
- Exit 0 and JSON on stdout — a response to the agent in Claude Code form: `hookSpecificOutput.additionalContext` (model context) or a refusal — `hookSpecificOutput.permissionDecision: "deny"` with the reason in `permissionDecisionReason`, `decision: "block"` with the reason in `reason` (on `subagent-stop` — the agent doesn't finish and gets the reason as its next instruction), `continue: false`.
- Exit 0 and plain text on stdout — model context on `session-start` and `prompt`; on other slots the agent shows it only in the verbose log.
- Exit 2 — a block (`pre-bash`) or a model rewake (`post-edit-async`); text for the model goes to stderr.
- Any other code — a hook error, not a refusal: stderr goes to the human, work continues.

**Guarding.** Guard hooks (`pre-bash`) protect against mistakes, not against a malicious agent: they have no full bash parser, and when the parse is in doubt (heredoc, quotes, arithmetic — `lib/bash-scan.sh`) the whole command is checked.

## Merging output

The agent parses one JSON per call, but a slot has several hooks. The dispatcher merges their output:

1. Exit 2 stops the slot: the following hooks don't run, the hook's stderr goes up, the dispatcher exits with code 2.
2. A refusal (`deny`, `block`, `continue: false`) stops the slot: exactly that JSON goes out, the context of earlier hooks is dropped.
3. `additionalContext` of all hooks is joined with a blank line into one JSON; plain text of hooks goes into the same context. `hookEventName` is taken from the hook, or if absent — from the slot. Other JSON fields are merged (for identical keys the later hook wins).
4. If no hook produced JSON, plain text is printed as is, line by line.
5. A hook's stderr with exit 0 or with an error goes up. The dispatcher's exit code on a hook error is 1 if there is no merged JSON, and 0 if there is: the agent doesn't parse JSON on a non-zero code.

## Project hooks

`.harness/local/hooks/<slot>/<name>.sh` — an executable script with the same contract. Order among your own hooks is set by name (`10-first.sh`, `20-second.sh`). A hook without the execute bit is skipped with a warning on stderr, like git hooks: removing `chmod +x` disables a hook without deleting it.

## Pi shim

In Pi the wiring is held by the `.pi/extensions/harness.ts` extension (managed). Pi loads it by itself: extensions from the project's `.pi/extensions/` are loaded at start if the project is trusted (`pi` asks once; in `pi -p` mode — the `--approve` flag). Pi is started from the project root: the `.pi/` directory is looked up only in the working directory.

The shim translates Pi events into slots, the input in Claude Code form, with `CLAUDE_PROJECT_DIR` set to the project root; tool names as in Claude Code (`bash` → `Bash`, `edit` → `Edit`, `write` → `Write`):

| Pi event | Slot | What happens to the response |
|---|---|---|
| `session_start` | `session-start` | context reaches the model as a message before the first turn |
| `session_shutdown` | `session-end` | output — to the human |
| `input` | `prompt` | refusal — the message doesn't reach the model, the reason goes to the human; context — as a message before the turn |
| `tool_call`, `bash` | `pre-bash` | refusal blocks the call, the reason goes to the model |
| `tool_result`, `bash` | `post-bash` | context is appended to the command result |
| `tool_result`, `edit`/`write` | `post-edit`, then `post-edit-async` | `post-edit` — before the model sees the result; `post-edit-async` — in the background |
| `agent_before_settle` of an executor child | `subagent-stop` | refusal or context — a message to the child's model and one continuation of the run (`continue: true`) |

Differences from Claude Code:

- **`subagent-stop`.** The counterpart of `SubagentStop` is `agent_before_settle` in a `pi-subagents` child session (Pi 0.87+): the last point where an extension can continue the run before the answer goes to the parent. It is called only for an executor (`*-developer`, like the `matcher` in Claude Code) and only for a completed, not aborted, run. The shim tracks `stop_hook_active` itself: true after continuing on a refusal, reset by a new child run.
- **`pre-handback`.** Pi has no such slot: `pi-subagents` has no `SubagentHandback` tool, the child's answer is its last message, and `subagent-stop` holds it.
- **`post-edit-async`.** The run happens in the background, the model gets the edit result right away. Exit 2 wakes the model with a message (`sendMessage` with `triggerTurn`): when idle a new turn starts, mid-work the text arrives before the next model request. Context without exit 2 waits for the next turn. In `pi -p` mode Pi exits after the last answer and doesn't wait for the background run; on session exit background runs are killed.
- **Subagents.** A `pi-subagents` child is a session of the same process with the same extension. The shim recognizes it by the `<active_agent name="…"/>` marker in the system prompt and puts `agent_id` and `agent_type` into the event — hooks stay silent on it just as in Claude Code. `session-start`, `session-end` and `prompt` are not called for a child: a Claude Code subagent has no such events.
- **Process rules.** The project's main rules file is `AGENTS.md`; `CLAUDE.md` next to it holds the imports `@AGENTS.md` and `@.harness/process.md` for Claude Code. Pi doesn't expand imports and loads one file from a directory (`AGENTS.md` before `CLAUDE.md`). The shim adds `.harness/process.md` to the session's context files right after the main file, and the main file too if Pi didn't take it. `CLAUDE.md` is not added when `AGENTS.md` exists, as long as it holds only imports; with its own rules (the project hasn't migrated yet, and `AGENTS.md` is a stub from a template update) it goes right after `AGENTS.md`. Without `AGENTS.md` the main file is `CLAUDE.md`, as before the migration.
- **Permissions.** `permissions` from `.claude/settings.json` don't apply in Pi; guarding is only the `pre-bash` hooks.

## Variables

- `HARNESS_CLIPPY` — a command instead of `cargo clippy` (tests); shared by `rust-clippy` and `agent-checks`.
- `HARNESS_CLIPPY_STATE` — the `rust-clippy` state directory (lock and error signature) instead of a temporary one.
- `HARNESS_TSC` — a command instead of `bunx tsc` (tests); shared by `js-typecheck` and `agent-checks`.

## Tests

`tests/run.sh` — each hook is checked with a firing case and a skip case, the dispatcher — by order, stopping, output merging. Everything runs on synthetic repositories in temporary directories. Run by pre-push (`.harness/lefthook.yml`) and CI (`.github/workflows/harness.yml`).

`tests/pi.test.ts` — the Pi shim: the extension's handlers are called with a fake Pi API, with the real dispatcher and hooks behind them. Run with `bun test ./.harness/hooks/tests/pi.test.ts`; run by the same pre-push and CI.
