import type { TrackedInstance } from "../model";

export interface InstancePanelProps {
  instances: TrackedInstance[];
  selected: string | null;
  onSelect(id: string | null): void;
}

export function shortId(id: string): string {
  return id.length > 13 ? `${id.slice(0, 8)}…` : id;
}

export function InstancePanel({ instances, selected, onSelect }: InstancePanelProps) {
  const sorted = [...instances].sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? ""));
  const current = selected ? instances.find((instance) => instance.id === selected) : undefined;
  return (
    <div className="instances" data-testid="instance-panel">
      <div className="instances-head">
        <h3>Instances ({instances.length})</h3>
        <select
          aria-label="Instance"
          value={selected ?? ""}
          onChange={(event) => onSelect(event.target.value || null)}
          data-testid="instance-picker"
        >
          <option value="">— pick an instance —</option>
          {sorted.map((instance) => (
            <option key={instance.id} value={instance.id}>
              {shortId(instance.id)} · {instance.state}
            </option>
          ))}
        </select>
      </div>
      {current ? (
        <div className="timeline" data-testid="timeline">
          <div className="mono small">{current.id}</div>
          <div className="muted small">
            state <strong>{current.state}</strong>
            {current.roles && Object.keys(current.roles).length > 0
              ? ` · roles ${Object.entries(current.roles)
                  .map(([role, wallet]) => `${role}=${shortId(String(wallet))}`)
                  .join(", ")}`
              : ""}
          </div>
          {current.history.length === 0 ? (
            <div className="muted small">No events for this instance since a4 dev started.</div>
          ) : (
            <ol>
              {current.history.map((entry) => (
                <li key={entry.seq} className={`entry ${entry.kind} ${entry.ok === false ? "fail" : ""}`}>
                  <span className="entry-at">{new Date(entry.at).toLocaleTimeString()}</span>
                  <span className="entry-label">{entry.label}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
      ) : null}
    </div>
  );
}
