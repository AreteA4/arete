// Run every test file as CI does: without the developer's `a4` login or any
// ARETE_* variable. The SDK reads ARETE_API_KEY, ARETE_PROFILE,
// ARETE_HOSTED_WEBSOCKET_SUFFIXES and others from the environment, and the
// login from ~/.arete/credentials.toml (or ARETE_CREDENTIALS_PATH). Setup
// files run before the test file imports the SDK, so module-level reads see
// the cleaned environment too.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

for (const name of Object.keys(process.env)) {
  if (name.startsWith('ARETE_')) delete process.env[name];
}
const isolatedHome = fs.mkdtempSync(path.join(os.tmpdir(), 'arete-react-test-home-'));
process.env.HOME = isolatedHome;
process.env.USERPROFILE = isolatedHome;
// Points at a file that never exists, even where the runtime's home
// directory lookup ignores the HOME set above.
process.env.ARETE_CREDENTIALS_PATH = path.join(isolatedHome, '.arete', 'credentials.toml');
afterAll(() => {
  fs.rmSync(isolatedHome, { recursive: true, force: true });
});

// Fail the test during which React reported a state update outside act(...),
// so the warnings never come back unnoticed.
const actWarnings = [];
const originalConsoleError = console.error;

console.error = (...args) => {
  const [message] = args;
  if (typeof message === 'string' && message.includes('not wrapped in act(')) {
    actWarnings.push(args.map(String).join(' '));
  }
  originalConsoleError.apply(console, args);
};

afterEach(() => {
  if (actWarnings.length === 0) return;
  const warnings = actWarnings.splice(0);
  throw new Error(`React state updates outside act(...):\n${warnings.join('\n')}`);
});
