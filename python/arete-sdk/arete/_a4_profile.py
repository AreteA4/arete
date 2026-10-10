"""The key from the active ``a4`` CLI login, for servers and local scripts.

The ``a4`` CLI stores the keys it logs in with in a TOML credentials file
(``~/.arete/credentials.toml``, or ``ARETE_CREDENTIALS_PATH``), keyed by
profile and API URL. This module mirrors the CLI's lookup (the Rust SDK's
``credentials`` module is the reference):

- Profile: ``.arete/auth.toml`` in the working directory (may pin only the
  ``agent`` profile), then ``ARETE_PROFILE``, then the single profile holding
  a key for the API URL. More than one candidate means no key.
- Only the key stored for the default Arete API is used, because that is where
  the SDK's default token endpoint sends it. Keys the CLI stored for another
  API URL are never sent anywhere else.
- Any problem (no file, unreadable, malformed, ambiguous) means no key.
"""

from __future__ import annotations

import logging
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, List, Mapping, Optional, Set, Tuple
from urllib.parse import urlsplit

if sys.version_info >= (3, 11):  # pragma: no cover - version dependent
    import tomllib as _toml
else:  # pragma: no cover - version dependent
    try:
        import tomli as _toml
    except ImportError:
        _toml = None  # type: ignore[assignment]

logger = logging.getLogger(__name__)

DEFAULT_API_URL = "https://api.arete.run"
PROFILE_ENV = "ARETE_PROFILE"
CREDENTIALS_PATH_ENV = "ARETE_CREDENTIALS_PATH"
AGENT_PROFILE = "agent"
HUMAN_PROFILE = "human"
_PROJECT_AUTH_RELATIVE_PATH = (".arete", "auth.toml")
_PROFILE_NAME = re.compile(r"^[A-Za-z0-9_-]{1,64}$")


_LOGIN_KEYS: Set[str] = set()
"""Keys taken from the ``a4`` login. Tracked by value so the destination
restriction survives config copies (binding paths ``replace`` the token
endpoint on the resolved config)."""


def remember_login_key(key: str) -> None:
    _LOGIN_KEYS.add(key)


def is_login_key(key: str) -> bool:
    return key in _LOGIN_KEYS


def is_login_key_destination(url: str) -> bool:
    """True when ``url`` is on the API the ``a4`` login key was stored for.

    A login key is never sent anywhere else, such as a session endpoint a
    stack names on another host.
    """
    try:
        parts = urlsplit(url)
        port = parts.port
    except ValueError:
        return False
    host = (parts.hostname or "").rstrip(".").lower()
    return (
        parts.scheme == "https"
        and host == urlsplit(DEFAULT_API_URL).hostname
        and port in (None, 443)
        and parts.username is None
        and parts.password is None
    )


@dataclass(frozen=True)
class ProfileKey:
    """Outcome of :func:`read_profile_key`."""

    key: Optional[str] = None
    ambiguous: bool = False


class _Invalid(Exception):
    pass


def normalize_api_url(url: str) -> str:
    """Mirror of the CLI's API URL normalisation for credential lookup."""
    trimmed = url.strip().rstrip("/")
    scheme, sep, rest = trimmed.partition("://")
    if not sep:
        scheme, rest = "", trimmed
    match = re.search(r"[/?#]", rest)
    authority_end = match.start() if match else len(rest)
    authority, tail = rest[:authority_end], rest[authority_end:]
    host_start = authority.rfind("@") + 1
    host_port = authority[host_start:]
    if host_port.startswith("["):
        close = host_port.find("]")
        host, port = (host_port, "") if close < 0 else (host_port[: close + 1], host_port[close + 1 :])
    else:
        colon = host_port.rfind(":")
        host, port = (host_port, "") if colon < 0 else (host_port[:colon], host_port[colon:])
    host = host.rstrip(".").lower()
    prefix = f"{scheme.lower()}://" if scheme else ""
    return f"{prefix}{authority[:host_start]}{host}{port}{tail}"


def _string_map(value: Any) -> Optional[Dict[str, str]]:
    if value is None:
        return None
    if not isinstance(value, dict) or not all(isinstance(v, str) for v in value.values()):
        raise _Invalid()
    return value


def _find_url_key(keys: Optional[Mapping[str, str]], api_url: str) -> Optional[str]:
    if not keys:
        return None
    wanted = normalize_api_url(api_url)
    exact = keys.get(api_url)
    if exact is None:
        exact = keys.get(wanted)
    if exact is not None and exact.strip():
        return exact.strip()
    for url in sorted(keys):
        if normalize_api_url(url) == wanted and keys[url].strip():
            return keys[url].strip()
    return None


def _key_fits_profile(profile: str, key: str) -> bool:
    agent_key = key.strip().startswith("a4_ak_")
    if profile == AGENT_PROFILE:
        return agent_key
    if profile == HUMAN_PROFILE:
        return not agent_key
    return True


