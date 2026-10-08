import { createClaudeCode } from '@ai-sdk/harness-claude-code';
import { createCodex } from '@ai-sdk/harness-codex';
import { createOpenCode } from '@ai-sdk/harness-opencode';
import type { HarnessAgentAdapter } from '@ai-sdk/harness/agent';
import type { RunConfig } from './types.js';

export interface HarnessSetup {
  adapter: HarnessAgentAdapter;
  /** Model id in the form this harness and auth mode expect. */
  model: string;
  /** `a4 init --agents` id for this harness. */
  a4AgentId: string;
  /**
   * Built-in tools to disable. Runs are unattended, so tools that wait for a
   * human answer would stall the turn. Codex has no question tool and does
   * not support built-in filtering.
   */
  inactiveTools?: string[];
}

/** `anthropic/claude-sonnet-5.5` → `claude-sonnet-5-5` for the Anthropic API. */
function anthropicModelId(model: string): string {
  return model.replace(/^anthropic\//, '').replace(/(\d)\.(\d)/g, '$1-$2');
}

/**
 * Build the adapter for a run. By default every harness authenticates
 * through the Vercel AI Gateway (`AI_GATEWAY_API_KEY`, or
 * `VERCEL_OIDC_TOKEN`), and the real key is brokered by the sandbox so it
 * never appears inside it. `modelAuth: 'direct'` uses provider keys instead.
 */
export function createHarness(config: RunConfig): HarnessSetup {
  const gateway = config.modelAuth === 'ai-gateway';
  switch (config.harness) {
    case 'claude-code': {
      const haiku = gateway ? 'anthropic/claude-haiku-5.5' : 'claude-haiku-5-5';
      return {
        adapter: createClaudeCode({
          auth: gateway ? 'ai-gateway' : 'direct',
          ...(config.effort ? { effort: config.effort } : {}),
          env: {
            // Background calls (titles, summaries) default to a dated Haiku id
            // the gateway does not route; pin them explicitly.
            ANTHROPIC_DEFAULT_HAIKU_MODEL: haiku,
            ANTHROPIC_SMALL_FAST_MODEL: haiku,
            DISABLE_AUTOUPDATER: '1',
            CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
          },
        }),
        model: gateway ? config.model : anthropicModelId(config.model),
        a4AgentId: 'claude-code',
        inactiveTools: ['askUserQuestions'],
      };
    }
    case 'codex':
      return {
        adapter: createCodex({
          auth: gateway ? 'ai-gateway' : 'direct',
          ...(config.effort ? { reasoningEffort: config.effort } : {}),
        }),
        model: gateway ? config.model : config.model.replace(/^openai\//, ''),
        a4AgentId: 'codex',
      };
    case 'opencode': {
      // Direct mode reaches Anthropic and OpenAI only: the adapter does not
      // forward custom OpenAI-compatible providers such as OpenRouter.
      const provider = config.model.split('/')[0];
      if (!gateway && provider !== 'anthropic' && provider !== 'openai') {
        throw new Error(`OpenCode direct mode supports anthropic/ and openai/ models; use the AI Gateway for ${config.model}`);
      }
      return {
        adapter: createOpenCode({ auth: gateway ? 'ai-gateway' : (provider as 'anthropic' | 'openai') }),
        model: gateway || provider === 'openai' ? config.model : `anthropic/${anthropicModelId(config.model)}`,
        a4AgentId: 'opencode',
        inactiveTools: ['askUserQuestions'],
      };
    }
  }
}

/** Directories (relative to HOME) holding each runtime's native session log. */
export const NATIVE_LOG_DIRS: Record<RunConfig['harness'], string[]> = {
  'claude-code': ['.claude/projects', '.claude/todos'],
  codex: ['.codex/sessions'],
  opencode: ['.local/share/opencode'],
};
