"""Where an account credential may be sent.

An account credential (a secret or agent key, whether passed in code, read
from ``ARETE_API_KEY`` or taken from the ``a4`` login) is only ever sent to a
trusted Arete API origin. Session endpoints named by stack, program or
protocol descriptors are data: they make no origin trusted.

Trusted origins:

- ``https://api.arete.run`` for every credential.
- The ``ARETE_API_URL`` origin, for ``ARETE_API_KEY`` and explicit keys.
- An ``api_url`` the app passed in code, for explicit keys only.

The ``a4`` login key goes only to ``https://api.arete.run``, the API it was
stored for.
"""

from __future__ import annotations

import os
from typing import Mapping, Optional
from urllib.parse import urlsplit

from arete.errors import AuthError

DEFAULT_API_URL = "https://api.arete.run"
ARETE_API_URL_ENV = "ARETE_API_URL"
UNTRUSTED_CREDENTIAL_DESTINATION = "UNTRUSTED_CREDENTIAL_DESTINATION"

EXPLICIT_SECRET_KEY = "secret key"
ENV_API_KEY = "ARETE_API_KEY"
A4_LOGIN = "a4 login"

_DEFAULT_PORTS = {"https": 443, "http": 80}


def url_origin(url: object) -> Optional[str]:
    """``scheme://host:port`` of an http(s) URL, else ``None``.

    The host is lowercased without a trailing dot and the port is always
    explicit. URLs with userinfo or that do not parse give ``None``.
    """
    if not isinstance(url, str):
        return None
    try:
        parts = urlsplit(url.strip())
        port = parts.port
    except ValueError:
        return None
    scheme = parts.scheme.lower()
    host = (parts.hostname or "").rstrip(".").lower()
    if (
        scheme not in _DEFAULT_PORTS
        or not host
        or parts.username is not None
        or parts.password is not None
        or "@" in parts.netloc
    ):
        return None
    if ":" in host:
        host = f"[{host}]"
    return f"{scheme}://{host}:{port if port is not None else _DEFAULT_PORTS[scheme]}"


def _display_origin(url: str) -> str:
    origin = url_origin(url)
    if origin is None:
        return "an unparseable URL"
    scheme, _, rest = origin.partition("://")
    default = f":{_DEFAULT_PORTS[scheme]}"
    return f"{scheme}://{rest[: -len(default)] if rest.endswith(default) else rest}"


def credential_may_reach(
    url: str,
    credential: str,
    *,
    api_url: Optional[str] = None,
    environ: Optional[Mapping[str, str]] = None,
) -> bool:
    """True when ``credential`` (one of ``EXPLICIT_SECRET_KEY``,
    ``ENV_API_KEY``, ``A4_LOGIN``) may be sent to ``url``.

    ``api_url`` is an API URL the app configured in code; it only widens an
    explicit key.
    """
    destination = url_origin(url)
    if destination is None:
        return False
    trusted = {url_origin(DEFAULT_API_URL)}
    if credential != A4_LOGIN:
        env = os.environ if environ is None else environ
        trusted.add(url_origin(env.get(ARETE_API_URL_ENV) or ""))
    if credential == EXPLICIT_SECRET_KEY and api_url:
        trusted.add(url_origin(api_url))
    trusted.discard(None)
    return destination in trusted


def ensure_credential_may_reach(
    url: str,
    credential: str,
    *,
    api_url: Optional[str] = None,
    environ: Optional[Mapping[str, str]] = None,
) -> None:
    """Raise :class:`AuthError` (``UNTRUSTED_CREDENTIAL_DESTINATION``) unless
    :func:`credential_may_reach`. The message never contains key material."""
    if credential_may_reach(url, credential, api_url=api_url, environ=environ):
        return
    origin = _display_origin(url)
    if credential == A4_LOGIN:
        advice = (
            f"The a4 login key is only sent to {DEFAULT_API_URL}; pass the key "
            "explicitly (AuthConfig(secret_key=...)) to use it elsewhere."
        )
    elif credential == ENV_API_KEY:
        advice = (
            f"Set {ARETE_API_URL_ENV} to that origin to allow it, or pass the "
            "key explicitly with api_url set to that origin in code."
        )
    else:
        advice = (
            "Pass api_url with that origin in code, or set "
            f"{ARETE_API_URL_ENV} to it, to allow it."
        )
    raise AuthError(
        f"Refusing to send the {credential} to {origin}: it is not a trusted "
        f"Arete API origin. {advice}",
        UNTRUSTED_CREDENTIAL_DESTINATION,
        {"destination_origin": origin, "credential": credential},
    )
