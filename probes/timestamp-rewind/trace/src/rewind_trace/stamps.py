"""Print how each process stamped its Night Light writes in a trace.

For each process and value, the report counts the `Current` writes stamped at
or below the highest timestamp written to that value earlier in the trace.
For each settings write by Windows Settings (SystemSettings.exe), it reports
which earlier value the timestamp is 2 s after: its own previous write, the
latest value it read, or an earlier value it read. For a write stamped from a
value it read, it gives how long before the write it last read that value.
It also lists the Windows Settings writes with preview off, and each write of
the state value and its Cloud record with its step from the stored state
timestamp. With a results file, only writes made during that run count.
"""

from dataclasses import dataclass
from enum import StrEnum
from typing import Final
lazy from collections.abc import Iterable, Sequence
lazy from datetime import datetime

from rewind_trace.summary import Operation, Record, Store, SummaryRow
from rewind_trace.trace_input import in_window, load_trace_input

SLIDER_WRITER: Final = "SystemSettings.exe"
STAMP_STEP: Final = 2
# A settings value with preview on is three bytes longer than the same value
# with preview off.
PREVIEW_BYTES: Final = 3


class StampSource(StrEnum):
    """Earlier value that a Windows Settings write is stamped 2 s after."""

    OWN_WRITE_AND_LATEST_READ = "its previous write and the latest value it read"
    OWN_WRITE = "its previous write only"
    LATEST_READ = "the latest value it read only"
    EARLIER_READ = "an earlier value it read"
    NONE = "none of these"


@dataclass(frozen=True, slots=True, kw_only=True)
class SliderWrite:
    """A settings write by Windows Settings.

    Attributes:
        ms: Milliseconds since the first event of the summary.
        stamp: Timestamp of the write.
        source: Earlier value the timestamp is 2 s after.
        read_delay_ms: Milliseconds from Windows Settings' latest read of the
            value 2 s below the timestamp to the write, or None when it did not
            read that value.
        previewing: Whether the write has preview on, or None when the value
            lengths in the summary do not show it.
    """

    ms: float
    stamp: int
    source: StampSource
    read_delay_ms: float | None
    previewing: bool | None


@dataclass(frozen=True, slots=True, kw_only=True)
class StateWrite:
    """A write of the state value or of its Cloud record.

    Attributes:
        ms: Milliseconds since the first event of the summary.
        process: Name of the process that wrote the value.
        store: CloudStore subtree of the value.
        stamp: Timestamp of the write.
        step: Timestamp minus the latest `Current` state timestamp before the
            write, or None when the trace has no earlier one.
    """

    ms: float
    process: str
    store: Store
    stamp: int
    step: int | None


@dataclass(frozen=True, slots=True, kw_only=True)
class StampReport:
    """How the writes in a trace were stamped.

    Attributes:
        writes: For each process and record, the number of `Current` writes and
            how many of them were stamped at or below the highest timestamp
            written to that record earlier in the trace.
        slider_writes: Settings writes by Windows Settings, in trace order.
        state_writes: Writes of the state value and its Cloud record, in trace
            order.
    """

    writes: frozendict[tuple[str, Record], tuple[int, int]]
    slider_writes: tuple[SliderWrite, ...]
    state_writes: tuple[StateWrite, ...]


def find_stamps(
    rows: Sequence[SummaryRow],
    window: tuple[datetime, datetime] | None,
) -> StampReport:
    """Find how the writes in a trace were stamped.

    Args:
        rows: Summary rows of a trace, in trace order.
        window: When given, report only writes made between these times. Rows
            without a UTC time are then outside it. Earlier rows still set the
            highest earlier timestamp and the values Windows Settings read.
    """
    return StampReport(
        writes=_count_writes(rows, window),
        slider_writes=_slider_writes(rows, window, _preview_lengths(rows)),
        state_writes=_state_writes(rows, window),
    )


def format_report(report: StampReport) -> list[str]:
    """Return the report lines."""
    lines = [
        f"{process} {Store.CURRENT}/{record} writes: {total}, "
        f"at or below the highest earlier timestamp: {at_or_below}"
        for (process, record), (total, at_or_below) in sorted(report.writes.items())
    ]
    slider = report.slider_writes
    lines.append(f"{SLIDER_WRITER} settings writes: {len(slider)}")
    lines.extend(
        f"  stamped 2 s after {source}: "
        f"{sum(1 for write in slider if write.source is source)}"
        for source in StampSource
    )
    delays = [
        write.read_delay_ms
        for write in slider
        if write.source in {StampSource.LATEST_READ, StampSource.EARLIER_READ}
        and write.read_delay_ms is not None
    ]
    if delays:
        lines.append(
            "  longest delay from the read to the write, for writes stamped "
            f"only from a value it read: {max(delays):.1f} ms",
        )
    if any(write.previewing is None for write in slider):
        lines.append("  preview flag: the value lengths in the summary do not show it")
    else:
        off = [f"{write.ms:.1f}" for write in slider if write.previewing is False]
        lines.append(
            f"  with preview off: {len(off)}, at ms: {', '.join(off) or 'none'}"
        )
    lines.append(f"state writes: {len(report.state_writes)}")
    lines.extend(_state_line(write) for write in report.state_writes)
    return lines


