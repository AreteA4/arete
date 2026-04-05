class HyperStackError(Exception):
    """Base exception for all HyperStack errors"""

    pass


class ConnectionError(HyperStackError):
    """WebSocket connection issues"""

    pass


class SubscriptionError(HyperStackError):
    """Subscription setup/management failures"""

    pass


class ParseError(HyperStackError):
    """Entity parsing failures"""

    pass


class TimeoutError(HyperStackError):
    """Operation timeouts"""

    pass


class AuthError(HyperStackError):
    """Authentication failures with optional error code"""

    def __init__(self, message: str, code=None, details=None):
        super().__init__(message)
        self.code = code
        self.details = details

    def __str__(self):
        if self.code:
            return f"[{self.code.value if hasattr(self.code, 'value') else self.code}] {super().__str__()}"
        return super().__str__()
