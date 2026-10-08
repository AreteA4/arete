import 'dotenv/config';
import { Sandbox } from '@vercel/sandbox';

/**
 * Print the toolchain of a sandbox image or runtime, e.g.
 *   npx tsx scripts/probe-image.ts vercel/sandbox/ubuntu
 *   npx tsx scripts/probe-image.ts node24
 */
const target = process.argv[2] ?? 'vercel/sandbox/universal';
const selector = target.includes('/') ? { image: target } : { runtime: target };
const sandbox = await Sandbox.create({
  ...selector,
  timeout: 5 * 60_000,
  token: process.env.VERCEL_TOKEN!,
  teamId: process.env.VERCEL_TEAM_ID!,
  projectId: process.env.VERCEL_PROJECT_ID!,
} as Parameters<typeof Sandbox.create>[0]);
try {
  const cmd = await sandbox.runCommand('bash', [
    '-lc',
    [
      'echo "== $(. /etc/os-release && echo $PRETTY_NAME) $(uname -m)"',
      'echo "user=$(whoami) home=$HOME pwd=$(pwd)"',
      'ldd --version 2>&1 | head -1',
      'for t in node npm pnpm git curl tar python3 timeout sudo; do printf "%s: " $t; (command -v $t >/dev/null && ($t --version 2>/dev/null | head -1 || echo present)) || echo MISSING; done',
      'echo "PATH=$PATH"',
      `curl -fsSL https://arete.run/install.sh | sh -s -- ${process.argv[3] ?? '0.32.0'} >/tmp/a4.log 2>&1 && ~/.local/bin/a4 --version || tail -3 /tmp/a4.log`,
    ].join('; '),
  ]);
  console.log(await cmd.stdout());
} finally {
  await sandbox.stop();
}
