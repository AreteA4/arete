"""WebSocket connection management with authentication support."""

import asyncio
import json
import logging
from typing import Optional, Callable, List, Any
from websockets import connect as ws_connect
from websockets.client import WebSocketClientProtocol
from websockets.exceptions import WebSocketException

from hyperstack.errors import ConnectionError, AuthError
from hyperstack.auth import (
    AuthConfig,
    AuthState,
    AuthErrorCode,
    TokenTransport,
    build_websocket_url,
    parse_error_code_from_close_reason,
    should_refresh_token,
    should_retry_error,
    DEFAULT_QUERY_PARAMETER,
)

logger = logging.getLogger(__name__)


class WebSocketManager:
    def __init__(
        self,
        url: str,
        reconnect_intervals: List[int] = None,
        ping_interval: int = 15,
        on_connect: Optional[Callable] = None,
        on_disconnect: Optional[Callable] = None,
        on_error: Optional[Callable] = None,
        on_socket_issue: Optional[Callable[[dict], Any]] = None,
        auth: Optional[AuthConfig] = None,
    ):
        """
        Initialize WebSocket manager with optional authentication.

        Args:
            url: WebSocket server URL
            reconnect_intervals: List of wait intervals (in seconds) between reconnection attempts
            ping_interval: Seconds between keep-alive ping messages
            on_connect: Optional callback invoked when connection is established
            on_disconnect: Optional callback invoked when connection is closed
            on_error: Optional callback invoked when an error occurs
            on_socket_issue: Optional callback for structured socket issues/errors
            auth: Optional authentication configuration
        """
        self.url = url
        self.reconnect_intervals = reconnect_intervals or [1, 2, 4, 8, 16]
        self.ping_interval = ping_interval
        self.on_connect = on_connect
        self.on_disconnect = on_disconnect
        self.on_error = on_error
        self.on_socket_issue = on_socket_issue

        # Authentication state
        self.auth = AuthState(url, auth)
        self._auth_config = auth

        self.ws: Optional[WebSocketClientProtocol] = None
        self.is_running = False
        self.reconnect_attempts = 0
        self.receive_task: Optional[asyncio.Task] = None
        self.ping_task: Optional[asyncio.Task] = None
        self.refresh_task: Optional[asyncio.Task] = None
        self.message_handler: Optional[Callable] = None

        # Track if we're reconnecting for token refresh
        self._force_token_refresh = False
        self._immediate_reconnect = False

    def set_message_handler(self, handler: Callable) -> None:
        """
        Set the callback function for handling incoming WebSocket messages.

        Args:
            handler: Async callable that processes incoming messages
        """
        self.message_handler = handler

    async def _ping_loop(self) -> None:
        """Send periodic ping messages to keep connection alive."""
        while self.is_running and self.ws:
            await asyncio.sleep(self.ping_interval)
            if self.is_running and self.ws:
                try:
                    await self.ws.send('{"type":"ping"}')
                    logger.debug("Sent keep-alive ping")
                except Exception as e:
                    logger.warning(f"Ping failed: {e}")
                    break

    def _start_ping(self) -> None:
        """Start the ping loop."""
        self._stop_ping()
        self.ping_task = asyncio.create_task(self._ping_loop())

    def _stop_ping(self) -> None:
        """Stop the ping loop."""
        if self.ping_task:
            self.ping_task.cancel()
            self.ping_task = None

    async def _token_refresh_loop(self) -> None:
        """Background task to refresh tokens before they expire."""
        while self.is_running:
            try:
                delay = self.auth.get_refresh_delay()
                if delay is None:
                    # No refresh needed, exit loop
                    break

                logger.debug(f"Token refresh scheduled in {delay} seconds")
                await asyncio.sleep(delay)

                if not self.is_running:
                    break

                # Refresh token
                previous_token = self.auth._current_token
                await self.auth.resolve_token(force_refresh=True)

                # If token changed, send in-band refresh or reconnect
                if (
                    previous_token != self.auth._current_token
                    and self.ws
                    and self.ws.open
                ):
                    await self._send_in_band_auth_refresh()

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.warning(f"Token refresh failed: {e}")
                # Retry after minimum delay
                await asyncio.sleep(1)

    def _start_token_refresh(self) -> None:
        """Start the token refresh background task."""
        if self.auth.has_refreshable_auth():
            self.refresh_task = asyncio.create_task(self._token_refresh_loop())

    def _stop_token_refresh(self) -> None:
        """Stop the token refresh task."""
        if self.refresh_task:
            self.refresh_task.cancel()
            self.refresh_task = None

    async def _send_in_band_auth_refresh(self) -> bool:
        """Send in-band auth refresh message via WebSocket.

        Returns True if message was sent successfully.
        """
        if not self.ws or not self.ws.open:
            return False

        try:
            message = json.dumps(
                {
                    "type": "refresh_auth",
                    "token": self.auth._current_token,
                }
            )
            await self.ws.send(message)
            logger.debug("Sent in-band auth refresh")
            return True
        except Exception as e:
            logger.warning(f"Failed to send in-band auth refresh: {e}")
            return False

    async def _resolve_auth(self) -> Optional[str]:
        """Resolve authentication token for connection.

        Returns the token string or None if no auth.
        Raises AuthError if auth fails.
        """
        try:
            return await self.auth.resolve_token(
                force_refresh=self._force_token_refresh
            )
        except AuthError:
            raise
        except Exception as e:
            raise AuthError(
                f"Authentication failed: {e}", AuthErrorCode.AUTH_REQUIRED
            ) from e

    def _get_connection_url(self, token: Optional[str]) -> str:
        """Build WebSocket URL with token (if using query transport)."""
        transport = (
            self._auth_config.token_transport
            if self._auth_config
            else TokenTransport.QUERY
        )
        return build_websocket_url(self.url, token, transport)

    def _get_connection_headers(self, token: Optional[str]) -> Optional[dict]:
        """Get connection headers (for bearer token transport)."""
        if not token:
            return None

        transport = (
            self._auth_config.token_transport
            if self._auth_config
            else TokenTransport.QUERY
        )
        if transport == TokenTransport.BEARER:
            return {"Authorization": f"Bearer {token}"}

        return None

    async def connect(self) -> None:
        """
        Establish WebSocket connection with automatic retry logic.

        Attempts to connect using the configured reconnect intervals.
        Starts the message receiving loop once connected.

        Raises:
            ConnectionError: If all connection attempts fail
            AuthError: If authentication fails
        """
        if self.is_running and self.ws:
            logger.info("Already connected")
            return

        attempt = 0
        while attempt < len(self.reconnect_intervals):
            try:
                # Resolve authentication
                token = await self._resolve_auth()
                self._force_token_refresh = False

                # Build connection URL and headers
                ws_url = self._get_connection_url(token)
                headers = self._get_connection_headers(token)

                logger.info(f"Connecting to {self.url}...")
                if headers:
                    self.ws = await ws_connect(ws_url, additional_headers=headers)
                else:
                    self.ws = await ws_connect(ws_url)

                self.is_running = True
                self.reconnect_attempts = 0
                self._immediate_reconnect = False
                logger.info("Connected")

                self.receive_task = asyncio.create_task(self.receive_messages())
                self._start_ping()
                self._start_token_refresh()

                if self.on_connect:
                    await self.on_connect()

                return

            except AuthError:
                # Don't retry auth errors, propagate immediately
                raise
            except Exception as e:
                attempt += 1
                if attempt >= len(self.reconnect_intervals):
                    raise ConnectionError(
                        f"Connection failed after {attempt} attempts: {e}"
                    )

                wait = self.reconnect_intervals[attempt - 1]
                logger.warning(f"Retrying in {wait}s (attempt {attempt})")
                await asyncio.sleep(wait)

        raise ConnectionError("Failed to connect")

    async def disconnect(self) -> None:
        """Close WebSocket connection and cleanup resources."""
        self.is_running = False
        self._stop_ping()
        self._stop_token_refresh()

        # Cancel refresh task
        if self.refresh_task and not self.refresh_task.done():
            self.refresh_task.cancel()
            try:
                await self.refresh_task
            except asyncio.CancelledError:
                pass

        if self.receive_task:
            self.receive_task.cancel()
            try:
                await self.receive_task
            except asyncio.CancelledError:
                pass

        if self.ws:
            await self.ws.close()
            self.ws = None

        # Cleanup auth state
        await self.auth.close()

        if self.on_disconnect:
            await self.on_disconnect()

        logger.info("Disconnected")

    async def _handle_socket_issue(self, issue: dict) -> None:
        """Handle structured socket issue messages from server."""
        error_code_str = issue.get("code")
        error_code = AuthErrorCode.from_wire(error_code_str) if error_code_str else None

        # Notify callback
        if self.on_socket_issue:
            try:
                await self.on_socket_issue(issue)
            except Exception as e:
                logger.error(f"Socket issue callback error: {e}")

        # Check if we should refresh token and reconnect
        if error_code and should_refresh_token(error_code):
            if self.auth.has_refreshable_auth():
                logger.info(f"Token refresh required due to: {error_code.value}")
                self.auth.clear_token()
                self._force_token_refresh = True
                self._immediate_reconnect = True

    async def _handle_refresh_auth_response(self, data: dict) -> None:
        """Handle refresh_auth response from server."""
        success = data.get("success", False)

        if success:
            # Update token expiry if provided
            expires_at = data.get("expires_at") or data.get("expiresAt")
            if expires_at:
                self.auth._token_expiry = int(expires_at)
            logger.debug("In-band auth refresh successful")
        else:
            error = data.get("error", "Unknown error")
            logger.warning(f"In-band auth refresh failed: {error}")

            # Clear token and force reconnect
            error_code = parse_error_code_from_close_reason(error)
            if error_code and should_refresh_token(error_code):
                self.auth.clear_token()
                self._force_token_refresh = True
            self._immediate_reconnect = True

    async def receive_messages(self) -> None:
        """
        Continuously receive and process WebSocket messages.

        Handles incoming messages via the configured message handler.
        Automatically triggers reconnection on WebSocket errors if still running.
        """
        if not self.ws:
            return

        try:
            async for message in self.ws:
                # First, check for special message types
                if isinstance(message, str):
                    try:
                        data = json.loads(message)

                        # Check for socket issue message
                        if data.get("type") == "error":
                            await self._handle_socket_issue(data)
                            continue

                        # Check for refresh auth response
                        if data.get("success") is not None and "expires_at" in data:
                            await self._handle_refresh_auth_response(data)
                            continue

                    except json.JSONDecodeError:
                        pass

                # Route to message handler
                if self.message_handler:
                    try:
                        await self.message_handler(message)
                    except Exception as e:
                        logger.error(f"Frame error: {e}")
                        if self.on_error:
                            await self.on_error(e)

        except WebSocketException as e:
            logger.error(f"WebSocket error: {e}")

            # Check if close reason indicates auth error
            if hasattr(e, "reason") and e.reason:
                error_code = parse_error_code_from_close_reason(e.reason)
                if error_code and should_refresh_token(error_code):
                    if self.auth.has_refreshable_auth():
                        logger.info(f"Token error detected, will refresh: {e.reason}")
                        self.auth.clear_token()
                        self._force_token_refresh = True
                        self._immediate_reconnect = True

            if self.on_error:
                await self.on_error(e)

            self._stop_ping()
            self._stop_token_refresh()
            if self.is_running:
                await self.handle_reconnect()

        except Exception as e:
            logger.error(f"Receive error: {e}")
            if self.on_error:
                await self.on_error(e)

    async def handle_reconnect(self) -> None:
        """Handle connection retry with exponential backoff."""
        self.reconnect_attempts += 1

        if self.reconnect_attempts > len(self.reconnect_intervals):
            logger.error("Max reconnect attempts reached")
            return

        delay = (
            0
            if self._immediate_reconnect
            else self.reconnect_intervals[self.reconnect_attempts - 1]
        )

        if delay > 0:
            logger.info(f"Reconnecting in {delay}s (attempt {self.reconnect_attempts})")
            await asyncio.sleep(delay)
        else:
            logger.info(f"Reconnecting immediately (attempt {self.reconnect_attempts})")

        await self.connect()
