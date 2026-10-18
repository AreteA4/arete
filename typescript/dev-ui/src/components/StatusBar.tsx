import type { Connection } from "../model";
import type { BuildStatus, Deployment } from "../types";

export interface StatusBarProps {
  title: string;
  connection: Connection;
  build: BuildStatus | null;
  buildSummary: string;
  deployment: Deployment | null;
  /** The cloud dev environment the session runs against. */
  environment?: { id: string; name: string } | null;
  chain: { slot?: number; unixTimestamp?: number } | null;
  degraded?: string | null;
}

export function StatusBar({ title, connection, build, buildSummary, deployment, environment, chain, degraded }: StatusBarProps) {
  return (
    <header className="status-bar" data-testid="status-bar">
      <div className="brand">
        <span className="logo">a4</span> <strong>{title}</strong>
      </div>
      <div className={`pill connection ${connection}`} data-testid="connection">
        <span className="dot" />
        {connection}
      </div>
      {build ? (
        <div className={`pill build ${build.state}`} data-testid="build-state" title={buildSummary}>
          build {build.state}
          {buildSummary ? <span className="muted"> · {buildSummary}</span> : null}
        </div>
      ) : null}
      {environment?.id ? (
        <div className="pill mono" data-testid="environment" title={environment.id}>
          env {environment.name}
        </div>
      ) : null}
      {deployment?.deploymentId ? (
        <div className="pill mono" title={deployment.releaseHash}>
          deployment {deployment.deploymentId.slice(0, 8)}
          {deployment.shortId ? ` (${deployment.shortId})` : ""}
        </div>
      ) : null}
      {chain?.unixTimestamp ? (
        <div className="pill mono" data-testid="chain-clock">
          slot {chain.slot} · {new Date(chain.unixTimestamp * 1000).toISOString().replace(".000Z", "Z")}
        </div>
      ) : null}
      {degraded ? (
        <div className="pill warn" title={degraded}>
          feed degraded
        </div>
      ) : null}
    </header>
  );
}
