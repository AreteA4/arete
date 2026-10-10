import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type A4ProfileHost,
  isA4LoginKeyDestination,
  lookupA4Credentials,
  parseSimpleToml,
  readA4ProfileKey,
  systemProfileHost,
} from './a4-profile';
import { resetAuthKeyWarningsForTesting, resolveAuthConfig, tokenEndpointApiKey } from './auth-keys';

const AGENT = 'a4_ak_agentkey';
const HUMAN = 'a4_sk_humankey';
const API = 'https://api.arete.run';

const BOTH_PROFILES = `[profiles.agent.keys]
"${API}" = "${AGENT}"

[profiles.human.keys]
"${API}" = "${HUMAN}"
`;

function host(options: {
  env?: Record<string, string>;
  files?: Record<string, string | Error>;
  cwd?: string | null;
  home?: string;
}): A4ProfileHost {
  const files = options.files ?? {};
  return {
    readEnv: (name) => options.env?.[name],
    readTextFile(path) {
      const content = files[path];
      if (content === undefined) return null;
      if (content instanceof Error) throw content;
      return content;
    },
    cwd: () => (options.cwd === null ? undefined : options.cwd ?? '/work'),
    homeDir: () => options.home ?? '/home/me',
    joinPath: (...parts) => parts.join('/'),
  };
}

const CREDENTIALS = '/home/me/.arete/credentials.toml';
const PROJECT = '/work/.arete/auth.toml';

describe('parseSimpleToml', () => {
  it('reads the format the a4 CLI writes', () => {
    const parsed = parseSimpleToml(`# comment
api_key = 'literal'
[profiles.agent.keys]
"https://api.arete.run" = "a4_ak_\\u0041x" # trailing
[profiles.agent.pendingSignup."https://api.arete.run"]
credential = "x"
count = 3
`);
    expect(parsed).toMatchObject({
      api_key: 'literal',
      profiles: { agent: { keys: { 'https://api.arete.run': 'a4_ak_Ax' } } },
    });
  });

  it('rejects what it does not understand', () => {
    for (const content of ['a = [1, 2]', 'a = { b = 1 }', 'a = """x"""', '[[t]]', 'a = "x"\na = "y"', 'nope']) {
      expect(() => parseSimpleToml(content), content).toThrow();
    }
  });
});

describe('lookupA4Credentials', () => {
  it('uses the only profile for the API URL, normalising URLs', () => {
    const content = `[profiles.agent.keys]\n"HTTPS://API.Arete.Run./" = "${AGENT}"\n`;
    expect(lookupA4Credentials(content, API, undefined)).toEqual({ kind: 'key', key: AGENT });
  });

  it('refuses to choose between several profiles', () => {
    expect(lookupA4Credentials(BOTH_PROFILES, API, undefined)).toEqual({ kind: 'ambiguous' });
    expect(lookupA4Credentials(BOTH_PROFILES, API, 'human')).toEqual({ kind: 'key', key: HUMAN });
  });

  it('ignores keys stored for another API URL', () => {
    const content = `[profiles.agent.keys]\n"http://localhost:3000" = "${AGENT}"\n`;
    expect(lookupA4Credentials(content, API, undefined)).toEqual({ kind: 'none' });
  });

  it('reads legacy schemas and checks reserved profiles', () => {
    expect(lookupA4Credentials(`api_key = "${HUMAN}"`, API, undefined)).toEqual({ kind: 'key', key: HUMAN });
    expect(lookupA4Credentials(`[keys]\n"${API}" = "${AGENT}"`, API, 'agent')).toEqual({ kind: 'key', key: AGENT });
    expect(lookupA4Credentials(`api_key = "${HUMAN}"`, API, 'agent')).toEqual({ kind: 'none' });
    const wrongKind = `[profiles.agent.keys]\n"${API}" = "${HUMAN}"\n`;
    expect(lookupA4Credentials(wrongKind, API, 'agent')).toEqual({ kind: 'invalid' });
  });
});

