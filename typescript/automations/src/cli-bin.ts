import { dev, main, manifest, migrate } from './cli';

const command = process.argv[2];
const args = process.argv.slice(2);
const exit = (code: number) => {
  process.exit(code);
};

if (command === 'dev') {
  dev().then(exit);
} else if (command === 'manifest') {
  manifest(args).then(exit);
} else if (command === 'migrate') {
  migrate(args).then(exit);
} else {
  process.exitCode = main(args);
}
