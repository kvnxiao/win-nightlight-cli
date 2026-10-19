from dataclasses import replace
from datetime import UTC, datetime

import pytest

from rewind_trace.summary import Operation, Record, Store, SummaryRow
from rewind_trace.write_reads import ReadReport, find_reads, format_report

PROBE = "win-nightlight-timestamp-rewind-probe.exe"


def at(second: int) -> datetime:
    return datetime(2026, 10, 19, 2, 10, second, tzinfo=UTC)


def row(ms: float, process: str, operation: Operation, stamp: int) -> SummaryRow:
    return SummaryRow(
        ms=ms,
        process=process,
        pid=1,
        operation=operation,
        store=Store.CURRENT,
        record=Record.SETTINGS,
        stamp=stamp,
        length=57,
        utc=None,
    )


def probe_write(ms: float, stamp: int, second: int) -> SummaryRow:
    return replace(row(ms, PROBE, Operation.WRITE, stamp), utc=at(second))


ROWS = [
    probe_write(0.0, 100, 1),
    row(0.5, "explorer.exe", Operation.READ, 100),
    probe_write(2.0, 102, 2),
    row(2.5, "ShellHost.exe", Operation.READ, 102),
    replace(row(3.0, "explorer.exe", Operation.READ, 102), store=Store.CLOUD),
    probe_write(4.0, 104, 3),
    row(4.5, "explorer.exe", Operation.WRITE, 90),
    row(5.25, "Explorer.EXE", Operation.READ, 104),
    row(6.0, "explorer.exe", Operation.READ, 104),
]


def test_matches_first_explorer_read_of_each_probe_write() -> None:
    assert find_reads(ROWS, None).lags == pytest.approx((0.5, None, 1.25))


def test_other_write_with_same_timestamp_ends_the_match() -> None:
    rows = [
        probe_write(0.0, 100, 1),
        row(0.5, "SystemSettings.exe", Operation.WRITE, 100),
        row(1.0, "explorer.exe", Operation.READ, 100),
    ]
    assert find_reads(rows, None).lags == (None,)


def test_counts_only_writes_inside_window() -> None:
    assert find_reads(ROWS, (at(2), at(3))).lags == pytest.approx((None, 1.25))


def test_formats_counts_delays_and_longest_unread_run() -> None:
    report = ReadReport(lags=(0.5, None, None, 1.5, 2.5))
    assert format_report(report) == [
        "probe settings writes: 5",
        "read by explorer.exe: 3",
        "delay to first read: median 1.5 ms, 90th percentile 2.3 ms, max 2.5 ms",
        "longest run of unread writes: 2",
    ]


def test_formats_report_without_writes() -> None:
    assert format_report(ReadReport(lags=())) == [
        "probe settings writes: 0",
        "read by explorer.exe: 0",
        "longest run of unread writes: 0",
    ]
