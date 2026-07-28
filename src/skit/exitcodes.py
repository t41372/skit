"""Machine-facing process status contracts.

The launched program owns every integer it returns. These constants classify only
skit's own outcomes, and ``exit_code_for_failure`` is the single bridge from a typed
launch failure to that process contract.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Literal, assert_never

EXIT_SUCCESS = 0
EXIT_USAGE = 2
EXIT_SKIT = 125
EXIT_NOT_EXECUTABLE = 126
EXIT_NOT_FOUND = 127
EXIT_ABORTED = 130

# Doctor's dependency-health probe deliberately follows the usual check-command
# convention: 1 means "required but missing". It is not a general skit failure code.
EXIT_DOCTOR_UNHEALTHY = 1

SkitExitCode = Literal[2, 125, 126, 127, 130]


class FailureReason(StrEnum):
    """Why a requested launch never reached the child process."""

    BAD_VALUE = "bad_value"
    DRIFT = "drift"
    MISSING = "missing"
    NOT_EXECUTABLE = "not_executable"
    LAUNCH = "launch"


def exit_code_for_failure(reason: FailureReason) -> Literal[125, 126, 127]:
    """Map the closed launch-refusal model to its Docker-convention status."""
    if reason in (FailureReason.BAD_VALUE, FailureReason.DRIFT, FailureReason.LAUNCH):
        return EXIT_SKIT
    if reason == FailureReason.NOT_EXECUTABLE:
        return EXIT_NOT_EXECUTABLE
    if reason == FailureReason.MISSING:
        return EXIT_NOT_FOUND
    assert_never(reason)
