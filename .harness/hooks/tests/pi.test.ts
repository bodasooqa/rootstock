/**
 * Тесты прослойки Pi `.pi/extensions/harness.ts`: обработчики событий зовутся с подставным
 * API Pi, а за ними работают настоящие диспетчер и хуки проекта. Проверяется перевод
 * в обе стороны: событие Pi → вход хука в форме Claude Code, ответ хука → решение Pi.
 *
 * Как и `run.sh`, всё гоняется на синтетических репозиториях во временных каталогах.
 * Случаи модульных хуков (rust-fmt, rust-clippy — модуль rust; js-fmt — модуль frontend)
 * идут, только если хук в проекте есть.
 *
 * Запуск: bun test ./.harness/hooks/tests/pi.test.ts
 */
import { afterAll, describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import harness from "../../../.pi/extensions/harness.ts";

const unset = (name: string) => {
  delete process.env[name];
};

// Под git-хуком в окружении стоят GIT_DIR и прочие — см. `run.sh`.
for (const name of Object.keys(process.env)) {
  if (name.startsWith("GIT_")) unset(name);
}
unset("CLAUDE_PROJECT_DIR");
// Настоящие clippy и tsc в фоне не нужны: где проверка нужна, подставляется своя.
process.env.HARNESS_CLIPPY = "true";
process.env.HARNESS_TSC = "true";

const root = resolve(import.meta.dir, "..", "..", "..");
const hooks = join(root, ".harness", "hooks");
const has = (hook: string) => existsSync(join(hooks, hook));

const base = mkdtempSync(join(tmpdir(), "harness-pi-test."));
const inProject = join(root, ".tmp-pi-test");

afterAll(() => {
  rmSync(base, { recursive: true, force: true });
  rmSync(inProject, { recursive: true, force: true });
});

// Окружение — явно: Bun (1.3) отдаёт дочернему процессу без `env` окружение,
// с которым стартовал сам, и `delete process.env` выше до git не доходит. Под
// pre-push git с унаследованным GIT_DIR писал коммиты в настоящий репозиторий.
function git(cwd: string, ...args: string[]) {
  execFileSync("git", args, { cwd, stdio: "ignore", env: { ...process.env } });
}

/** Временный git-репозиторий с пустым коммитом на main. */
function newRepo(): string {
  const dir = mkdtempSync(join(base, "repo."));
  git(dir, "init", "-q", "-b", "main");
  git(
    dir,
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@t",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "init",
  );
  return dir;
}

type Handler = (event: unknown, ctx: unknown) => unknown;

interface Sent {
  message: { customType: string; content: unknown; display: boolean };
  options: { triggerTurn?: boolean; deliverAs?: string } | undefined;
}

interface ContextFile {
  path: string;
  content: string;
}

/** Подставной Pi: собирает обработчики и отправленные сообщения. */
function fakePi(options: { cwd: string; systemPrompt?: string }) {
  const handlers = new Map<string, Handler[]>();
  const sent: Sent[] = [];
  const api = {
    on(name: string, handler: Handler) {
      handlers.set(name, [...(handlers.get(name) ?? []), handler]);
    },
    sendMessage(message: Sent["message"], sendOptions?: Sent["options"]) {
      sent.push({ message, options: sendOptions });
    },
  };
  const ctx = {
    cwd: options.cwd,
    hasUI: false,
    ui: { notify() {} },
    sessionManager: {
      getSessionId: () => "pi-test",
      getSessionFile: () => undefined,
    },
    getSystemPrompt: () => options.systemPrompt ?? "",
  };
  harness(api as unknown as Parameters<typeof harness>[0]);
  const emit = async (name: string, event: unknown) => {
    let result: unknown;
    for (const handler of handlers.get(name) ?? []) result = await handler(event, ctx);
    return result;
  };
  return { emit, sent };
}

const bash = (command: string) => ({
  type: "tool_call",
  toolName: "bash",
  toolCallId: `call-${command}`,
  input: { command },
});

const edited = (toolName: "edit" | "write", path: string, input: Record<string, unknown>) => ({
  type: "tool_result",
  toolName,
  toolCallId: `call-${path}`,
  input: { path, ...input },
  content: [{ type: "text", text: "ok" }],
  isError: false,
});

const agentStart = (contextFiles: ContextFile[], systemPrompt = "") => ({
  type: "before_agent_start",
  prompt: "привет",
  systemPrompt,
  systemPromptOptions: { cwd: root, contextFiles, skills: [] },
});

async function waitFor(check: () => boolean, ms: number): Promise<boolean> {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    if (check()) return true;
    await Bun.sleep(50);
  }
  return check();
}

describe("pre-bash: tool_call bash", () => {
  test("git add -A в общем каталоге — отказ с причиной", async () => {
    const pi = fakePi({ cwd: newRepo() });
    const result = (await pi.emit("tool_call", bash("git add -A"))) as {
      block: boolean;
      reason: string;
    };
    expect(result.block).toBe(true);
    expect(result.reason).toContain("валовая постановка");
  });

  test("обычная команда проходит", async () => {
    const pi = fakePi({ cwd: newRepo() });
    expect(await pi.emit("tool_call", bash("ls"))).toBeUndefined();
  });

  test("git add -A в своём дереве проходит", async () => {
    const repo = newRepo();
    git(repo, "worktree", "add", "-q", join(repo, "wt"), "-b", "wt");
    const pi = fakePi({ cwd: join(repo, "wt") });
    expect(await pi.emit("tool_call", bash("git add -A"))).toBeUndefined();
  });

  test("не bash — не трогает", async () => {
    const pi = fakePi({ cwd: newRepo() });
    const event = { type: "tool_call", toolName: "read", toolCallId: "r", input: { path: "a" } };
    expect(await pi.emit("tool_call", event)).toBeUndefined();
  });
});

