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
