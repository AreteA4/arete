class AreteError(Exception):
    """Base exception for all Arete errors"""

    pass


class ConnectionError(AreteError):
    """WebSocket connection issues"""

    pass


class SubscriptionError(AreteError):
    """Subscription setup/management failures"""

    pass


class ParseError(AreteError):
    """Entity parsing failures"""

    pass


class TimeoutError(AreteError):
    """Operation timeouts"""

    pass


class AuthError(AreteError):
    """Authentication failures with optional error code"""

    def __init__(self, message: str, code=None, details=None):
        super().__init__(message)
        self.code = code
        self.details = details

    def __str__(self):
        if self.code:
            return f"[{self.code.value if hasattr(self.code, 'value') else self.code}] {super().__str__()}"
        return super().__str__()
