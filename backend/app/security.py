import re
import secrets
from collections import defaultdict
from time import monotonic

from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerificationError, VerifyMismatchError

_hasher = PasswordHasher()
_CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
_FAIL_WINDOW = 600.0
_FAIL_LIMIT = 10
_failures: dict[str, list[float]] = defaultdict(list)


def hash_password(password: str) -> str:
    return _hasher.hash(password)


def verify_password(password: str, password_hash: str) -> bool:
    try:
        return _hasher.verify(password_hash, password)
    except (VerifyMismatchError, VerificationError, InvalidHashError):
        return False


def generate_invite_code(length: int = 10) -> str:
    return "".join(secrets.choice(_CROCKFORD) for _ in range(length))


def normalize_invite_code(value: str) -> str:
    return re.sub(r"[\s-]", "", value).upper()


def too_many_failures(username: str) -> bool:
    now = monotonic()
    recent = [stamp for stamp in _failures[username] if now - stamp < _FAIL_WINDOW]
    _failures[username] = recent
    return len(recent) >= _FAIL_LIMIT


def record_failure(username: str) -> None:
    _failures[username].append(monotonic())


def clear_failures(username: str) -> None:
    _failures.pop(username, None)


def reset_rate_limits() -> None:
    _failures.clear()
