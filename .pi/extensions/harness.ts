/**
 * Прослойка обвязки для Pi: события Pi → `.harness/hooks/dispatch.sh <слот>` со входом JSON
 * в форме Claude Code. Управляемый файл: обновляется шаблоном, руками не правится.
 *
 * Логики хуков здесь нет — она в shell-скриптах, общих с Claude Code. Прослойка только
 * переводит событие Pi в событие Claude, зовёт диспетчер и переводит его ответ обратно.
 * Имена инструментов отдаются в форме Claude (`bash` → `Bash`), чтобы хуки не знали
 * про агента. Слоты и контракт ответа — `.harness/hooks/README.md`.
 *
 * Сверх хуков прослойка доводит до модели общие правила процесса: Pi не понимает импорт
 * `@.harness/process.md` из `CLAUDE.md` и берёт файл как есть, поэтому `.harness/process.md`
 * добавляется в context files сессии рядом с `CLAUDE.md`.
 *
 * Зависимостей npm нет: только встроенные модули и типы Pi.
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
  | "post-edit-async";

type Content = NonNullable<ToolResultEventResult["content"]>;

interface SlotResult {
  /** Код выхода диспетчера; `null` — не запустился или убит по таймауту. */
  code: number | null;
  stdout: string;
  stderr: string;
}

interface Verdict {
  /** Отказ: причина для модели (pre-bash, prompt) или текст пробуждения. */
  block?: string;
  /** Контекст модели (`additionalContext` или простой текст там, где он контекст). */
  context?: string;
  /** Сообщение человеку: ошибка хука или `systemMessage`. */
  notice?: string;
}

/** Корень проекта: расширение лежит в `<корень>/.pi/extensions/`. */
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const dispatcher = join(root, ".harness", "hooks", "dispatch.sh");
const processRules = join(root, ".harness", "process.md");
const projectInstructions = join(root, "CLAUDE.md");

/** Таймауты слотов — те же, что в `.claude/settings.json`. */
const timeouts: Record<Slot, number> = {
  "session-start": 20_000,
  "session-end": 15_000,
  prompt: 10_000,
  "pre-bash": 10_000,
  "post-bash": 10_000,
  "post-edit": 60_000,
  "post-edit-async": 300_000,
};

/** На каких слотах простой текст хука — контекст модели (как у Claude Code). */
const plainIsContext: ReadonlySet<Slot> = new Set(["session-start", "prompt"]);

/** Имена инструментов Pi → Claude Code. */
const claudeToolName: Record<string, string> = {
  bash: "Bash",
  edit: "Edit",
  write: "Write",
};

/** Метка субагента `pi-subagents` в системном промпте ребёнка. */
const activeAgentTag = /<active_agent name="([^"]+)"\s*\/>/;

/** Запуск диспетчера слота: событие на stdin, сбор вывода, таймаут. Нет диспетчера — молчание. */
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
      finish({ code: null, stdout, stderr: `${stderr}\nслот ${slot}: таймаут` });
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

/** Ответ диспетчера → решение прослойки. Контракт — `.harness/hooks/README.md`, «Выход». */
function interpret(slot: Slot, result: SlotResult): Verdict {
  const stderr = result.stderr.trim();
  if (result.code === 2) {
    return { block: stderr || `хук слота ${slot} отказал без причины` };
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
      verdict.block = text(specific.permissionDecisionReason) ?? text(json.reason) ?? "отказ хука";
    } else if (json.decision === "block") {
      verdict.block = text(json.reason) ?? "отказ хука";
    } else if (json.continue === false) {
      verdict.block = text(json.stopReason) ?? "отказ хука";
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

/** Имя субагента `pi-subagents`, если сессия — его ребёнок. */
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

/** Путь инструмента Pi → абсолютный, как `file_path` у Claude Code. */
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
  /** Контекст для модели, ждущий первого запуска агента (session-start, prompt). */
  let pending: string[] = [];
  /** Контекст `pre-bash`, который приедет модели вместе с результатом команды. */
  const preContext = new Map<string, string>();
  /** Фоновые прогоны `post-edit-async`: на выходе их убивают. */
  const background = new Set<ChildProcess>();
  let closed = false;

  const base = (ctx: ExtensionContext, event: string, prompt?: string) => {
    const payload: Record<string, unknown> = {
      session_id: ctx.sessionManager.getSessionId(),
      transcript_path: ctx.sessionManager.getSessionFile() ?? "",
      cwd: ctx.cwd,
      hook_event_name: event,
    };
    // Claude Code кладёт `agent_id` только в события субагента; хуки по нему молчат.
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
   * `post-edit-async` — как `async` + `asyncRewake` у Claude Code. Прогон идёт в фоне,
   * результат правки модель получает сразу. Код 2 будит модель текстом из stderr: в простое
   * сообщение начинает новый ход, посреди работы приходит перед следующим запросом к модели.
   * Контекст без кода 2 ждёт следующего хода и не будит.
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
    // Сессия-ребёнок `pi-subagents`: у Claude Code SessionStart субагента не бывает.
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
    // Выход сессии — человеку: модели после него уже нет.
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
    const options = event.systemPromptOptions;
    const known = (path: string) =>
      options.contextFiles.some((file) => file.path === path) || event.systemPrompt.includes(path);
    // Pi грузит из каталога один context file (AGENTS.md раньше CLAUDE.md) и не понимает
    // импорт `@файл`. Правила проекта и общие правила процесса доводятся до модели здесь.
    const extra: Array<{ path: string; content: string }> = [];
    for (const path of [projectInstructions, processRules]) {
      if (!known(path) && existsSync(path)) {
        extra.push({ path, content: readFileSync(path, "utf8") });
      }
    }
    if (extra.length > 0) {
      const at = options.contextFiles.findIndex((file) => file.path === projectInstructions);
      options.contextFiles.splice(at === -1 ? options.contextFiles.length : at + 1, 0, ...extra);
    }
    // Ребёнок `pi-subagents` не забирает контекст главной сессии.
    if (activeAgent(event.systemPrompt) || pending.length === 0) return undefined;
    const content = pending.join("\n\n");
    pending = [];
    return { message: { customType: "harness-hook", content, display: true } };
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