def lookup_credentials(
    content: str, api_url: str, requested_profile: Optional[str]
) -> Tuple[str, Optional[str]]:
    """Mirror of the Rust ``lookup_credentials``.

    Returns ``(kind, key)`` where kind is ``key``, ``none``, ``ambiguous`` or
    ``invalid``.
    """
    if _toml is None:
        return ("invalid", None)
    try:
        parsed = _toml.loads(content)
        profiles_value = parsed.get("profiles")
        if profiles_value is not None and not isinstance(profiles_value, dict):
            raise _Invalid()
        profiles: List[Tuple[str, Optional[Dict[str, str]]]] = []
        for name, profile in (profiles_value or {}).items():
            if not isinstance(profile, dict):
                raise _Invalid()
            profiles.append((name, _string_map(profile.get("keys"))))
        legacy_keys = _string_map(parsed.get("keys"))
        api_key = parsed.get("api_key")
        if api_key is not None and not isinstance(api_key, str):
            raise _Invalid()
    except Exception:
        return ("invalid", None)
    legacy = _find_url_key(legacy_keys, api_url) or ((api_key or "").strip() or None)

    if requested_profile is not None:
        if not _PROFILE_NAME.match(requested_profile):
            return ("invalid", None)
        keys = next((k for name, k in profiles if name == requested_profile), None)
        key = _find_url_key(keys, api_url)
        if key is not None:
            return ("key", key) if _key_fits_profile(requested_profile, key) else ("invalid", None)
        if legacy is not None:
            if requested_profile == AGENT_PROFILE:
                compatible = legacy.startswith("a4_ak_")
            else:
                compatible = requested_profile == HUMAN_PROFILE and not legacy.startswith("a4_ak_")
            if compatible:
                return ("key", legacy)
        return ("none", None)

    matches = [
        (name, key)
        for name, key in ((name, _find_url_key(keys, api_url)) for name, keys in profiles)
        if key is not None
    ]
    if len(matches) > 1:
        return ("ambiguous", None)
    if len(matches) == 1:
        name, key = matches[0]
        return ("key", key) if _key_fits_profile(name, key) else ("invalid", None)
    return ("key", legacy) if legacy is not None else ("none", None)


def _read_text(path: Path) -> Optional[str]:
    """File contents, ``None`` when it does not exist; raises otherwise."""
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None


def _select_profile(
    env: Mapping[str, str], cwd: Optional[Path], read_text: Callable[[Path], Optional[str]]
) -> Tuple[bool, Optional[str]]:
    """``(ok, profile)``; ``ok`` is False when the selection is invalid."""
    if cwd is not None:
        try:
            project = read_text(cwd.joinpath(*_PROJECT_AUTH_RELATIVE_PATH))
        except Exception:
            return (False, None)
        if project is not None:
            if _toml is None:
                return (False, None)
            try:
                profile = _toml.loads(project).get("default_profile")
            except Exception:
                return (False, None)
            return (True, AGENT_PROFILE) if profile == AGENT_PROFILE else (False, None)
    profile = env.get(PROFILE_ENV)
    if profile is None:
        return (True, None)
    profile = profile.strip()
    return (True, profile) if _PROFILE_NAME.match(profile) else (False, None)


def read_profile_key(
    env: Optional[Mapping[str, str]] = None,
    cwd: Optional[Path] = None,
    home: Optional[Path] = None,
    read_text: Callable[[Path], Optional[str]] = _read_text,
) -> ProfileKey:
    """Key of the active ``a4`` login for the default Arete API. Never raises.

    The caller checks the key class.
    """
    try:
        if env is None:
            env = os.environ
        if cwd is None:
            try:
                cwd = Path.cwd()
            except OSError:
                # The project file cannot be checked; choose nothing rather
                # than risk a profile the project excludes.
                logger.debug("working directory unknown; not using an a4 login key")
                return ProfileKey()
        ok, profile = _select_profile(env, cwd, read_text)
        if not ok:
            logger.debug("a4 profile selection is invalid; not using an a4 login key")
            return ProfileKey()
        override = env.get(CREDENTIALS_PATH_ENV)
        if override:
            path = Path(override)
        else:
            if home is None:
                home = Path.home()
            path = home / ".arete" / "credentials.toml"
        content = read_text(path)
        if content is None:
            return ProfileKey()
        kind, key = lookup_credentials(content, DEFAULT_API_URL, profile)
        if kind == "key":
            return ProfileKey(key=key)
        if kind == "ambiguous":
            return ProfileKey(ambiguous=True)
        if kind == "invalid":
            logger.debug("a4 credentials could not be used")
        return ProfileKey()
    except Exception as error:  # never let credential discovery crash a client
        logger.debug("could not read a4 credentials: %s", type(error).__name__)
        return ProfileKey()
