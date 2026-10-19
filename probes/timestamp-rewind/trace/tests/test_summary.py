from datetime import UTC, datetime

import pytest

from rewind_trace.summary import (
    Operation,
    Record,
    Store,
    SummaryRow,
    format_row,
    parse_summary,
)

FULL_LINE = (
    "12.5\texplorer.exe\t4321\tread\tCurrent/settings\t1792362613\t60\t"
    "2026-10-19T02:10:03.123456+00:00"
)
FULL_ROW = SummaryRow(
    ms=12.5,
    process="explorer.exe",
    pid=4321,
    operation=Operation.READ,
    store=Store.CURRENT,
    record=Record.SETTINGS,
    stamp=1792362613,
    length=60,
    utc=datetime(2026, 10, 19, 2, 10, 3, 123456, tzinfo=UTC),
)


def test_parses_line_with_every_column() -> None:
    summary = parse_summary([FULL_LINE])
    assert summary.rows == (FULL_ROW,)
    assert summary.has_lengths
    assert summary.has_utc


def test_marks_missing_length_and_utc_columns() -> None:
    summary = parse_summary(["0.0\twin-nightlight-probe.exe\t99\twrite\tCloud/state\t"])
    assert summary.rows == (
        SummaryRow(
            ms=0.0,
            process="win-nightlight-probe.exe",
            pid=99,
            operation=Operation.WRITE,
            store=Store.CLOUD,
            record=Record.STATE,
            stamp=None,
            length=None,
            utc=None,
        ),
    )
    assert not summary.has_lengths
    assert not summary.has_utc


def test_rejects_line_with_too_few_columns() -> None:
    with pytest.raises(ValueError, match="expected at least 6 columns, got 5"):
        parse_summary(["0.0\texplorer.exe\t1\tread\tCurrent/settings"])


def test_rejects_unknown_operation() -> None:
    with pytest.raises(ValueError, match="'delete' is not a valid Operation"):
        parse_summary(["0.0\texplorer.exe\t1\tdelete\tCurrent/settings\t"])


def test_formats_row_as_tab_separated_line() -> None:
    assert format_row(FULL_ROW) == FULL_LINE


def test_formats_missing_values_as_empty_columns() -> None:
    row = SummaryRow(
        ms=3.25,
        process="ShellHost.exe",
        pid=7,
        operation=Operation.WRITE,
        store=Store.CURRENT,
        record=Record.STATE,
        stamp=None,
        length=None,
        utc=None,
    )
    assert format_row(row) == "3.2\tShellHost.exe\t7\twrite\tCurrent/state\t\t\t"
