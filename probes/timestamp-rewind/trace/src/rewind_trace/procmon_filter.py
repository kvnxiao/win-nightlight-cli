"""Write a Process Monitor configuration that keeps only CloudStore registry events."""

import argparse
from pathlib import Path

from rewind_trace.procmon import write_cloudstore_filter


def main() -> int:
    """Write the configuration to the path given on the command line."""
    parser = argparse.ArgumentParser(prog="procmon-filter", description=__doc__)
    parser.add_argument("output", type=Path, help="configuration file to write (.pmc)")
    arguments = parser.parse_args()
    with arguments.output.open("wb") as config:
        write_cloudstore_filter(config)
    return 0