describe('readA4ProfileKey', () => {
  it('selects the profile from ARETE_PROFILE', () => {
    const files = { [CREDENTIALS]: BOTH_PROFILES };
    expect(readA4ProfileKey(host({ files }))).toEqual({ ambiguous: true });
    expect(readA4ProfileKey(host({ files, env: { ARETE_PROFILE: 'agent' } }))).toEqual({ key: AGENT });
    expect(readA4ProfileKey(host({ files, env: { ARETE_PROFILE: 'human' } }))).toEqual({ key: HUMAN });
    expect(readA4ProfileKey(host({ files, env: { ARETE_PROFILE: 'bad name' } }))).toEqual({});
  });

  it('lets the project file pin the agent profile over ARETE_PROFILE', () => {
    const files = { [CREDENTIALS]: BOTH_PROFILES, [PROJECT]: 'default_profile = "agent"\n' };
    expect(readA4ProfileKey(host({ files, env: { ARETE_PROFILE: 'human' } }))).toEqual({ key: AGENT });
  });

  it('uses no key when the project file is invalid', () => {
    for (const project of ['default_profile = "human"', 'garbage {{', new Error('EACCES')]) {
      const files = { [CREDENTIALS]: BOTH_PROFILES, [PROJECT]: project };
      expect(readA4ProfileKey(host({ files, env: { ARETE_PROFILE: 'agent' } }))).toEqual({});
    }
  });

  it('uses no key when the project file cannot be checked', () => {
    const files = { [CREDENTIALS]: BOTH_PROFILES };
    expect(readA4ProfileKey(host({ files, cwd: null, env: { ARETE_PROFILE: 'human' } }))).toEqual({});
  });

  it('honours the credentials path override', () => {
    const files = { '/elsewhere/creds.toml': `api_key = "${HUMAN}"` };
    const env = { ARETE_CREDENTIALS_PATH: '/elsewhere/creds.toml' };
    expect(readA4ProfileKey(host({ files, env }))).toEqual({ key: HUMAN });
  });

  it('treats missing, unreadable and malformed files as no key', () => {
    expect(readA4ProfileKey(host({}))).toEqual({});
    expect(readA4ProfileKey(host({ files: { [CREDENTIALS]: new Error('EACCES') } }))).toEqual({});
    expect(readA4ProfileKey(host({ files: { [CREDENTIALS]: 'not toml {{{' } }))).toEqual({});
  });
});

