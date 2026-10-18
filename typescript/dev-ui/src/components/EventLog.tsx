import type { DevEvent } from "../types";

function summary(event: DevEvent): string {
  const data = event.data ?? {};
  switch (event.type) {
    case "step.prepared":
    case "step.submitted":
    case "step.confirmed":
    case "step.failed":
      return `${data.step} · ${data.rejected ? `rejected ${data.rejected}` : data.state}${data.instanceId ? ` · ${String(data.instanceId).slice(0, 8)}` : ""}${data.error && data.rejected ? ` · ${data.error}` : ""}${data.hint ? ` · ${data.hint}` : ""}`;
    case "instance.transitioned":
      return `${String(data.instanceId).slice(0, 8)} ${data.from ?? "∅"} → ${data.to}`;
    case "instance.emitted":
      return `${data.event} (${data.reaction})`;
    case "stack.row":
      return `${data.view} ${data.op} ${String(data.key).slice(0, 12)}`;
    case "build.failed":
      return data.diagnostics?.[0]?.message ?? "failed";
    case "build.started":
      return data.changed?.join(", ") || "initial";
    case "deploy.activated":
      return String(data.deploymentId ?? "");
    case "chain.clock":
      return `${data.before ?? ""} → ${data.unixTimestamp}`;
    case "env.changed":
      return `${data.kind}${data.detail?.reason ? ` (${data.detail.reason})` : ""}${data.detail?.name ? ` ${data.detail.name}` : ""}`;
    default:
      return "";
  }
}

export function EventLog({ events }: { events: DevEvent[] }) {
  return (
    <div className="event-log" data-testid="event-log">
      <h3>Events</h3>
      <ol>
        {events.slice(0, 80).map((event) => (
          <li
            key={`${event.seq}:${event.type}:${event.at}`}
            className={`log-entry ${event.type.replace(/\./g, "-")}${event.data?.history ? " history" : ""}`}
            title={event.data?.history ? "Happened before this session's feed started" : undefined}
          >
            <span className="log-seq">{event.seq || "·"}</span>
            <span className="log-type">
              {event.type}
              {event.data?.history ? <span className="muted small"> (history)</span> : null}
            </span>
            <span className="log-summary">{summary(event)}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
