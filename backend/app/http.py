"""ASGI helpers for reverse-proxy deploys."""

from __future__ import annotations

from starlette.types import ASGIApp, Message, Receive, Scope, Send


class AutoSecureCookieMiddleware:
    """Add the Secure flag to the session cookie when the request is HTTPS.

    SessionMiddleware only has a process-wide https_only switch. Self-hosters
    often hit the same container over LAN HTTP and via a TLS reverse proxy, so
    we decide per request after ProxyHeadersMiddleware has set the scheme.
    """

    def __init__(self, app: ASGIApp, cookie_name: str = "bookclub_session") -> None:
        self.app = app
        self.cookie_name = cookie_name

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return

        prefix = f"{self.cookie_name}=".encode()

        async def send_wrapper(message: Message) -> None:
            if scope.get("scheme") == "https" and message["type"] == "http.response.start":
                headers = []
                for key, value in message.get("headers", []):
                    if key.lower() == b"set-cookie" and value.startswith(prefix):
                        lower = value.lower()
                        if b"; secure" not in lower and not lower.endswith(b"secure"):
                            value = value + b"; Secure"
                    headers.append((key, value))
                message = {**message, "headers": headers}
            await send(message)

        await self.app(scope, receive, send_wrapper)
