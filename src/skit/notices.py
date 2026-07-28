"""Typed, presentation-neutral notices produced by parameter edits.

Notices cross the decision/presentation boundary: analyzers, declared-schema editors, and
normalizers decide what happened; the CLI and TUI decide how to word it.  Keeping the code,
subject, and severity in separate fields prevents a parameter name from becoming part of a
mini-protocol (the former ``"code:name"`` strings broke as soon as consumers guessed where a
colon belonged).
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum


class NoticeSeverity(StrEnum):
    """Whether a requested edit happened."""

    INFO = "info"
    REFUSAL = "refusal"


class NoticeCode(StrEnum):
    """The closed notice vocabulary shared by both parameter-definition lanes."""

    NOT_MANAGED = "not-managed"
    UNMANAGE_NOT_MANAGED = "unmanage-not-managed"
    ENV_SOURCE_NOT_MANAGED = "env-source-not-managed"
    ENV_SOURCE_NOT_SECRET = "env-source-not-secret"  # noqa: S105 — outcome id, not a credential
    RESYNC_DROPPED = "resync-dropped"
    ALREADY_MANAGED = "already-managed"
    NOT_A_CANDIDATE = "not-a-candidate"
    RESYNC_SKIPPED = "resync-skipped"
    RESYNC_REBOUND = "resync-rebound"
    NOT_DECLARED = "not-declared"
    RM_NOT_DECLARED = "rm-not-declared"
    ALREADY_DECLARED = "already-declared"
    BAD_DELIVERY = "bad-delivery"
    NOT_A_PLACEHOLDER = "not-a-placeholder"
    BAD_TYPE = "bad-type"
    BAD_DEFAULT = "bad-default"
    CHOICE_WITHOUT_CHOICES = "choice-without-choices"
    BOOL_FLAG_ON_BY_DEFAULT = "bool-flag-on-by-default"


class NormalizeNoticeCode(StrEnum):
    """The separate closed vocabulary for the opt-in source normalizer."""

    NOT_A_CONST = "not-a-const"
    MULTIPLE_ASSIGNMENTS = "multiple-assignments"
    READONLY = "readonly"
    ALREADY_ENV = "already-env"
    UNSAFE_LITERAL = "unsafe-literal"
    SYNTAX_ERROR = "syntax-error"


@dataclass(frozen=True)
class Notice[CodeT: StrEnum]:
    """One decision-layer outcome, without localized presentation text."""

    code: CodeT
    name: str
    severity: NoticeSeverity


EditNotice = Notice[NoticeCode]
NormalizeNotice = Notice[NormalizeNoticeCode]


_REFUSAL_CODES = frozenset(
    {
        NoticeCode.NOT_MANAGED,
        NoticeCode.NOT_A_CANDIDATE,
        NoticeCode.NOT_DECLARED,
        NoticeCode.NOT_A_PLACEHOLDER,
        NoticeCode.BAD_DELIVERY,
        NoticeCode.BAD_TYPE,
        NoticeCode.BAD_DEFAULT,
        NoticeCode.CHOICE_WITHOUT_CHOICES,
        NoticeCode.BOOL_FLAG_ON_BY_DEFAULT,
        NoticeCode.ENV_SOURCE_NOT_MANAGED,
        NoticeCode.ENV_SOURCE_NOT_SECRET,
    }
)


def edit_notice(code: NoticeCode, name: str = "") -> EditNotice:
    """Build an edit notice with the one authoritative severity classification."""
    severity = NoticeSeverity.REFUSAL if code in _REFUSAL_CODES else NoticeSeverity.INFO
    return Notice(code=code, name=name, severity=severity)


def normalize_refusal(code: NormalizeNoticeCode, name: str) -> NormalizeNotice:
    """Build a refusal from the normalizer's separate outcome vocabulary."""
    return Notice(code=code, name=name, severity=NoticeSeverity.REFUSAL)
