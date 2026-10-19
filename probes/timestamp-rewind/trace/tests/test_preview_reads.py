import json

import pytest

from rewind_trace.preview_reads import Trial, load_trials, report
from rewind_trace.summary import Operation, Record, Store, SummaryRow

OFF = 1792370000
OUTCOME = "applied, later ordinary refreshes applied"


def row(
    ms: float,
    process: str,
    operation: Operation,
    stamp: int,
    length: int = 57,
) -> SummaryRow:
    return SummaryRow(
        ms=ms,
        process=process,
        pid=1,
        operation=operation,
        store=Store.CURRENT,
        record=Record.SETTINGS,
        stamp=stamp,
        length=length,
        utc=None,
    )


def probe_write(ms: float, stamp: int, length: int) -> SummaryRow:
    return row(ms, "win-nightlight-probe.exe", Operation.WRITE, stamp, length)


def explorer_read(ms: float, stamp: int) -> SummaryRow:
    return row(ms, "explorer.exe", Operation.READ, stamp)


TRIAL_WRITES = [
    probe_write(100.0, OFF - 4, 60),
    probe_write(100.2, OFF - 2, 60),
    probe_write(100.4, OFF, 57),
]


def results_line(event: str, data: dict[str, object]) -> str:
    return json.dumps({"event": event, "utc": "2026-10-19T02:10:03Z", "data": data})


def test_loads_trials_with_their_outcomes() -> None:
    lines = [
        results_line(
            "timed_preview_off", {"snapshot": {"settings_modified_unix_s": OFF}}
        ),
        results_line("observer", {"step": "timed_preview"}),
        results_line(
            "trial_outcome",
            {"round": 2, "gap_ms": 50, "hold_ms": 0, "outcome": "ignored"},
        ),
        results_line(
            "timed_preview_off", {"snapshot": {"settings_modified_unix_s": 7}}
        ),
        results_line(
            "timed_preview_off", {"snapshot": {"settings_modified_unix_s": 9}}
        ),
        results_line("hold_outcome", {"hold_ms": 5, "outcome": OUTCOME}),
    ]
    assert load_trials(lines) == [
        Trial(off_stamp=OFF, heading="round 2, 50 ms gap, 0 ms hold: ignored"),
        Trial(off_stamp=7, heading=None),
        Trial(off_stamp=9, heading=f"round 1, 0 ms gap, 5 ms hold: {OUTCOME}"),
    ]


def test_rejects_outcome_without_hold() -> None:
    lines = [results_line("trial_outcome", {"outcome": "ignored"})]
    with pytest.raises(ValueError, match="results record lacks the field 'hold_ms'"):
        load_trials(lines)


def test_reports_explorer_reads_relative_to_preview_off() -> None:
    rows = [
        explorer_read(99.0, OFF - 4),
        *TRIAL_WRITES[:2],
        explorer_read(100.3, OFF - 2),
        explorer_read(100.35, OFF - 2),
        TRIAL_WRITES[2],
        explorer_read(100.8, OFF),
        explorer_read(130.0, OFF - 4),
    ]
    trial = Trial(off_stamp=OFF, heading="round 1, 0 ms gap, 0 ms hold: ignored")
    assert report([trial], rows) == [
        "round 1, 0 ms gap, 0 ms hold: ignored",
        "  explorer.exe reads: on/new -0.1 ms, off/new +0.4 ms",
    ]


def test_reports_trial_without_reads() -> None:
    trial = Trial(off_stamp=OFF, heading="round 1, 0 ms gap, 0 ms hold: ignored")
    assert report([trial], TRIAL_WRITES) == [
        "round 1, 0 ms gap, 0 ms hold: ignored",
        "  explorer.exe reads: none",
    ]


def test_reports_trial_without_matching_writes() -> None:
    trial = Trial(off_stamp=OFF + 10, heading="round 1, 0 ms gap, 0 ms hold: ignored")
    assert report([trial], TRIAL_WRITES) == [
        "round 1, 0 ms gap, 0 ms hold: ignored",
        "  no matching writes in the trace",
    ]


def test_reports_trial_without_outcome() -> None:
    assert report([Trial(off_stamp=OFF, heading=None)], TRIAL_WRITES) == [
        f"trial ending at settings timestamp {OFF}: no outcome recorded",
    ]


def test_matches_each_trial_to_later_writes() -> None:
    second_writes = [
        probe_write(300.0, OFF - 4, 60),
        probe_write(300.2, OFF - 2, 60),
        probe_write(300.4, OFF, 57),
    ]
    rows = [*TRIAL_WRITES, *second_writes, explorer_read(300.5, OFF)]
    trials = [
        Trial(off_stamp=OFF, heading="first"),
        Trial(off_stamp=OFF, heading="second"),
    ]
    assert report(trials, rows) == [
        "first",
        "  explorer.exe reads: none",
        "second",
        "  explorer.exe reads: off/new +0.1 ms",
    ]
