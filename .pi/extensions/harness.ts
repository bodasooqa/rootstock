/**
 * The harness shim for Pi: Pi events → `.harness/hooks/dispatch.sh <slot>` with JSON input
 * in Claude Code form. A managed file: updated by the template, not edited by hand.
 *
 * There is no hook logic here — it lives in shell scripts shared with Claude Code. The shim only
 * translates a Pi event into a Claude event, calls the dispatcher and translates its answer back.
 * Tool names are given in Claude form (`bash` → `Bash`) so the hooks don't know about
 * the agent. Slots and the response contract — `.harness/hooks/README.md`.
 *
 * Beyond hooks, the shim brings the shared process rules to the model. The project's main rules
 * file is `AGENTS.md`; `CLAUDE.md` next to it holds imports for Claude Code (`@AGENTS.md`,
 * `@.harness/process.md`). Pi doesn't understand `@file` imports and loads one file per directory,
 * `AGENTS.md` before `CLAUDE.md`, so the shim adds `.harness/process.md` to the session's context
 * files right after the main file, and the main file too if Pi didn't take it. For a project
 * not yet moved to `AGENTS.md`, the main file is `CLAUDE.md`, as before; a `CLAUDE.md`
 * with its own rules next to `AGENTS.md` is brought in too (`projectInstructions`).
 *
 * No npm dependencies: only built-in modules and Pi types.
 */
import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type {
  ExtensionAPI,
  ExtensionContext,
  SessionShutdownEvent,
  SessionStartEvent,
  ToolResultEventResult,
} from "@earendil-works/pi-coding-agent";

type Slot =
  | "session-start"
  | "session-end"
  | "prompt"
  | "pre-bash"
  | "post-bash"
  | "post-edit"
  | "post-edit-async"
  | "subagent-stop";

type Content = NonNullable<ToolResultEventResult["content"]>;

interface SlotResult {
  /** The dispatcher's exit code; `null` — didn't start or was killed on timeout. */
  code: number | null;
  stdout: string;
  stderr: string;
}

interface Verdict {
  /** Denial: the reason for the model (pre-bash, prompt) or the wake-up text. */
  block?: string;
  /** Model context (`additionalContext` or plain text where it counts as context). */
  context?: string;
  /** Message to the human: a hook error or `systemMessage`. */
  notice?: string;
}

/** Project root: the extension lives in `<root>/.pi/extensions/`. */
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const dispatcher = join(root, ".harness", "hooks", "dispatch.sh");
const processRules = join(root, ".harness", "process.md");
const agentsInstructions = join(root, "AGENTS.md");
const claudeInstructions = join(root, "CLAUDE.md");

/** The text has only `@file` imports and HTML comments — no rules of its own. */
function importsOnly(text: string): boolean {
  return text
    .replace(/<!--[\s\S]*?-->/g, "")
    .split("\n")
    .every((line) => line.trim() === "" || line.trim().startsWith("@"));
}

/**
 * The project's rules files in order, the first is the main one: `AGENTS.md`, or without it `CLAUDE.md`.
 * With `AGENTS.md`, the `CLAUDE.md` file holds imports for Claude Code and isn't needed in Pi. But if
 * it has rules of its own, it follows: a template update creates the starter `AGENTS.md`
 * as a stub even in a project that still keeps its rules in `CLAUDE.md`. Checked on every
 * turn: a project that moved to `AGENTS.md` mid-session doesn't keep the old choice.
 */
function projectInstructions(): string[] {
  if (!existsSync(agentsInstructions)) return [claudeInstructions];
  if (existsSync(claudeInstructions) && !importsOnly(readFileSync(claudeInstructions, "utf8"))) {
    return [agentsInstructions, claudeInstructions];
  }
  return [agentsInstructions];
}

/** Slot timeouts — the same as in `.claude/settings.json`. */
const timeouts: Record<Slot, number> = {
  "session-start": 20_000,
  "session-end": 15_000,
  prompt: 10_000,
  "pre-bash": 10_000,
  "post-bash": 10_000,
  "post-edit": 60_000,
  "post-edit-async": 300_000,
  "subagent-stop": 600_000,
};

/** Slots where a hook's plain text is model context (as in Claude Code). */
const plainIsContext: ReadonlySet<Slot> = new Set(["session-start", "prompt"]);

/** Tool names Pi → Claude Code. */
const claudeToolName: Record<string, string> = {
  bash: "Bash",
  edit: "Edit",
  write: "Write",
};

/** The `pi-subagents` subagent tag in the child's system prompt. */
const activeAgentTag = /<active_agent name="([^"]+)"\s*\/>/;

