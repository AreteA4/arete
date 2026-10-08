import type { WorkspaceDiff } from './workspace.js';
import type { FrictionReport } from './friction.js';

export type HarnessKind = 'claude-code' | 'codex' | 'opencode';

export type Effort = 'low' | 'medium' | 'high' | 'xhigh' | 'max';

/** How the run's Arete agent credential is provided. */
export type KeyMode =
  /** Write a pooled `a4_ak_*` key into the `agent` profile before the agent starts. */
  | 'pool'
  /** Let `a4 auth signup --if-missing` create a fresh trial agent (5/hour/IP). */
  | 'signup';

export interface SandboxSettings {
  /**
   * Vercel Sandbox image, e.g. `vercel/sandbox/universal` (Ubuntu). Legacy
   * runtimes such as `node24` (Amazon Linux 2023) can be selected with
   * `runtime` instead.
   */
  image?: string;
  runtime?: string;
  vcpus: number;
  timeoutMinutes: number;
}

export interface RunConfig {
  harness: HarnessKind;
  /** AI Gateway model id, e.g. `anthropic/claude-sonnet-5.5`. */
  model: string;
  /** Task file in `tasks/`, e.g. `ore-live-round.ts`. */
  task: string;
  /** Optional label used in run ids and comparison tables. */
  label?: string;
  effort?: Effort;
  /** Pinned `a4` release installed in the sandbox. */
  a4Version: string;
  /** Pinned `AreteA4/skills` ref passed to `a4 init --skills-ref`. */
  skillsRef?: string;
  keyMode: KeyMode;
  /**
   * `ai-gateway`: one Vercel AI Gateway key for every harness.
   * `direct`: provider keys (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) with
   * provider-native model ids. Configs may say `auto`, which picks the
   * gateway when its credentials are set.
   */
  modelAuth: 'ai-gateway' | 'direct';
  /** Extra instructions appended after the benchmark's unattended-run note. */
  instructions?: string;
  turnTimeoutMinutes: number;
  sandbox: SandboxSettings;
}

/** Commands available to setup hooks and verifiers. */
export interface CommandResult {
  command: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
}

export interface SandboxShell {
  /** Run a bash command in the project directory. */
  run(command: string, opts?: { timeoutSeconds?: number }): Promise<CommandResult>;
  readText(path: string): Promise<string | null>;
  readBinary(path: string): Promise<Uint8Array | null>;
  /** Write a file without the content passing through a logged command. */
  writeText(path: string, content: string): Promise<void>;
  workDir: string;
  homeDir: string;
}

export interface CheckResult {
  id: string;
  passed: boolean;
  detail: string;
  /** Required checks gate `verification.passed`; optional ones only score. */
  required: boolean;
}

export interface VerifyContext {
  shell: SandboxShell;
  config: RunConfig;
  transcript: Transcript;
  /** Concatenated final assistant text of every turn. */
  finalText: string;
}

export interface TaskTurn {
  prompt: string;
  /**
   * Start this turn in a new harness session in the same sandbox, simulating
   * the user restarting their agent host so new skills and MCP config load.
   */
  freshSession?: boolean;
}

/** Typical cost profile of one run, used to estimate a sweep before it starts. */
export interface TaskEstimate {
  noCacheTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  wallMs: number;
  /** Where the numbers came from. */
  source: string;
}

export interface TaskDefinition {
  name: string;
  description: string;
  estimate?: TaskEstimate;
  track: 'onboarding' | 'build' | 'discovery';
  /**
   * `bare`: empty project; the agent installs and configures Arete itself.
   * `initialized`: `a4` installed, `a4 init` run for the harness, agent key
   * provisioned and `doctor` checked before the agent starts.
   */
  setup: 'bare' | 'initialized';
  turns: TaskTurn[];
  verify(ctx: VerifyContext): Promise<CheckResult[]>;
}

// ---------------------------------------------------------------------------
// Transcript

export interface ToolCallRecord {
  id: string;
  turn: number;
  step: number;
  name: string;
  input: unknown;
  /** Output as returned by the harness (may be large). */
  output?: unknown;
  isError: boolean;
  providerExecuted: boolean;
  /** ms since agent start when the call was reported. */
  startMs: number;
  /** ms since agent start when the result was reported. */
  endMs?: number;
}

