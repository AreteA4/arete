"""Incremental ``text/event-stream`` parser.

Port of ``typescript/core/src/protocol/sse.ts`` (WHATWG HTML, "Server-sent
events"): feed decoded text chunks; complete events come back as they finish.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import List, Optional

_LINE_BREAK = re.compile(r"\r\n|\r|\n")
_DIGITS = re.compile(r"[0-9]+")


@dataclass(frozen=True)
class ServerSentEvent:
    """One dispatched Server-Sent Event."""

    event: str
    """``event:`` field; ``message`` when absent."""
    data: str
    id: Optional[str]
    """The last event id in effect when this event was dispatched."""


class ServerSentEventParser:
    def __init__(self, last_event_id: Optional[str] = None) -> None:
        self._buffer = ""
        self._event_type = ""
        self._data: List[str] = []
        self._has_data = False
        self._last_event_id = last_event_id
        self.retry_ms: Optional[int] = None
        """Reconnection time the server asked for with ``retry:``."""

    @property
    def last_event_id(self) -> Optional[str]:
        return self._last_event_id

    def push(self, chunk: str) -> List[ServerSentEvent]:
        self._buffer += chunk
        events: List[ServerSentEvent] = []
        while True:
            match = _LINE_BREAK.search(self._buffer)
            if match is None:
                break
            # A lone CR at the end of the buffer may be the first half of CRLF.
            if match.group(0) == "\r" and match.start() == len(self._buffer) - 1:
                break
            line = self._buffer[: match.start()]
            self._buffer = self._buffer[match.end() :]
            event = self._line(line)
            if event is not None:
                events.append(event)
        return events

    def end(self) -> None:
        """Discard a partial event at end of stream, as the specification requires."""
        self._buffer = ""
        self._event_type = ""
        self._data = []
        self._has_data = False

    def _line(self, line: str) -> Optional[ServerSentEvent]:
        if line == "":
            return self._dispatch()
        if line.startswith(":"):
            return None
        field, colon, value = line.partition(":")
        if not colon:
            value = ""
        if value.startswith(" "):
            value = value[1:]
        if field == "event":
            self._event_type = value
        elif field == "data":
            self._data.append(value)
            self._has_data = True
        elif field == "id":
            if "\0" not in value:
                self._last_event_id = value
        elif field == "retry":
            if _DIGITS.fullmatch(value):
                self.retry_ms = int(value)
        return None

    def _dispatch(self) -> Optional[ServerSentEvent]:
        event = (
            ServerSentEvent(
                event=self._event_type or "message",
                data="\n".join(self._data),
                id=self._last_event_id,
            )
            if self._has_data
            else None
        )
        self._event_type = ""
        self._data = []
        self._has_data = False
        return event
