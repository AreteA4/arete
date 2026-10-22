import { dev, main } from './cli';

if (process.argv[2] === 'dev') {
  dev().then((code) => {
    process.exit(code);
  });
} else {
  process.exitCode = main(process.argv.slice(2));
}
