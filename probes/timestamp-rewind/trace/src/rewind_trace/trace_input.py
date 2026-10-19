"""Load the trace summary and the results window that the report commands take."""

import argparse
import json
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
lazy from collections.abc import Sequence

from rewind_trace.summary import Summary, SummaryRow, parse_summary


@dataclass(frozen=True, slots=True, kw_only=True)
class TraceInput:
    """A trace summary and the run whose writes a report counts.

    Attributes:
        summary: The parsed trace summary.
        window: Times of the first and last records of the run's results file,
            or None when the command got no results file.
    """

    summary: Summary
    window: tuple[datetime, datetime] | None


def load_trace_input(
    prog: str,
    description: str | None,
    argv: Sequence[str] | None = None,
) -> TraceInput:
    """Parse a report command's arguments and load its summary and results window.

    Args:
        prog: Command name for the usage and error messages.
        description: Help text of the command.
        argv: Arguments to parse instead of the process's command line.

    Raises:
        SystemExit: With status 2 if the arguments are invalid. With status 1
            if the summary or results file cannot be parsed, or if a results
            file comes with a summary that has no UTC column.
    """
    parser = argparse.ArgumentParser(
        prog=prog,
        description=description,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("summary", type=Path, help="trace summary (.tsv)")
    parser.add_argument(
        "results",
        type=Path,
        nargs="?",
        help="results file (.jsonl) of the run whose writes to count",
    )
    arguments = parser.parse_args(argv)
    try:
        with arguments.summary.open(newline="") as handle:
            summary = parse_summary(handle)
        window = None
        if arguments.results is not None:
            window = run_window(arguments.results.read_text().splitlines())
    except ValueError as error:
        parser.exit(1, f"{prog}: {error}\n")
    if window is not None and not summary.has_utc:
        parser.exit(
            1,
            f"{arguments.summary} has no UTC column; "
            "rerun summarize-trace on its .pml capture\n",
        )
    return TraceInput(summary=summary, window=window)


def in_window(row: SummaryRow, window: tuple[datetime, datetime] | None) -> bool:
    """Return whether `row` happened inside `window`, or True without a window.

    A row without a UTC time is outside every window.
    """
    return window is None or (row.utc is not None and window[0] <= row.utc <= window[1])


def run_window(lines: Sequence[str]) -> tuple[datetime, datetime]:
    """Return the times of the first and last records in a results file's lines.

    Raises:
        ValueError: There are no lines, or the first or last line is not a
            JSON record with a `utc` time.
    """
    if not lines:
        message = "results file has no records"
        raise ValueError(message)
    return _record_time(lines[0]), _record_time(lines[-1])


def _record_time(line: str) -> datetime:
    record = json.loads(line)
    try:
        return datetime.fromisoformat(record["utc"])
    except KeyError as error:
        message = "results record lacks the field 'utc'"
        raise ValueError(message) from error