describe("post-edit: tool_result edit/write", () => {
  test.if(has("rust-fmt.sh"))("правка .rs форматируется, путь от cwd", async () => {
    const dir = mkdtempSync(join(base, "fmt."));
    writeFileSync(join(dir, "a.rs"), "fn  main ( )  {\nlet x=1;\n}\n");
    const pi = fakePi({ cwd: dir });
    await pi.emit("tool_result", edited("edit", "a.rs", { edits: [] }));
    expect(readFileSync(join(dir, "a.rs"), "utf8")).toStartWith("fn main() {\n");
  });

  test.if(has("js-fmt.sh"))(
    "запись .tsx форматируется",
    async () => {
      // Внутри проекта: biome берётся из его node_modules, правила — из его biome.json.
      mkdirSync(inProject, { recursive: true });
      const file = join(inProject, "a.tsx");
      const content = "const  x  =   1;\nexport  {x}\n";
      writeFileSync(file, content);
      const pi = fakePi({ cwd: root });
      await pi.emit("tool_result", edited("write", file, { content }));
      expect(readFileSync(file, "utf8")).toStartWith("const x = 1;\n");
    },
    30_000,
  );

  test("правка с ошибкой хуки не зовёт", async () => {
    const dir = mkdtempSync(join(base, "err."));
    const raw = "fn  main ( )  {}\n";
    writeFileSync(join(dir, "a.rs"), raw);
    const pi = fakePi({ cwd: dir });
    await pi.emit("tool_result", { ...edited("edit", "a.rs", { edits: [] }), isError: true });
    expect(readFileSync(join(dir, "a.rs"), "utf8")).toBe(raw);
  });

  test.if(has("rust-clippy.sh"))(
    "post-edit-async: код 2 будит модель сообщением",
    async () => {
      // Крейт — внутри проекта: файл чужого репозитория clippy не проверяет.
      const crate = join(inProject, "crates", "core", "src");
      mkdirSync(crate, { recursive: true });
      writeFileSync(join(crate, "lib.rs"), "fn main() {}\n");
      const fake = join(base, "fake-clippy");
      writeFileSync(fake, "#!/usr/bin/env bash\necho 'error[E0061]: сломано'\nexit 1\n");
      chmodSync(fake, 0o755);
      process.env.HARNESS_CLIPPY = fake;
      process.env.HARNESS_CLIPPY_STATE = mkdtempSync(join(base, "clippy-state."));
      try {
        const pi = fakePi({ cwd: root });
        await pi.emit(
          "tool_result",
          edited("edit", ".tmp-pi-test/crates/core/src/lib.rs", { edits: [] }),
        );
        expect(await waitFor(() => pi.sent.length > 0, 20_000)).toBe(true);
        const [wake] = pi.sent;
        expect(String(wake?.message.content)).toContain("E0061");
        expect(wake?.options).toEqual({ triggerTurn: true, deliverAs: "steer" });
      } finally {
        process.env.HARNESS_CLIPPY = "true";
        unset("HARNESS_CLIPPY_STATE");
      }
    },
    30_000,
  );
});

describe("session-start и правила процесса: before_agent_start", () => {
  const instructions = join(root, "CLAUDE.md");
  const processRules = join(root, ".harness", "process.md");

  test("контекст session-start приходит сообщением, process.md — после CLAUDE.md", async () => {
    const pi = fakePi({ cwd: newRepo() });
    await pi.emit("session_start", { type: "session_start", reason: "startup" });
    const files: ContextFile[] = [{ path: instructions, content: "правила проекта" }];
    const result = (await pi.emit("before_agent_start", agentStart(files))) as {
      message: { content: string };
    };
    expect(result.message.content).toContain("ОБЩЕМ каталоге");
    expect(files.map((file) => file.path)).toEqual([instructions, processRules]);
    expect(files[1]?.content).toBe(readFileSync(processRules, "utf8"));

    // Второй ход: контекст уже доставлен, process.md не дублируется.
    expect(await pi.emit("before_agent_start", agentStart(files))).toBeUndefined();
    expect(files.map((file) => file.path)).toEqual([instructions, processRules]);
  });

  test("AGENTS.md затенил CLAUDE.md — оба возвращаются", async () => {
    const pi = fakePi({ cwd: newRepo() });
    const agents = join(root, "AGENTS.md");
    const files: ContextFile[] = [{ path: agents, content: "чужие правила" }];
    await pi.emit("before_agent_start", agentStart(files));
    expect(files.map((file) => file.path)).toEqual([agents, instructions, processRules]);
  });

  test("субагент pi-subagents: session-start молчит, контекст не забирает", async () => {
    const tag = '<active_agent name="rust-reviewer"/>';
    const pi = fakePi({ cwd: newRepo(), systemPrompt: tag });
    await pi.emit("session_start", { type: "session_start", reason: "startup" });
    const files: ContextFile[] = [{ path: instructions, content: "" }];
    expect(await pi.emit("before_agent_start", agentStart(files, tag))).toBeUndefined();
  });

  test("выход сессии проходит", async () => {
    const pi = fakePi({ cwd: newRepo() });
    await pi.emit("session_shutdown", { type: "session_shutdown", reason: "quit" });
    expect(pi.sent).toEqual([]);
  });
});
