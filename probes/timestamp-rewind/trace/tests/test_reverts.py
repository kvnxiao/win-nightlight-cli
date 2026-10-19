import json
from datetime import UTC, datetime

import pytest

from rewind_trace.reverts import RevertReport, find_reverts, format_report, run_window
from rewind_trace.summary import Operation, Record, Store, SummaryRow

CLOUD = 1000
PROBE = "win-nightlight-probe.exe"


def at(second: int) -> datetime:
    return datetime(2026, 10, 19, 2, 10, second, tzinfo=UTC)


def cloud_read(ms: float, stamp: int) -> SummaryRow:
    return SummaryRow(
        ms=ms,
        process="explorer.exe",
        pid=1,
        operation=Operation.READ,
        store=Store.CLOUD,
        record=Record.SETTINGS,
        stamp=stamp,
        length=13,
        utc=None,
    )


def settings_write(
    ms: float,
    process: str,
    stamp: int,
    utc: datetime | None = None,
) -> SummaryRow:
    return SummaryRow(
        ms=ms,
        process=process,
        pid=1,
        operation=Operation.WRITE,
        store=Store.CURRENT,
        record=Record.SETTINGS,
        stamp=stamp,
        length=57,
        utc=utc,
    )


ROWS = [
    cloud_read(0.0, CLOUD),
    settings_write(10.0, PROBE, CLOUD - 1, at(1)),
    settings_write(12.5, "ShellHost.exe", CLOUD),
    settings_write(13.5, "explorer.exe", CLOUD),
    settings_write(14.0, "explorer.exe", CLOUD),
    settings_write(20.0, PROBE, CLOUD + 1, at(2)),
    settings_write(30.0, PROBE, CLOUD - 1, at(3)),
    settings_write(75.0, "explorer.exe", CLOUD),
]


def test_finds_first_revert_by_each_process() -> None:
    report = find_reverts(ROWS, None)
    assert report.below_cloud == 2
    assert report.delays == {
        "ShellHost.exe": pytest.approx((2.5,)),
        "explorer.exe": pytest.approx((3.5, 45.0)),
    }


def test_counts_only_writes_inside_window() -> None:
    report = find_reverts(ROWS, (at(3), at(4)))
    assert report.below_cloud == 1
    assert report.delays == {"explorer.exe": pytest.approx((45.0,))}


def test_ignores_writes_before_cloud_is_known() -> None:
    report = find_reverts(ROWS[1:], None)
    assert report.below_cloud == 0
    assert report.delays == {}


def test_reads_window_from_first_and_last_records() -> None:
    lines = [
        json.dumps({"event": "start", "utc": "2026-10-19T02:10:01.1234567Z"}),
        json.dumps({"event": "restored", "utc": "2026-10-19T02:10:09Z"}),
    ]
    assert run_window(lines) == (
        datetime(2026, 10, 19, 2, 10, 1, 123456, tzinfo=UTC),
        at(9),
    )


@pytest.mark.parametrize(
    ("lines", "message"),
    [
        pytest.param([], "results file has no records", id="empty"),
        pytest.param(
            [json.dumps({"event": "sample"})],
            "results record lacks the field 'utc'",
            id="no-utc",
        ),
    ],
)
def test_rejects_results_without_times(lines: list[str], message: str) -> None:
    with pytest.raises(ValueError, match=message):
        run_window(lines)


def test_formats_report_with_late_reverts() -> None:
    report = RevertReport(
        below_cloud=3,
        delays=frozendict({"explorer.exe": (2.0, 3.0, 44.25), "ShellHost.exe": (2.5,)}),
    )
    assert format_report(report) == [
        "probe writes below Cloud: 3",
        (
            "ShellHost.exe: reverted 1, median 2.5 ms, max 2.5 ms, "
            "later than 10 ms: 0 []"
        ),
        (
            "explorer.exe: reverted 3, median 3.0 ms, max 44.2 ms, "
            "later than 10 ms: 1 [44.2]"
        ),
    ]
