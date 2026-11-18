/**
 * Tests of the Pi shim `.pi/extensions/harness.ts`: event handlers are called with a stand-in
 * Pi API, and behind them the real dispatcher and project hooks run. The translation is
 * checked both ways: Pi event → hook input in Claude Code form, hook answer → Pi decision.
 *
 * Like `run.sh`, everything runs on synthetic repositories in temporary directories.
 * Module hook cases (rust-fmt, rust-clippy — module rust; js-fmt — module frontend)
 * run only if the hook exists in the project.
 *
 * Run: bun test ./.harness/hooks/tests/pi.test.ts
 */
import { afterAll, describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import harness from "../../../.pi/extensions/harness.ts";

const unset = (name: string) => {
  delete process.env[name];
};

// Under a git hook the environment has GIT_DIR and the like — see `run.sh`.
for (const name of Object.keys(process.env)) {
  if (name.startsWith("GIT_")) unset(name);
}
unset("CLAUDE_PROJECT_DIR");
// Real clippy and tsc aren't needed in the background: where a check is needed, our own is substituted.
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

// The environment is explicit: Bun (1.3) gives a child process without `env` the environment
// it started with itself, and `delete process.env` above doesn't reach git. Under
// pre-push, git with an inherited GIT_DIR wrote commits into the real repository.
function git(cwd: string, ...args: string[]) {
  execFileSync("git", args, { cwd, stdio: "ignore", env: { ...process.env } });
}

/** A temporary git repository with an empty commit on main. */
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

type Extension = typeof harness;

/** A stand-in Pi: collects handlers and sent messages. */
function fakePi(options: { cwd: string; systemPrompt?: string; extension?: Extension }) {
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
  (options.extension ?? harness)(api as unknown as Parameters<Extension>[0]);
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
  prompt: "hello",
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
  test("git add -A in the shared checkout — denied with a reason", async () => {
    const pi = fakePi({ cwd: newRepo() });
    const result = (await pi.emit("tool_call", bash("git add -A"))) as {
      block: boolean;
      reason: string;
    };
    expect(result.block).toBe(true);
    expect(result.reason).toContain("wholesale staging");
  });

  test("an ordinary command passes", async () => {
    const pi = fakePi({ cwd: newRepo() });
    expect(await pi.emit("tool_call", bash("ls"))).toBeUndefined();
  });

  test("git add -A in own worktree passes", async () => {
    const repo = newRepo();
    git(repo, "worktree", "add", "-q", join(repo, "wt"), "-b", "wt");
    const pi = fakePi({ cwd: join(repo, "wt") });
    expect(await pi.emit("tool_call", bash("git add -A"))).toBeUndefined();
  });

  test("not bash — untouched", async () => {
    const pi = fakePi({ cwd: newRepo() });
    const event = { type: "tool_call", toolName: "read", toolCallId: "r", input: { path: "a" } };
    expect(await pi.emit("tool_call", event)).toBeUndefined();
  });
});