export interface UsageRecord {
  inputTokens: number;
  noCacheTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  reasoningTokens: number;
}

export interface StepRecord {
  turn: number;
  step: number;
  startMs: number;
  /**
   * When the model could start this step: the turn start or the previous
   * step's last tool result. `firstOutputMs - readyMs` is time to first token.
   */
  readyMs: number;
  /** First text, reasoning or tool-input part of the step. */
  firstOutputMs?: number;
  endMs?: number;
  finishReason?: string;
  usage?: UsageRecord;
  text: string;
  reasoning: string;
  toolCallIds: string[];
}

export interface TurnRecord {
  turn: number;
  prompt: string;
  freshSession: boolean;
  startMs: number;
  endMs?: number;
  finishReason?: string;
  /** Turn total from the `finish` part (the only real usage for Codex). */
  totalUsage?: UsageRecord;
  providerMetadata?: unknown;
  /** True when the harness left the turn waiting for host input. */
  unfinished: boolean;
  error?: string;
  text: string;
}

export interface Transcript {
  turns: TurnRecord[];
  steps: StepRecord[];
  toolCalls: ToolCallRecord[];
  errors: Array<{ atMs: number; turn: number; message: string }>;
}

// ---------------------------------------------------------------------------
// Report

export interface PhaseTimings {
  totalMs: number;
  sandboxMs: number;
  setupMs: number;
  agentMs: number;
  verifyMs: number;
  collectMs: number;
}

export interface Milestones {
  /** All values are ms since agent start. */
  firstA4CommandMs?: number;
  a4InstalledMs?: number;
  doctorOkMs?: number;
  firstDependencyInstallMs?: number;
  firstProgramRunMs?: number;
}

export interface TokenSummary extends UsageRecord {
  totalTokens: number;
  /** Largest single-step prompt (input incl. cache) — the real context size. */
  peakContextTokens: number;
  contextWindow?: number;
  peakContextFraction?: number;
  compactions: number;
}

export interface CostSummary {
  modelUsd: number;
  /** `gateway-pricing` from the AI Gateway catalog, or `unpriced`. */
  source: 'gateway-pricing' | 'unpriced';
  /** Cost the harness itself reported (Claude Code only), for comparison. */
  harnessReportedUsd?: number;
  /** Upper bound: wall-clock vCPU + memory time at Pro rates. */
  sandboxUsdUpperBound: number;
}

export interface ToolSummary {
  total: number;
  failed: number;
  byName: Record<string, number>;
  byCategory: Record<string, number>;
  a4Commands: Record<string, number>;
  a4Failures: number;
  mcpCalls: Record<string, number>;
  helpLookups: number;
  skillReads: number;
  docsLookups: number;
  directApiCalls: number;
  toolTimeMs: number;
}

export interface TimingSummary extends PhaseTimings {
  turnsMs: number[];
  /** Sum of step durations minus tool time — approximate model time. */
  modelMs: number;
  toolMs: number;
  ttftMs: { first?: number; median?: number; max?: number };
  milestones: Milestones;
}

export interface AreteState {
  a4Version?: string;
  skillsHash?: string;
  doctorBefore?: { status: string; nonOk: string[] };
  doctorAfter?: { status: string; nonOk: string[] };
  /** Change in `/api/agents/me` usage meters across the agent phase. */
  usageDelta?: Record<string, number>;
  usageAttribution: 'exclusive' | 'shared' | 'unknown';
}

export interface VerificationSummary {
  passed: boolean;
  score: number;
  checks: CheckResult[];
}

export interface RunReport {
  schemaVersion: 2;
  runId: string;
  startedAt: string;
  config: RunConfig;
  task: { name: string; track: TaskDefinition['track']; setup: TaskDefinition['setup'] };
  versions: Record<string, string>;
  status: 'success' | 'agent-error' | 'setup-error' | 'infra-error';
  error?: string;
  nativeSessionIds: Record<string, string>;
  timing: TimingSummary;
  tokens: TokenSummary;
  cost: CostSummary;
  tools: ToolSummary;
  steps: number;
  arete: AreteState;
  /** Model API retries the harness reported; rate limits inflate timings. */
  harness: { apiRetries: number; rateLimitRetries: number };
  workspace?: WorkspaceDiff;
  friction: FrictionReport;
  verification: VerificationSummary;
  finalText: string;
}
