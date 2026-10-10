"""The ``a4`` login fallback in the server-side credential chain."""

from __future__ import annotations

import logging
from pathlib import Path

import pytest

from arete import auth as auth_module
from arete._a4_profile import ProfileKey, lookup_credentials, read_profile_key
from arete.auth import AuthConfig, resolve_auth_config

AGENT = "a4_ak_agentkey"
HUMAN = "a4_sk_humankey"
API = "https://api.arete.run"
BOTH = f"""[profiles.agent.keys]
"{API}" = "{AGENT}"

[profiles.human.keys]
"{API}" = "{HUMAN}"
"""


def write(path: Path, content: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    return path


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


@pytest.fixture
def work(tmp_path: Path) -> Path:
    path = tmp_path / "work"
    path.mkdir()
    return path


def read(home: Path, work: Path, **env: str) -> ProfileKey:
    return read_profile_key(env=env, cwd=work, home=home)


def test_single_profile_is_used_and_urls_are_normalised(home, work):
    write(home / ".arete/credentials.toml", f'[profiles.agent.keys]\n"HTTPS://API.Arete.Run./" = "{AGENT}"\n')
    assert read(home, work) == ProfileKey(key=AGENT)


def test_profile_selection(home, work):
    write(home / ".arete/credentials.toml", BOTH)
    assert read(home, work) == ProfileKey(ambiguous=True)
    assert read(home, work, ARETE_PROFILE="agent") == ProfileKey(key=AGENT)
    assert read(home, work, ARETE_PROFILE="human") == ProfileKey(key=HUMAN)
    assert read(home, work, ARETE_PROFILE="missing") == ProfileKey()
    assert read(home, work, ARETE_PROFILE="bad name") == ProfileKey()


def test_project_file_pins_agent_over_arete_profile(home, work):
    write(home / ".arete/credentials.toml", BOTH)
    write(work / ".arete/auth.toml", 'default_profile = "agent"\n')
    assert read(home, work, ARETE_PROFILE="human") == ProfileKey(key=AGENT)


@pytest.mark.parametrize("project", ['default_profile = "human"\n', "garbage {{", "other = 1\n"])
def test_invalid_project_file_gives_no_key(home, work, project):
    write(home / ".arete/credentials.toml", BOTH)
    write(work / ".arete/auth.toml", project)
    assert read(home, work, ARETE_PROFILE="agent") == ProfileKey()


def test_credentials_path_override(home, work, tmp_path):
    override = write(tmp_path / "elsewhere.toml", f'api_key = "{HUMAN}"\n')
    assert read(home, work, ARETE_CREDENTIALS_PATH=str(override)) == ProfileKey(key=HUMAN)


def test_only_default_api_url_key_is_used(home, work):
    write(home / ".arete/credentials.toml", f'[profiles.agent.keys]\n"http://localhost:3000" = "{AGENT}"\n')
    assert read(home, work, ARETE_API_URL="http://localhost:3000") == ProfileKey()


def test_missing_unreadable_and_malformed_files_give_no_key(home, work):
    assert read(home, work) == ProfileKey()
    write(home / ".arete/credentials.toml", "not toml {{{")
    assert read(home, work) == ProfileKey()
    (home / ".arete/credentials.toml").unlink()
    (home / ".arete/credentials.toml").mkdir()  # unreadable as a file
    assert read(home, work) == ProfileKey()


def test_legacy_schemas_and_reserved_profiles():
    assert lookup_credentials(f'api_key = "{HUMAN}"', API, None) == ("key", HUMAN)
    assert lookup_credentials(f'[keys]\n"{API}" = "{AGENT}"', API, "agent") == ("key", AGENT)
    assert lookup_credentials(f'api_key = "{HUMAN}"', API, "agent") == ("none", None)
    wrong = f'[profiles.agent.keys]\n"{API}" = "{HUMAN}"\n'
    assert lookup_credentials(wrong, API, "agent") == ("invalid", None)


class TestResolveChain:
    @pytest.fixture(autouse=True)
    def _reset(self, monkeypatch):
        monkeypatch.delenv("ARETE_API_KEY", raising=False)
        auth_module._warned.clear()

    def test_option_then_env_then_profile(self, monkeypatch):
        def never():
            raise AssertionError("profile must not be read")

        assert resolve_auth_config(AuthConfig(secret_key=HUMAN), never).secret_key == HUMAN
        monkeypatch.setenv("ARETE_API_KEY", "a4_sk_fromenv")
        assert resolve_auth_config(None, never).secret_key == "a4_sk_fromenv"
        monkeypatch.delenv("ARETE_API_KEY")
        assert resolve_auth_config(None, lambda: ProfileKey(key=AGENT)).secret_key == AGENT
        assert resolve_auth_config(None, lambda: ProfileKey()) is None

    def test_publishable_env_falls_through_to_profile(self, monkeypatch):
        monkeypatch.setenv("ARETE_API_KEY", "a4_pk_public")
        assert resolve_auth_config(None, lambda: ProfileKey(key=AGENT)).secret_key == AGENT

    def test_only_secret_class_profile_keys(self):
        assert resolve_auth_config(None, lambda: ProfileKey(key="a4_pk_public")) is None
        assert resolve_auth_config(None, lambda: ProfileKey(key="custom")) is None

    def test_default_reader_uses_the_a4_login(self, monkeypatch, tmp_path):
        creds = write(tmp_path / "creds.toml", f'[profiles.agent.keys]\n"{API}" = "{AGENT}"\n')
        monkeypatch.setenv("ARETE_CREDENTIALS_PATH", str(creds))
        monkeypatch.chdir(tmp_path)
        assert resolve_auth_config(None).secret_key == AGENT

    def test_ambiguous_profiles_warn_without_paths(self, caplog):
        with caplog.at_level(logging.WARNING):
            resolve_auth_config(None, lambda: ProfileKey(ambiguous=True))
        text = caplog.text
        assert "ARETE_PROFILE" in text
        assert ".arete" not in text and "credentials" not in text


def test_missing_key_hint_names_commands_not_paths():
    assert "a4 auth login" in auth_module.NO_API_KEY_HINT
    assert "ARETE_API_KEY" in auth_module.NO_API_KEY_HINT
    assert ".arete" not in auth_module.NO_API_KEY_HINT


def test_unknown_working_directory_gives_no_key(home, monkeypatch):
    write(home / ".arete/credentials.toml", BOTH)

    def no_cwd():
        raise FileNotFoundError("cwd removed")

    monkeypatch.setattr(Path, "cwd", staticmethod(no_cwd))
    assert read_profile_key(env={"ARETE_PROFILE": "human"}, home=home) == ProfileKey()


def test_login_key_goes_only_to_the_arete_api(monkeypatch):
    from dataclasses import replace

    from arete._a4_profile import is_login_key_destination
    from arete.auth import api_key_for_endpoint

    monkeypatch.delenv("ARETE_API_KEY", raising=False)
    login_key = "a4_ak_loginonlyforarete"
    resolved = resolve_auth_config(None, lambda: ProfileKey(key=login_key))
    assert api_key_for_endpoint(resolved, "https://api.arete.run/ws/sessions") == login_key
    rebound = replace(resolved, token_endpoint="https://evil.example/sessions")
    assert api_key_for_endpoint(rebound, "https://evil.example/sessions") is None
    explicit = AuthConfig(secret_key=HUMAN)
    assert api_key_for_endpoint(explicit, "https://evil.example/sessions") == HUMAN

    assert is_login_key_destination("https://API.arete.run.:443/x")
    for url in [
        "http://api.arete.run/ws/sessions",
        "https://api.arete.run:8443/ws/sessions",
        "https://api.arete.run.evil.example/",
        "https://user@api.arete.run/",
        "https://other.arete.run/",
        "not a url",
    ]:
        assert not is_login_key_destination(url), url


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "config, hinted",
    [
        (None, True),
        (AuthConfig(token_endpoint_headers={"authorization": "Bearer custom"}), False),
    ],
)
async def test_401_hint_only_without_any_credential(config, hinted):
    import httpx

    from arete.auth import request_token_from_endpoint
    from arete.errors import AuthError

    transport = httpx.MockTransport(
        lambda request: httpx.Response(401, json={"error": "rejected"})
    )
    async with httpx.AsyncClient(transport=transport) as client:
        with pytest.raises(AuthError) as caught:
            await request_token_from_endpoint(
                client, "https://api.arete.run/ws/sessions", config, {}
            )
    assert ("No Arete API key found" in str(caught.value)) is hinted