describe("post-edit: tool_result edit/write", () => {
  test.if(has("rust-fmt.sh"))("an .rs edit is formatted, path relative to cwd", async () => {
    const dir = mkdtempSync(join(base, "fmt."));
    writeFileSync(join(dir, "a.rs"), "fn  main ( )  {\nlet x=1;\n}\n");
    const pi = fakePi({ cwd: dir });
    await pi.emit("tool_result", edited("edit", "a.rs", { edits: [] }));
    expect(readFileSync(join(dir, "a.rs"), "utf8")).toStartWith("fn main() {\n");
  });

  test.if(has("js-fmt.sh"))(
    "a .tsx write is formatted",
    async () => {
      // Inside the project: biome comes from its node_modules, rules from its biome.json.
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

  test("a failed edit doesn't call hooks", async () => {
    const dir = mkdtempSync(join(base, "err."));
    const raw = "fn  main ( )  {}\n";
    writeFileSync(join(dir, "a.rs"), raw);
    const pi = fakePi({ cwd: dir });
    await pi.emit("tool_result", { ...edited("edit", "a.rs", { edits: [] }), isError: true });
    expect(readFileSync(join(dir, "a.rs"), "utf8")).toBe(raw);
  });

  test.if(has("rust-clippy.sh"))(
    "post-edit-async: exit 2 wakes the model with a message",
    async () => {
      // The crate is inside the project: clippy doesn't check a file of a foreign repository.
      const crate = join(inProject, "crates", "core", "src");
      mkdirSync(crate, { recursive: true });
      writeFileSync(join(crate, "lib.rs"), "fn main() {}\n");
      const fake = join(base, "fake-clippy");
      writeFileSync(fake, "#!/usr/bin/env bash\necho 'error[E0061]: broken'\nexit 1\n");
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

describe("subagent-stop: agent_before_settle of an executor child", () => {
  const settle = (entries: unknown[] = []) => ({
    type: "agent_before_settle",
    entries,
    continue: false,
    outcome: "completed",
  });
  const tag = (name: string) => `<active_agent name="${name}"/>`;

  /** Changes in the project worktree and red stand-in clippy and tsc for the duration of the test. */
  async function withRedChecks(run: () => Promise<void>) {
    // Inside the project: the hook checks only its own repository. Works for all layouts:
    // a crate in crates/, a package in packages/ — monorepo; the project root — a single crate or
    // a single app.
    const crate = join(inProject, "crates", "core", "src");
    const pkg = join(inProject, "packages", "web");
    mkdirSync(crate, { recursive: true });
    mkdirSync(join(pkg, "src"), { recursive: true });
    writeFileSync(join(crate, "lib.rs"), "pub fn a() {}\n");
    writeFileSync(join(pkg, "tsconfig.json"), "{}\n");
    writeFileSync(join(pkg, "src", "index.ts"), "export const n = 1;\n");
    const fake = join(base, "fake-red-check");
    writeFileSync(fake, "#!/usr/bin/env bash\necho 'error: broken'\nexit 1\n");
    chmodSync(fake, 0o755);
    process.env.HARNESS_CLIPPY = fake;
    process.env.HARNESS_TSC = fake;
    try {
      await run();
    } finally {
      process.env.HARNESS_CLIPPY = "true";
      process.env.HARNESS_TSC = "true";
      rmSync(inProject, { recursive: true, force: true });
    }
  }

  type Settled = { entries: Array<{ type: string; content: string }>; continue: boolean };

  test("an executor with red checks — one continuation with the output", async () => {
    await withRedChecks(async () => {
      const pi = fakePi({ cwd: root, systemPrompt: tag("rust-developer") });
      const prior = { type: "custom", customType: "other" };
      const first = (await pi.emit("agent_before_settle", settle([prior]))) as Settled;
      expect(first.continue).toBe(true);
      expect(first.entries[0]).toEqual(prior);
      const [, note] = first.entries;
      expect(note?.type).toBe("custom_message");
      expect(note?.content).toContain("broken");
      // A continuation after a denial — like Claude Code's stop_hook_active: not held a second time.
      expect(await pi.emit("agent_before_settle", settle())).toBeUndefined();
      // A new run of the child — under the check again.
      await pi.emit("before_agent_start", agentStart([], tag("rust-developer")));
      const again = (await pi.emit("agent_before_settle", settle())) as Settled;
      expect(again.continue).toBe(true);
    });
  }, 30_000);

  test("not an executor and the main session — no check", async () => {
    await withRedChecks(async () => {
      for (const systemPrompt of [tag("rust-reviewer"), ""]) {
        const pi = fakePi({ cwd: root, systemPrompt });
        expect(await pi.emit("agent_before_settle", settle())).toBeUndefined();
      }
    });
  }, 30_000);

  test("an aborted run is not held", async () => {
    await withRedChecks(async () => {
      const pi = fakePi({ cwd: root, systemPrompt: tag("frontend-developer") });
      const aborted = { ...settle(), outcome: "aborted" };
      expect(await pi.emit("agent_before_settle", aborted)).toBeUndefined();
    });
  }, 30_000);
});

/**
 * A copy of the shim in a synthetic project with given files: the shim takes its root from
 * its own path, so the set of instruction files is checked without touching the real project.
 */
async function isolatedHarness(files: Record<string, string>) {
  // A path without links (on macOS the temp directory is a link into /private): the shim's root
  // is computed from the module path, and the paths in the expectations must match it.
  const dir = realpathSync(mkdtempSync(join(base, "project.")));
  const copy = join(dir, ".pi", "extensions", "harness.ts");
  mkdirSync(dirname(copy), { recursive: true });
  writeFileSync(copy, readFileSync(join(root, ".pi", "extensions", "harness.ts"), "utf8"));
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), content);
  }
  const module = (await import(copy)) as { default: Extension };
  return { dir, extension: module.default };
}

describe("session-start and process rules: before_agent_start", () => {
  // The project's main file is AGENTS.md; for a project not yet moved to it — CLAUDE.md.
  const instructions = existsSync(join(root, "AGENTS.md"))
    ? join(root, "AGENTS.md")
    : join(root, "CLAUDE.md");
  const processRules = join(root, ".harness", "process.md");

  test("session-start context arrives as a message, process.md after the main file", async () => {
    const pi = fakePi({ cwd: newRepo() });
    await pi.emit("session_start", { type: "session_start", reason: "startup" });
    const files: ContextFile[] = [{ path: instructions, content: "project rules" }];
    const result = (await pi.emit("before_agent_start", agentStart(files))) as {
      message: { content: string };
    };
    expect(result.message.content).toContain("SHARED checkout");
    // A CLAUDE.md with its own rules may stand between them: the project hasn't moved to
    // AGENTS.md yet, and the update created the stub. The set per project kind — tests below.
    const paths = files.map((file) => file.path);
    expect(paths[0]).toBe(instructions);
    expect(paths.at(-1)).toBe(processRules);
    expect(files.at(-1)?.content).toBe(readFileSync(processRules, "utf8"));

    // Second turn: the context is already delivered, nothing is duplicated.
    expect(await pi.emit("before_agent_start", agentStart(files))).toBeUndefined();
    expect(files.map((file) => file.path)).toEqual(paths);
  });

  test("AGENTS.md and CLAUDE.md of imports — process.md is added, CLAUDE.md isn't", async () => {
    const { dir, extension } = await isolatedHarness({
      "AGENTS.md": "# project rules\n",
      "CLAUDE.md": "@AGENTS.md\n@.harness/process.md\n",
      ".harness/process.md": "# process rules\n",
    });
    const agents = join(dir, "AGENTS.md");
    const rules = join(dir, ".harness", "process.md");
    const pi = fakePi({ cwd: dir, extension });

    // Pi took AGENTS.md from the directory and doesn't load the CLAUDE.md next to it.
    const files: ContextFile[] = [{ path: agents, content: "# project rules\n" }];
    await pi.emit("before_agent_start", agentStart(files));
    expect(files.map((file) => file.path)).toEqual([agents, rules]);
    expect(files[1]?.content).toBe("# process rules\n");

    // Pi took nothing — the shim brings in AGENTS.md, not CLAUDE.md.
    const bare: ContextFile[] = [];
    await pi.emit("before_agent_start", agentStart(bare));
    expect(bare.map((file) => file.path)).toEqual([agents, rules]);
    expect(bare[0]?.content).toBe("# project rules\n");
  });

  test("an AGENTS.md stub and a CLAUDE.md with its own rules — both are added", async () => {
    // A template update creates the starter AGENTS.md even in a project that still keeps
    // its rules in CLAUDE.md: Pi takes the stub, and the project rules must not get lost.
    const { dir, extension } = await isolatedHarness({
      "AGENTS.md": "# stub\n",
      "CLAUDE.md": "# project rules\n\n@.harness/process.md\n",
      ".harness/process.md": "# process rules\n",
    });
    const agents = join(dir, "AGENTS.md");
    const claude = join(dir, "CLAUDE.md");
    const rules = join(dir, ".harness", "process.md");
    const pi = fakePi({ cwd: dir, extension });

    const files: ContextFile[] = [{ path: agents, content: "# stub\n" }];
    await pi.emit("before_agent_start", agentStart(files));
    expect(files.map((file) => file.path)).toEqual([agents, claude, rules]);
  });

  test("only CLAUDE.md (a project before AGENTS.md) — it and process.md, as before", async () => {
    const { dir, extension } = await isolatedHarness({
      "CLAUDE.md": "# project rules\n\n@.harness/process.md\n",
      ".harness/process.md": "# process rules\n",
    });
    const claude = join(dir, "CLAUDE.md");
    const rules = join(dir, ".harness", "process.md");
    const pi = fakePi({ cwd: dir, extension });

    const files: ContextFile[] = [{ path: claude, content: "# project rules\n" }];
    await pi.emit("before_agent_start", agentStart(files));
    expect(files.map((file) => file.path)).toEqual([claude, rules]);

    const bare: ContextFile[] = [];
    await pi.emit("before_agent_start", agentStart(bare));
    expect(bare.map((file) => file.path)).toEqual([claude, rules]);
  });

  test("pi-subagents subagent: session-start is silent, doesn't take the context", async () => {
    const tag = '<active_agent name="rust-reviewer"/>';
    const pi = fakePi({ cwd: newRepo(), systemPrompt: tag });
    await pi.emit("session_start", { type: "session_start", reason: "startup" });
    const files: ContextFile[] = [{ path: instructions, content: "" }];
    expect(await pi.emit("before_agent_start", agentStart(files, tag))).toBeUndefined();
  });

  test("session exit passes", async () => {
    const pi = fakePi({ cwd: newRepo() });
    await pi.emit("session_shutdown", { type: "session_shutdown", reason: "quit" });
    expect(pi.sent).toEqual([]);
  });
});
