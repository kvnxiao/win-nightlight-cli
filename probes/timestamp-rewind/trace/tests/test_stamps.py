from dataclasses import replace
from datetime import UTC, datetime

import pytest

from rewind_trace.stamps import (
    SliderWrite,
    StampReport,
    StampSource,
    StateWrite,
    find_stamps,
    format_report,
)
from rewind_trace.summary import Operation, Record, Store, SummaryRow

PROBE = "win-nightlight-probe.exe"
SLIDER = "SystemSettings.exe"
PREVIEW_ON = 60
PREVIEW_OFF = 57


def at(second: int) -> datetime:
    return datetime(2026, 10, 19, 7, 10, second, tzinfo=UTC)


def write(ms: float, process: str, stamp: int) -> SummaryRow:
    return SummaryRow(
        ms=ms,
        process=process,
        pid=1,
        operation=Operation.WRITE,
        store=Store.CURRENT,
        record=Record.SETTINGS,
        stamp=stamp,
        length=PREVIEW_ON,
        utc=None,
    )


def read(ms: float, process: str, stamp: int) -> SummaryRow:
    return replace(write(ms, process, stamp), operation=Operation.READ)


def test_counts_writes_at_or_below_the_highest_earlier_timestamp() -> None:
    rows = [
        write(0.0, PROBE, 100),
        write(1.0, PROBE, 102),
        write(2.0, SLIDER, 100),
        write(3.0, PROBE, 104),
        write(4.0, SLIDER, 103),
        write(5.0, PROBE, 106),
        replace(write(6.0, PROBE, 50), store=Store.CLOUD),
        replace(write(7.0, PROBE, 10), record=Record.STATE),
    ]
    assert find_stamps(rows, None).writes == {
        (PROBE, Record.SETTINGS): (4, 0),
        (SLIDER, Record.SETTINGS): (2, 2),
        (PROBE, Record.STATE): (1, 0),
    }


def test_classifies_what_each_slider_write_is_stamped_from() -> None:
    rows = [
        read(0.0, SLIDER, 100),
        write(10.0, SLIDER, 102),
        read(11.0, SLIDER, 102),
        write(20.0, SLIDER, 104),
        write(22.0, PROBE, 110),
        read(25.0, SLIDER, 110),
        write(30.0, SLIDER, 106),
        read(40.0, SLIDER, 120),
        read(50.0, SLIDER, 130),
        write(60.0, SLIDER, 122),
        write(70.0, SLIDER, 200),
    ]
    writes = find_stamps(rows, None).slider_writes
    assert [(write.source, write.read_delay_ms) for write in writes] == [
        (StampSource.LATEST_READ, pytest.approx(10.0)),
        (StampSource.OWN_WRITE_AND_LATEST_READ, pytest.approx(9.0)),
        (StampSource.OWN_WRITE, None),
        (StampSource.EARLIER_READ, pytest.approx(20.0)),
        (StampSource.NONE, None),
    ]


@pytest.mark.parametrize(
    ("lengths", "expected"),
    [
        pytest.param((PREVIEW_ON, PREVIEW_OFF), [True, False], id="both-lengths"),
        pytest.param((PREVIEW_ON, PREVIEW_ON), [None, None], id="one-length"),
    ],
)
def test_reads_the_preview_flag_from_value_lengths(
    lengths: tuple[int, int],
    expected: list[bool | None],
) -> None:
    rows = [
        replace(write(0.0, SLIDER, 100), length=lengths[0]),
        replace(write(1.0, SLIDER, 102), length=lengths[1]),
    ]
    assert [
        write.previewing for write in find_stamps(rows, None).slider_writes
    ] == expected


def test_lists_state_writes_with_their_step_from_the_stored_state() -> None:
    rows = [
        replace(write(0.0, PROBE, 400), record=Record.STATE),
        replace(read(1.0, "explorer.exe", 500), record=Record.STATE),
        replace(write(2.0, "explorer.exe", 502), record=Record.STATE),
        replace(
            write(3.0, "explorer.exe", 502), store=Store.CLOUD, record=Record.STATE
        ),
        replace(write(4.0, PROBE, 510), record=Record.STATE),
    ]
    assert find_stamps(rows, None).state_writes == (
        StateWrite(ms=0.0, process=PROBE, store=Store.CURRENT, stamp=400, step=None),
        StateWrite(
            ms=2.0, process="explorer.exe", store=Store.CURRENT, stamp=502, step=2
        ),
        StateWrite(
            ms=3.0, process="explorer.exe", store=Store.CLOUD, stamp=502, step=0
        ),
        StateWrite(ms=4.0, process=PROBE, store=Store.CURRENT, stamp=510, step=8),
    )


def test_reports_only_writes_inside_the_window() -> None:
    rows = [
        replace(write(0.0, PROBE, 110), utc=at(1)),
        replace(read(1.0, SLIDER, 110), utc=at(1)),
        replace(write(2.0, SLIDER, 112), utc=at(3)),
        replace(write(3.0, PROBE, 100), record=Record.STATE, utc=at(3)),
        replace(write(4.0, PROBE, 104), utc=at(5)),
    ]
    report = find_stamps(rows, (at(2), at(4)))
    assert report.writes == {
        (SLIDER, Record.SETTINGS): (1, 0),
        (PROBE, Record.STATE): (1, 0),
    }
    assert [write.source for write in report.slider_writes] == [StampSource.LATEST_READ]
    assert [write.stamp for write in report.state_writes] == [100]


def test_formats_report() -> None:
    report = StampReport(
        writes=frozendict(
            {(SLIDER, Record.SETTINGS): (2, 2), (PROBE, Record.SETTINGS): (3, 1)},
        ),
        slider_writes=(
            SliderWrite(
                ms=10.0,
                stamp=102,
                source=StampSource.LATEST_READ,
                read_delay_ms=58.25,
                previewing=True,
            ),
            SliderWrite(
                ms=20.0,
                stamp=104,
                source=StampSource.OWN_WRITE_AND_LATEST_READ,
                read_delay_ms=1.0,
                previewing=False,
            ),
        ),
        state_writes=(
            StateWrite(
                ms=30.0, process="explorer.exe", store=Store.CLOUD, stamp=502, step=0
            ),
        ),
    )
    assert format_report(report) == [
        (
            "SystemSettings.exe Current/settings writes: 2, "
            "at or below the highest earlier timestamp: 2"
        ),
        (
            "win-nightlight-probe.exe Current/settings writes: 3, "
            "at or below the highest earlier timestamp: 1"
        ),
        "SystemSettings.exe settings writes: 2",
        "  stamped 2 s after its previous write and the latest value it read: 1",
        "  stamped 2 s after its previous write only: 0",
        "  stamped 2 s after the latest value it read only: 1",
        "  stamped 2 s after an earlier value it read: 0",
        "  stamped 2 s after none of these: 0",
        (
            "  longest delay from the read to the write, for writes stamped only "
            "from a value it read: 58.2 ms"
        ),
        "  with preview off: 1, at ms: 20.0",
        "state writes: 1",
        "  30.0 ms explorer.exe Cloud/state 502, +0 s from the stored state timestamp",
    ]
