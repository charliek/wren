"""Fail closed unless every required GitHub Actions job succeeded."""

import json
import os
import sys


def all_succeeded(raw: str | None) -> bool:
    try:
        needs = json.loads(raw or "")
    except (TypeError, ValueError):
        return False
    return (
        isinstance(needs, dict)
        and bool(needs)
        and all(
            isinstance(job, dict) and job.get("result") == "success"
            for job in needs.values()
        )
    )


def main() -> int:
    if not all_succeeded(os.environ.get("NEEDS")):
        print("Required CI jobs did not all succeed, or NEEDS is invalid.", file=sys.stderr)
        return 1
    print("All required CI jobs succeeded.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
