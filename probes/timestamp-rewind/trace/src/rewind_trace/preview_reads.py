"""Print what explorer.exe read during each timed preview of a preview-timing run.

Each trial makes three settings writes two seconds apart in timestamp: preview
on with the old temperature, the new temperature with preview still on, and
preview off. Each explorer.exe read of one of them is labeled on/old, on/new,
or off/new and printed in milliseconds relative to the preview-off write.
Consecutive reads of the same value are printed once.
"""

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Final
lazy from collections.abc import Iterable, Sequence

from rewind_trace.summary import (
    PROBE_PROCESS_PREFIX,
    Operation,
    Record,
    Store,
    SummaryRow,
    parse_summary,
)

LABELS: Final = frozendict({-4: "on/old", -2: "on/new", 0: "off/new"})


@dataclass(frozen=True, slots=True, kw_only=True)
class Trial:
    """One timed preview of a preview-timing run.

    Attributes:
        off_stamp: Settings timestamp of the preview-off write.
        heading: The trial's timing and outcome, or None when the run did not
            record an outcome.
    """

    off_stamp: int
    heading: str | None


@dataclass(frozen=True, slots=True, kw_only=True)
class _PreviewOff:
    stamp: int


@dataclass(frozen=True, slots=True, kw_only=True)
class _Outcome:
    heading: str


@dataclass(frozen=True, slots=True, kw_only=True)
class _Write:
    ms: float
    stamp: int
    length: int


def load_trials(lines: Iterable[str]) -> list[Trial]:
    """Return the timed previews in the lines of a preview-timing results file.

    Raises:
        ValueError: A line is not JSON, or a preview-off or outcome record
            lacks a field the report needs.
    """
    events = [_parse_event(line) for line in lines]
    return [
        Trial(off_stamp=event.stamp, heading=_heading(events[index + 1 :]))
        for index, event in enumerate(events)
        if isinstance(event, _PreviewOff)
    ]


def report(trials: Iterable[Trial], rows: Sequence[SummaryRow]) -> list[str]:
    """Return the report lines for `trials`, using the summary rows of their trace.

    Trials are matched to the probe's writes in order, so `trials` must be in
    the order the run made them.
    """
    writes = _probe_writes(rows)
    lines: list[str] = []
    start = 2
    for trial in trials:
        if trial.heading is None:
            lines.append(
                f"trial ending at settings timestamp {trial.off_stamp}: "
                "no outcome recorded",
            )
            continue
        lines.append(trial.heading)
        matched = _match_writes(writes, trial.off_stamp, start)
        if matched is None:
            lines.append("  no matching writes in the trace")
            continue
        first_ms, off_ms, start = matched
        reads = _explorer_reads(rows, trial.off_stamp, first_ms, off_ms)
        listed = ", ".join(f"{label} {offset:+.1f} ms" for label, offset in reads)
        lines.append(f"  explorer.exe reads: {listed or 'none'}")
    return lines


def _parse_event(line: str) -> _PreviewOff | _Outcome | None:
    record = json.loads(line)
    try:
        match record["event"]:
            case "timed_preview_off":
                snapshot = record["data"]["snapshot"]
                return _PreviewOff(stamp=int(snapshot["settings_modified_unix_s"]))
            case "trial_outcome" | "hold_outcome":
                data = record["data"]
                return _Outcome(
                    heading=(
                        f"round {data.get('round', 1)}, "
                        f"{data.get('gap_ms', 0)} ms gap, "
                        f"{data['hold_ms']} ms hold: {data['outcome']}"
                    ),
                )
            case _:
                return None
    except KeyError as error:
        message = f"results record lacks the field {error}"
        raise ValueError(message) from error


def _heading(later: Iterable[_PreviewOff | _Outcome | None]) -> str | None:
    for event in later:
        match event:
            case _PreviewOff():
                return None
            case _Outcome(heading=heading):
                return heading
            case None:
                pass
    return None


def _probe_writes(rows: Iterable[SummaryRow]) -> list[_Write]:
    return [
        _Write(ms=row.ms, stamp=row.stamp, length=row.length)
        for row in rows
        if row.process.startswith(PROBE_PROCESS_PREFIX)
        and row.is_settings_write
        and row.stamp is not None
        and row.length is not None
    ]


def _match_writes(
    writes: list[_Write],
    off: int,
    start: int,
) -> tuple[float, float, int] | None:
    for position in range(start, len(writes)):
        first, middle, last = writes[position - 2 : position + 1]
        # A settings value with preview on is three bytes longer than with
        # preview off.
        on_length = last.length + 3
        if (
            (first.stamp, first.length) == (off - 4, on_length)
            and (middle.stamp, middle.length) == (off - 2, on_length)
            and last.stamp == off
        ):
            return first.ms, last.ms, position + 1
    return None


def _explorer_reads(
    rows: Iterable[SummaryRow],
    off: int,
    first_ms: float,
    off_ms: float,
) -> list[tuple[str, float]]:
    reads: list[tuple[str, float]] = []
    for row in rows:
        # Later writes can reuse these timestamps, so reads more than 20 ms
        # after preview off belong to other steps.
        if (
            row.process != "explorer.exe"
            or row.operation is not Operation.READ
            or row.store is not Store.CURRENT
            or row.record is not Record.SETTINGS
            or row.stamp is None
            or not first_ms <= row.ms <= off_ms + 20.0
        ):
            continue
        label = LABELS.get(row.stamp - off)
        if label is not None and (not reads or reads[-1][0] != label):
            reads.append((label, row.ms - off_ms))
    return reads


def main() -> int:
    """Print the explorer.exe reads of each timed preview in a preview-timing run."""
    parser = argparse.ArgumentParser(
        prog="preview-reads",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("results", type=Path, help="preview-timing results (.jsonl)")
    parser.add_argument("summary", type=Path, help="summary of a trace of that run")
    arguments = parser.parse_args()
    try:
        with arguments.summary.open(newline="") as handle:
            summary = parse_summary(handle)
        trials = load_trials(arguments.results.read_text().splitlines())
    except ValueError as error:
        print(f"preview-reads: {error}", file=sys.stderr)  # noqa: T201 -- command-line error message
        return 1
    if not summary.has_lengths:
        print(  # noqa: T201 -- command-line error message
            f"{arguments.summary} has no value length column; "
            "rerun summarize-trace on its .pml capture",
            file=sys.stderr,
        )
        return 1
    for line in report(trials, summary.rows):
        print(line)  # noqa: T201 -- the report is this command's output
    return 0