/**
 * An executor is a `*-developer` agent. The same selection as the `matcher` of the `subagent-stop`
 * slot in `.claude/settings.json`: a project hook of the slot sees the same executors in both agents.
 */
const executorAgent = /-developer$/;

/** Runs the slot's dispatcher: event on stdin, output collection, timeout. No dispatcher — silence. */
function runSlot(
  slot: Slot,
  payload: Record<string, unknown>,
  cwd: string,
  track?: (child: ChildProcess) => void,
): Promise<SlotResult> {
  if (!existsSync(dispatcher)) {
    return Promise.resolve({ code: 0, stdout: "", stderr: "" });
  }
  return new Promise((done) => {
    let stdout = "";
    let stderr = "";
    let settled = false;
    const finish = (result: SlotResult) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      done(result);
    };
    const child = spawn(dispatcher, [slot], {
      cwd,
      env: { ...process.env, CLAUDE_PROJECT_DIR: root },
      stdio: ["pipe", "pipe", "pipe"],
    });
    track?.(child);
    const timer = setTimeout(() => {
      child.kill("SIGTERM");
      finish({ code: null, stdout, stderr: `${stderr}\nslot ${slot}: timeout` });
    }, timeouts[slot]);
    child.stdout?.on("data", (chunk: Buffer) => {
      stdout += chunk.toString("utf8");
    });
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    child.on("error", (error) => finish({ code: null, stdout, stderr: String(error) }));
    child.on("close", (code) => finish({ code, stdout, stderr }));
    child.stdin?.on("error", () => {});
    child.stdin?.end(JSON.stringify(payload));
  });
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : undefined;
}

/** The dispatcher's answer → the shim's decision. Contract — `.harness/hooks/README.md`, "Output". */
function interpret(slot: Slot, result: SlotResult): Verdict {
  const stderr = result.stderr.trim();
  if (result.code === 2) {
    return { block: stderr || `the ${slot} slot hook denied without a reason` };
  }
  const verdict: Verdict = {};
  const out = result.stdout.trim();
  let json: Record<string, unknown> | undefined;
  if (out !== "") {
    try {
      json = record(JSON.parse(out));
    } catch {
      json = undefined;
    }
  }
  if (json) {
    const specific = record(json.hookSpecificOutput);
    if (specific?.permissionDecision === "deny") {
      verdict.block = text(specific.permissionDecisionReason) ?? text(json.reason) ?? "hook denial";
    } else if (json.decision === "block") {
      verdict.block = text(json.reason) ?? "hook denial";
    } else if (json.continue === false) {
      verdict.block = text(json.stopReason) ?? "hook denial";
    }
    verdict.context = text(specific?.additionalContext);
    verdict.notice = text(json.systemMessage);
  } else if (out !== "" && plainIsContext.has(slot)) {
    verdict.context = out;
  }
  if (result.code !== 0 && stderr !== "") {
    verdict.notice = verdict.notice ? `${verdict.notice}\n${stderr}` : stderr;
  }
  return verdict;
}

function notify(ctx: ExtensionContext, message: string, level: "info" | "warning" | "error") {
  if (ctx.hasUI) {
    ctx.ui.notify(message, level);
  } else {
    process.stderr.write(`harness: ${message}\n`);
  }
}

/** The `pi-subagents` subagent name, if the session is its child. */
function activeAgent(prompt: string | undefined): string | undefined {
  return prompt ? activeAgentTag.exec(prompt)?.[1] : undefined;
}

function systemPrompt(ctx: ExtensionContext): string | undefined {
  try {
    return ctx.getSystemPrompt();
  } catch {
    return undefined;
  }
}

/** A Pi tool path → absolute, like Claude Code's `file_path`. */
function absolutePath(path: string, cwd: string): string {
  const bare = path.startsWith("@") ? path.slice(1) : path;
  if (bare === "~" || bare.startsWith("~/")) return join(homedir(), bare.slice(1));
  return isAbsolute(bare) ? bare : resolve(cwd, bare);
}

function stringField(input: object, key: string): string | undefined {
  const value = (input as Record<string, unknown>)[key];
  return typeof value === "string" ? value : undefined;
}

function sessionSource(reason: SessionStartEvent["reason"]): string {
  switch (reason) {
    case "new":
      return "clear";
    case "resume":
    case "fork":
      return "resume";
    default:
      return "startup";
  }
}

function sessionEndReason(reason: SessionShutdownEvent["reason"]): string {
  switch (reason) {
    case "quit":
      return "prompt_input_exit";
    case "new":
      return "clear";
    default:
      return "other";
  }
}

