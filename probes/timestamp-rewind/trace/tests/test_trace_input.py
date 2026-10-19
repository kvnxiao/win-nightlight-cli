import json
from datetime import UTC, datetime
lazy from pathlib import Path

import pytest

from rewind_trace.trace_input import load_trace_input, run_window

SUMMARY_LINE = (
    "0.0\texplorer.exe\t4321\twrite\tCurrent/settings\t1000\t57\t"
    "2026-10-19T02:10:03+00:00\n"
)
SUMMARY_LINE_WITHOUT_UTC = "0.0\texplorer.exe\t4321\twrite\tCurrent/settings\t1000\n"
RESULTS = (
    json.dumps({"event": "start", "utc": "2026-10-19T02:10:01.1234567Z"})
    + "\n"
    + json.dumps({"event": "restored", "utc": "2026-10-19T02:10:09Z"})
    + "\n"
)


def write(tmp_path: Path, name: str, text: str) -> str:
    path = tmp_path / name
    path.write_text(text)
    return str(path)


def test_reads_window_from_first_and_last_records() -> None:
    assert run_window(RESULTS.splitlines()) == (
        datetime(2026, 10, 19, 2, 10, 1, 123456, tzinfo=UTC),
        datetime(2026, 10, 19, 2, 10, 9, tzinfo=UTC),
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


def test_loads_summary_and_results_window(tmp_path: Path) -> None:
    trace = load_trace_input(
        "report",
        None,
        [
            write(tmp_path, "trace.tsv", SUMMARY_LINE),
            write(tmp_path, "run.jsonl", RESULTS),
        ],
    )
    assert [row.stamp for row in trace.summary.rows] == [1000]
    assert trace.window == (
        datetime(2026, 10, 19, 2, 10, 1, 123456, tzinfo=UTC),
        datetime(2026, 10, 19, 2, 10, 9, tzinfo=UTC),
    )


def test_loads_summary_without_results(tmp_path: Path) -> None:
    trace = load_trace_input(
        "report",
        None,
        [write(tmp_path, "trace.tsv", SUMMARY_LINE_WITHOUT_UTC)],
    )
    assert trace.window is None
    assert not trace.summary.has_utc


@pytest.mark.parametrize(
    ("summary", "results", "message"),
    [
        pytest.param(
            SUMMARY_LINE,
            "",
            "report: results file has no records",
            id="empty-results",
        ),
        pytest.param(
            "0.0\texplorer.exe\n",
            RESULTS,
            "report: expected at least 6 columns, got 2",
            id="short-summary-line",
        ),
        pytest.param(
            SUMMARY_LINE_WITHOUT_UTC,
            RESULTS,
            "has no UTC column; rerun summarize-trace on its .pml capture",
            id="summary-without-utc",
        ),
    ],
)
def test_exits_with_status_1_on_unusable_input(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    summary: str,
    results: str,
    message: str,
) -> None:
    arguments = [
        write(tmp_path, "trace.tsv", summary),
        write(tmp_path, "run.jsonl", results),
    ]
    with pytest.raises(SystemExit) as caught:
        load_trace_input("report", None, arguments)
    assert caught.value.code == 1
    assert message in capsys.readouterr().err
