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

/** A task's name, plus its grading version once that has changed. */
export function taskLabel(task: RunReport['task']): string {
  const grading = task.grading ?? 1;
  return grading > 1 ? `${task.name} (grading ${grading})` : task.name;
}

/**
 * A run's comparison label: its task label, marked when the agent had to
 * create its own account, since that adds signup to the work being timed
 * and graded.
 */
export function runLabel(report: Pick<RunReport, 'task' | 'config'>): string {
  const label = taskLabel(report.task);
  return report.config.keyMode === 'fresh' ? `${label} [fresh account]` : label;
}

export function quantile(values: number[], q: number): number | undefined {
  if (!values.length) return undefined;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
}