export default function harness(pi: ExtensionAPI) {
  /** Context for the model waiting for the agent's first run (session-start, prompt). */
  let pending: string[] = [];
  /** `pre-bash` context that will reach the model together with the command result. */
  const preContext = new Map<string, string>();
  /** Background `post-edit-async` runs: they are killed on exit. */
  const background = new Set<ChildProcess>();
  let closed = false;
  /**
   * The run continues because of a `subagent-stop` denial — like Claude Code's `stop_hook_active`:
   * the same run isn't held a second time. Reset by a new run (`before_agent_start`;
   * a continuation after a denial doesn't call it).
   */
  let stopHookActive = false;

  const base = (ctx: ExtensionContext, event: string, prompt?: string) => {
    const payload: Record<string, unknown> = {
      session_id: ctx.sessionManager.getSessionId(),
      transcript_path: ctx.sessionManager.getSessionFile() ?? "",
      cwd: ctx.cwd,
      hook_event_name: event,
    };
    // Claude Code puts `agent_id` only into subagent events; hooks stay silent on it.
    const agent = activeAgent(prompt ?? systemPrompt(ctx));
    if (agent) {
      payload.agent_id = agent;
      payload.agent_type = agent;
    }
    return payload;
  };

  const report = (ctx: ExtensionContext, verdict: Verdict) => {
    if (verdict.notice) notify(ctx, verdict.notice, "warning");
  };

  /**
   * `post-edit-async` — like Claude Code's `async` + `asyncRewake`. The run goes in the background,
   * the model gets the edit result immediately. Exit 2 wakes the model with the stderr text: when idle
   * the message starts a new turn, mid-work it arrives before the next request to the model.
   * Context without exit 2 waits for the next turn and doesn't wake.
   */
  const startAsync = (payload: Record<string, unknown>, ctx: ExtensionContext) => {
    let child: ChildProcess | undefined;
    void runSlot("post-edit-async", payload, ctx.cwd, (spawned) => {
      child = spawned;
      background.add(spawned);
    }).then((result) => {
      if (child) background.delete(child);
      if (closed) return;
      const verdict = interpret("post-edit-async", result);
      report(ctx, verdict);
      if (verdict.block) {
        pi.sendMessage(
          { customType: "harness-hook", content: verdict.block, display: true },
          { triggerTurn: true, deliverAs: "steer" },
        );
      } else if (verdict.context) {
        pi.sendMessage(
          { customType: "harness-hook", content: verdict.context, display: true },
          { deliverAs: "nextTurn" },
        );
      }
    });
  };

  pi.on("session_start", async (event, ctx) => {
    closed = false;
    // A `pi-subagents` child session: Claude Code has no subagent SessionStart.
    if (activeAgent(systemPrompt(ctx))) return;
    const payload = { ...base(ctx, "SessionStart"), source: sessionSource(event.reason) };
    const verdict = interpret("session-start", await runSlot("session-start", payload, ctx.cwd));
    report(ctx, verdict);
    const context = verdict.context ?? verdict.block;
    if (context) pending.push(context);
  });

  pi.on("session_shutdown", async (event, ctx) => {
    closed = true;
    for (const child of background) child.kill("SIGTERM");
    background.clear();
    if (activeAgent(systemPrompt(ctx))) return;
    const payload = { ...base(ctx, "SessionEnd"), reason: sessionEndReason(event.reason) };
    const result = await runSlot("session-end", payload, ctx.cwd);
    // Session exit goes to the human: there is no model after it.
    const out = [result.stdout.trim(), result.code === 0 ? "" : result.stderr.trim()]
      .filter((part) => part !== "")
      .join("\n");
    if (out !== "") notify(ctx, out, "info");
  });

  pi.on("input", async (event, ctx) => {
    if (activeAgent(systemPrompt(ctx))) return { action: "continue" };
    const payload = { ...base(ctx, "UserPromptSubmit"), prompt: event.text };
    const verdict = interpret("prompt", await runSlot("prompt", payload, ctx.cwd));
    report(ctx, verdict);
    if (verdict.block) {
      notify(ctx, verdict.block, "warning");
      return { action: "handled" };
    }
    if (verdict.context) pending.push(verdict.context);
    return { action: "continue" };
  });

  pi.on("before_agent_start", (event) => {
    stopHookActive = false;
    const options = event.systemPromptOptions;
    const known = (path: string) =>
      options.contextFiles.some((file) => file.path === path) || event.systemPrompt.includes(path);
    // Pi loads one context file per directory (AGENTS.md before CLAUDE.md) and doesn't understand
    // `@file` imports. The project rules and the shared process rules reach the model here.
    const instructions = projectInstructions();
    const extra: Array<{ path: string; content: string }> = [];
    for (const path of [...instructions, processRules]) {
      if (!known(path) && existsSync(path)) {
        extra.push({ path, content: readFileSync(path, "utf8") });
      }
    }
    if (extra.length > 0) {
      // Right after the last project rules file Pi has already taken.
      const at = Math.max(
        ...instructions.map((path) => options.contextFiles.findIndex((file) => file.path === path)),
      );
      options.contextFiles.splice(at === -1 ? options.contextFiles.length : at + 1, 0, ...extra);
    }
    // A `pi-subagents` child doesn't take the main session's context.
    if (activeAgent(event.systemPrompt) || pending.length === 0) return undefined;
    const content = pending.join("\n\n");
    pending = [];
    return { message: { customType: "harness-hook", content, display: true } };
  });

  /**
   * `subagent-stop` — like Claude Code's `SubagentStop`. A `pi-subagents` child is a session of the
   * same process with this same extension, and `agent_before_settle` in it is the last point before
   * the child's run ends and the answer goes to the parent. A slot denial is a message to the
   * child's model and one continuation of the run. In the main session the slot isn't called: the
   * template doesn't wire `Stop` in Claude Code either.
   */
  pi.on("agent_before_settle", async (event, ctx) => {
    if (event.outcome !== "completed") return undefined;
    const agent = activeAgent(systemPrompt(ctx));
    if (!agent || !executorAgent.test(agent)) return undefined;
    const payload = { ...base(ctx, "SubagentStop"), stop_hook_active: stopHookActive };
    const verdict = interpret("subagent-stop", await runSlot("subagent-stop", payload, ctx.cwd));
    report(ctx, verdict);
    // In Claude Code both a denial and context on this event continue the agent's work.
    const content = verdict.block ?? verdict.context;
    if (!content) return undefined;
    stopHookActive = true;
    return {
      entries: [
        ...event.entries,
        { type: "custom_message" as const, customType: "harness-hook", content, display: true },
      ],
      continue: true,
    };
  });

  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "bash") return undefined;
    const command = stringField(event.input, "command") ?? "";
    const payload = {
      ...base(ctx, "PreToolUse"),
      tool_name: "Bash",
      tool_input: { command },
      tool_use_id: event.toolCallId,
    };
    const verdict = interpret("pre-bash", await runSlot("pre-bash", payload, ctx.cwd));
    report(ctx, verdict);
    if (verdict.block) return { block: true, reason: verdict.block };
    if (verdict.context) preContext.set(event.toolCallId, verdict.context);
    return undefined;
  });

  pi.on("tool_result", async (event, ctx) => {
    const notes: string[] = [];
    const pre = preContext.get(event.toolCallId);
    if (pre) {
      notes.push(pre);
      preContext.delete(event.toolCallId);
    }

    const tool = claudeToolName[event.toolName];
    const output = event.content
      .map((part) => (part.type === "text" ? part.text : ""))
      .join("")
      .trim();

    if (tool === "Bash") {
      const payload = {
        ...base(ctx, "PostToolUse"),
        tool_name: tool,
        tool_input: { command: stringField(event.input, "command") ?? "" },
        tool_response: { stdout: output, stderr: "", interrupted: false, isImage: false },
        tool_use_id: event.toolCallId,
      };
      const verdict = interpret("post-bash", await runSlot("post-bash", payload, ctx.cwd));
      report(ctx, verdict);
      for (const note of [verdict.block, verdict.context]) if (note) notes.push(note);
    } else if ((tool === "Edit" || tool === "Write") && !event.isError) {
      const path = stringField(event.input, "path");
      if (path) {
        const filePath = absolutePath(path, ctx.cwd);
        const toolInput: Record<string, unknown> = { file_path: filePath };
        if (tool === "Write") toolInput.content = stringField(event.input, "content") ?? "";
        else toolInput.edits = (event.input as Record<string, unknown>).edits ?? [];
        const payload = {
          ...base(ctx, "PostToolUse"),
          tool_name: tool,
          tool_input: toolInput,
          tool_response: { filePath, success: true },
          tool_use_id: event.toolCallId,
        };
        const verdict = interpret("post-edit", await runSlot("post-edit", payload, ctx.cwd));
        report(ctx, verdict);
        for (const note of [verdict.block, verdict.context]) if (note) notes.push(note);
        startAsync(payload, ctx);
      }
    }

    if (notes.length === 0) return undefined;
    const content: Content = [...event.content, { type: "text", text: notes.join("\n\n") }];
    return { content, structuredContent: event.structuredContent };
  });
}
