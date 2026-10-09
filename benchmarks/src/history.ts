import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import type { RunReport } from './types.js';

export function loadReports(root: string): RunReport[] {
  const reports: RunReport[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        if (name === 'workspace' || name === 'native') continue;
        walk(path);
      } else if (name === 'report.json') {
        const report = JSON.parse(readFileSync(path, 'utf8')) as RunReport;
        if (report.schemaVersion === 2) reports.push(report);
      }
    }
  };
  if (existsSync(root)) walk(root);
  return reports.sort((a, b) => a.startedAt.localeCompare(b.startedAt));
}

export function quantile(values: number[], q: number): number | undefined {
  if (!values.length) return undefined;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
}