def main() -> int:
    """Print how each process stamped its Night Light writes in a trace."""
    trace = load_trace_input("stamps", __doc__)
    for line in format_report(find_stamps(trace.summary.rows, trace.window)):
        print(line)  # noqa: T201 -- the report is this command's output
    return 0


def _count_writes(
    rows: Iterable[SummaryRow],
    window: tuple[datetime, datetime] | None,
) -> frozendict[tuple[str, Record], tuple[int, int]]:
    highest: dict[Record, int] = {}
    counts: dict[tuple[str, Record], tuple[int, int]] = {}
    for row in rows:
        if (
            row.operation is not Operation.WRITE
            or row.store is not Store.CURRENT
            or row.stamp is None
        ):
            continue
        earlier = highest.get(row.record)
        if in_window(row, window):
            total, at_or_below = counts.get((row.process, row.record), (0, 0))
            low = earlier is not None and row.stamp <= earlier
            counts[row.process, row.record] = (total + 1, at_or_below + int(low))
        highest[row.record] = row.stamp if earlier is None else max(earlier, row.stamp)
    return frozendict(counts)


def _slider_writes(
    rows: Iterable[SummaryRow],
    window: tuple[datetime, datetime] | None,
    preview_lengths: tuple[int, int] | None,
) -> tuple[SliderWrite, ...]:
    last_read_ms: dict[int, float] = {}
    latest_read: int | None = None
    previous: int | None = None
    writes: list[SliderWrite] = []
    for row in rows:
        if (
            row.process != SLIDER_WRITER
            or row.store is not Store.CURRENT
            or row.record is not Record.SETTINGS
            or row.stamp is None
        ):
            continue
        if row.operation is Operation.READ:
            last_read_ms[row.stamp] = row.ms
            latest_read = row.stamp
            continue
        if in_window(row, window):
            source_stamp = row.stamp - STAMP_STEP
            read_ms = last_read_ms.get(source_stamp)
            writes.append(
                SliderWrite(
                    ms=row.ms,
                    stamp=row.stamp,
                    source=_source(
                        from_own_write=previous == source_stamp,
                        from_latest_read=latest_read == source_stamp,
                        from_earlier_read=read_ms is not None,
                    ),
                    read_delay_ms=None if read_ms is None else row.ms - read_ms,
                    previewing=_previewing(row.length, preview_lengths),
                ),
            )
        previous = row.stamp
    return tuple(writes)


def _source(
    *,
    from_own_write: bool,
    from_latest_read: bool,
    from_earlier_read: bool,
) -> StampSource:
    if from_own_write and from_latest_read:
        return StampSource.OWN_WRITE_AND_LATEST_READ
    if from_latest_read:
        return StampSource.LATEST_READ
    if from_own_write:
        return StampSource.OWN_WRITE
    if from_earlier_read:
        return StampSource.EARLIER_READ
    return StampSource.NONE


def _preview_lengths(rows: Iterable[SummaryRow]) -> tuple[int, int] | None:
    lengths = {
        row.length
        for row in rows
        if row.store is Store.CURRENT
        and row.record is Record.SETTINGS
        and row.length is not None
    }
    longest = max(lengths, default=None)
    if longest is None or longest - PREVIEW_BYTES not in lengths:
        return None
    return longest - PREVIEW_BYTES, longest


def _previewing(
    length: int | None, preview_lengths: tuple[int, int] | None
) -> bool | None:
    if preview_lengths is None or length not in preview_lengths:
        return None
    return length == preview_lengths[1]


def _state_line(write: StateWrite) -> str:
    value = f"{write.store}/{Record.STATE}"
    line = f"  {write.ms:.1f} ms {write.process} {value} {write.stamp}"
    if write.step is None:
        return line
    return f"{line}, {write.step:+d} s from the stored state timestamp"


def _state_writes(
    rows: Iterable[SummaryRow],
    window: tuple[datetime, datetime] | None,
) -> tuple[StateWrite, ...]:
    stored: int | None = None
    writes: list[StateWrite] = []
    for row in rows:
        if row.record is not Record.STATE or row.stamp is None:
            continue
        if row.operation is Operation.WRITE and in_window(row, window):
            writes.append(
                StateWrite(
                    ms=row.ms,
                    process=row.process,
                    store=row.store,
                    stamp=row.stamp,
                    step=None if stored is None else row.stamp - stored,
                ),
            )
        if row.store is Store.CURRENT:
            stored = row.stamp
    return tuple(writes)
