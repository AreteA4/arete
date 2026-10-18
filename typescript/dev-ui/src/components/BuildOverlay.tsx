import type { BuildStatus } from "../types";

export function BuildOverlay({ build, onDismiss }: { build: BuildStatus; onDismiss?(): void }) {
  if (build.state === "building") {
    return (
      <div className="overlay building" data-testid="build-building">
        Rebuilding{build.changed?.length ? ` (${build.changed.join(", ")})` : ""}…
      </div>
    );
  }
  if (build.state !== "failed") return null;
  const diagnostics = build.diagnostics ?? [];
  return (
    <div className="overlay failed" role="alert" data-testid="build-failed">
      <div className="overlay-head">
        <strong>Build failed</strong>
        <span className="muted"> — the diagram shows the last good build</span>
        {onDismiss ? (
          <button className="icon" onClick={onDismiss} aria-label="Dismiss">
            ×
          </button>
        ) : null}
      </div>
      <ul>
        {diagnostics.map((diagnostic, index) => (
          <li key={index} className={diagnostic.severity}>
            {diagnostic.file ? (
              <span className="mono">
                {diagnostic.file}:{diagnostic.line}:{diagnostic.column}{" "}
              </span>
            ) : diagnostic.path ? (
              <span className="mono">{diagnostic.path} </span>
            ) : null}
            {diagnostic.message}
            {diagnostic.code ? <span className="tag">{diagnostic.code}</span> : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