describe('resolveAuthConfig credential chain', () => {
  const profileKey = () => ({ key: AGENT });

  beforeEach(() => {
    resetAuthKeyWarningsForTesting();
    vi.stubEnv('ARETE_API_KEY', '');
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('prefers an explicit option, then ARETE_API_KEY, then the a4 login', () => {
    const never = () => {
      throw new Error('profile must not be read');
    };
    expect(resolveAuthConfig({ secretKey: HUMAN }, never)?.secretKey).toBe(HUMAN);
    expect(resolveAuthConfig({ token: 't' }, never)?.secretKey).toBeUndefined();

    vi.stubEnv('ARETE_API_KEY', 'a4_sk_fromenv');
    expect(resolveAuthConfig(undefined, never)?.secretKey).toBe('a4_sk_fromenv');

    vi.stubEnv('ARETE_API_KEY', '');
    expect(resolveAuthConfig(undefined, profileKey)?.secretKey).toBe(AGENT);
    expect(resolveAuthConfig(undefined, () => ({}))).toBeUndefined();
  });

  it('falls through a publishable ARETE_API_KEY to the a4 login', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.stubEnv('ARETE_API_KEY', 'a4_pk_public');
    expect(resolveAuthConfig(undefined, profileKey)?.secretKey).toBe(AGENT);
  });

  it('accepts only secret-class keys from the a4 login', () => {
    expect(resolveAuthConfig(undefined, () => ({ key: 'a4_pk_public' }))).toBeUndefined();
    expect(resolveAuthConfig(undefined, () => ({ key: 'custom' }))).toBeUndefined();
  });

  it('warns once, without paths, when several profiles hold a key', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    resolveAuthConfig(undefined, () => ({ ambiguous: true }));
    resolveAuthConfig(undefined, () => ({ ambiguous: true }));
    expect(warn).toHaveBeenCalledTimes(1);
    const message = String(warn.mock.calls[0]?.[0]);
    expect(message).toContain('ARETE_PROFILE');
    expect(message).not.toContain('credentials');
    expect(message).not.toContain('.arete');
  });

  it('never reads the a4 login, or touches Node builtins, in a browser', () => {
    vi.stubGlobal('window', {});
    vi.stubGlobal('document', {});
    const readProfile = vi.fn(() => ({ key: AGENT }));
    expect(resolveAuthConfig(undefined, readProfile)).toBeUndefined();
    expect(readProfile).not.toHaveBeenCalled();

    const processLike = globalThis.process as { getBuiltinModule?: (id: string) => unknown };
    const getBuiltinModule = vi.spyOn(processLike, 'getBuiltinModule' as never);
    expect(resolveAuthConfig(undefined)).toBeUndefined();
    expect(getBuiltinModule).not.toHaveBeenCalled();
  });

  it('sends an a4 login key only to the Arete API', () => {
    const auth = resolveAuthConfig(undefined, profileKey);
    expect(tokenEndpointApiKey(auth, 'https://api.arete.run/ws/sessions')).toBe(AGENT);
    expect(tokenEndpointApiKey(auth, 'https://API.arete.run.:443/x')).toBe(AGENT);
    // A stack-provided session endpoint on another host gets no key, even
    // after the config is copied, as binding paths do.
    const copied = { ...auth, tokenEndpoint: 'https://evil.example/ws/sessions' };
    expect(tokenEndpointApiKey(copied, 'https://evil.example/ws/sessions')).toBeUndefined();
    // Explicit and environment keys are unaffected.
    expect(tokenEndpointApiKey({ secretKey: HUMAN }, 'https://evil.example/s')).toBe(HUMAN);
  });

  it('restricts only the config whose key came from the a4 login', () => {
    const custom = 'https://auth.example.com/token';
    const discovered = resolveAuthConfig(undefined, profileKey);
    // The same key, passed explicitly by another client in the same process.
    const explicit = resolveAuthConfig({ secretKey: AGENT, tokenEndpoint: custom }, profileKey);
    expect(tokenEndpointApiKey(explicit, custom)).toBe(AGENT);
    expect(tokenEndpointApiKey(discovered, custom)).toBeUndefined();
    // A copy of the discovered config given its own key is no longer restricted.
    expect(tokenEndpointApiKey({ ...discovered, secretKey: HUMAN }, custom)).toBe(HUMAN);
    // ARETE_API_KEY keys are not restricted either.
    vi.stubEnv('ARETE_API_KEY', AGENT);
    expect(tokenEndpointApiKey(resolveAuthConfig(undefined, profileKey), custom)).toBe(AGENT);
  });

  it('recognises only the Arete API as the login key destination', () => {
    expect(isA4LoginKeyDestination('https://api.arete.run/ws/sessions')).toBe(true);
    for (const url of [
      'http://api.arete.run/ws/sessions',
      'https://api.arete.run:8443/ws/sessions',
      'https://api.arete.run.evil.example/',
      'https://user@api.arete.run/',
      'https://other.arete.run/',
      'not a url',
    ]) {
      expect(isA4LoginKeyDestination(url), url).toBe(false);
    }
  });

  it('builds a Node host without static imports', () => {
    const system = systemProfileHost();
    expect(system).toBeDefined();
    expect(system?.joinPath('a', 'b')).toMatch(/^a[\\/]b$/);
  });
});
