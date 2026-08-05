"""Closed contracts for typed parameter-edit and normalization notices."""

from __future__ import annotations

from typing import cast

import pytest

from skit import analysis, cli
from skit.notices import (
    NormalizeNoticeCode,
    Notice,
    NoticeCode,
    NoticeSeverity,
    edit_notice,
    normalize_refusal,
)


def test_edit_notice_classifies_every_code() -> None:
    refusal_codes = {
        NoticeCode.NOT_MANAGED,
        NoticeCode.ENV_SOURCE_NOT_MANAGED,
        NoticeCode.ENV_SOURCE_NOT_SECRET,
        NoticeCode.NOT_A_CANDIDATE,
        NoticeCode.NOT_DECLARED,
        NoticeCode.BAD_DELIVERY,
        NoticeCode.NOT_A_PLACEHOLDER,
        NoticeCode.BAD_TYPE,
        NoticeCode.BAD_DEFAULT,
        NoticeCode.CHOICE_WITHOUT_CHOICES,
        NoticeCode.BOOL_FLAG_ON_BY_DEFAULT,
    }

    actual_refusals = {
        code
        for code in NoticeCode
        if edit_notice(code, "subject").severity is NoticeSeverity.REFUSAL
    }

    assert actual_refusals == refusal_codes
    for code in NoticeCode:
        notice = edit_notice(code, "na:me")
        assert notice.code is code
        assert notice.name == "na:me"
        assert notice.severity is (
            NoticeSeverity.REFUSAL if code in refusal_codes else NoticeSeverity.INFO
        )


def test_normalize_refusal_preserves_typed_code_and_subject() -> None:
    for code in NormalizeNoticeCode:
        notice = normalize_refusal(code, "na:me")
        assert notice.code is code
        assert notice.name == "na:me"
        assert notice.severity is NoticeSeverity.REFUSAL


def test_edit_renderer_rejects_a_code_outside_its_closed_vocabulary() -> None:
    invalid = Notice(
        code=cast(NoticeCode, "future-code"),
        name="subject",
        severity=NoticeSeverity.INFO,
    )

    with pytest.raises(AssertionError):
        analysis.render_notice(invalid)


def test_normalize_renderer_rejects_a_code_outside_its_closed_vocabulary() -> None:
    invalid = Notice(
        code=cast(NormalizeNoticeCode, "future-code"),
        name="subject",
        severity=NoticeSeverity.REFUSAL,
    )

    with pytest.raises(AssertionError):
        cli._render_normalize_notice(invalid)
