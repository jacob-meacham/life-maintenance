# Life Maintenance Tracker — Phase 1 (Core + CLI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `lm` CLI and its pure scheduling engine over git-backed YAML/JSONL files, so recurring maintenance tasks can be tracked, completed, searched, and reported on from the terminal.

**Architecture:** A pure computation engine (`schedule.py` → `status.py`) with no I/O, wrapped by a DAO (`_store.py`) that reads/writes the git files, a controller (`service.py`) that orchestrates them, and a Typer CLI (`cli.py`) as the transport layer. Boundary data (the YAML/JSONL files) is validated with Pydantic v2, then converted to frozen domain dataclasses that flow internally. Money is integer cents throughout.

**Tech Stack:** Python 3.12, `uv` (package manager/runner), Typer (CLI), Pydantic v2 (file-boundary validation), PyYAML, python-dateutil (`relativedelta` for calendar math), pytest (+ parametrize), ruff, pyright (strict).

**Constitution:** Follows `agent-instructions/coding/constitution/general.md` + `python.md`. Key rules in force: `uv` only (no pip/venv); `src/` layout; full type annotations, no `Any`, pyright strict; `@dataclass(frozen=True, slots=True)` for domain models, Pydantic at the file boundary; per-module exception hierarchy, no bare `except`, `raise ... from`; pytest with exact assertions (no `assertNotNull`-style terminal asserts); ruff families `E,W,F,I,B,UP,SIM,RUF,TID,PL,TCH` at line length 100; `from __future__ import annotations` atop every module; no `print()` in library code; functions < 50 lines, files < 500.

**Data directory:** All commands operate on a *data dir* (the git repo that holds `tasks.yaml`, `vendors.yaml`, `completions.jsonl`). Resolved as: `--data-dir` flag → `LM_DATA_DIR` env var → default `Path.cwd() / "data"`. Completions are committed into that repo.

---

## File Structure

```
life-maintenance/
├── pyproject.toml                 # project + tool config (ruff, pyright, pytest)
├── .python-version                # 3.12
├── .gitignore                     # (already exists; add caches)
├── src/
│   └── lifemaint/
│       ├── __init__.py            # package marker + __all__ for public surface
│       ├── errors.py              # exception hierarchy
│       ├── schedule.py            # Relative + Fixed schedules; parse_schedule / parse_interval
│       ├── _schema.py             # Pydantic raw boundary models (RawTask/RawVendor/RawCompletion)
│       ├── models.py              # frozen domain dataclasses (Task/Vendor/Completion) + from_raw
│       ├── status.py              # PURE engine: compute_status(tasks, completions, today)
│       ├── _store.py              # DAO: load files, append completion, git commit
│       ├── service.py             # controller: list/due/done/history/vendors/export/report
│       └── cli.py                 # transport: Typer app, all commands support --json
├── tests/
│   ├── conftest.py                # fixtures: tmp data dir (git-initialised), sample data
│   ├── test_schedule.py
│   ├── test_schema.py
│   ├── test_models.py
│   ├── test_status.py
│   ├── test_store.py
│   ├── test_service.py
│   └── test_cli.py
└── data/                          # example/live data (created in final task)
    ├── tasks.yaml
    ├── vendors.yaml
    └── completions.jsonl
```

**Responsibility boundaries (maps to the 3-layer rule):**
- `cli.py` (transport) — arg parsing, `--json` serialisation, exit codes. Calls `service` only.
- `service.py` (controller) — orchestration; knows nothing about argparse or the terminal.
- `_store.py` (DAO) — file + git I/O only; no business logic.
- `schedule.py` + `status.py` — pure functions, no I/O, the testable brain.

---

## Task 1: Project scaffolding

**Files:**
- Create: `pyproject.toml`, `.python-version`, `src/lifemaint/__init__.py`, `tests/conftest.py`, `tests/test_smoke.py`

- [ ] **Step 1: Pin Python and init the project layout**

Run:
```bash
echo "3.12" > .python-version
mkdir -p src/lifemaint tests
```

- [ ] **Step 2: Create `pyproject.toml`**

Create `pyproject.toml`:
```toml
[project]
name = "lifemaint"
version = "0.1.0"
description = "Track and complete recurring home/life maintenance tasks."
requires-python = ">=3.12"
dependencies = [
    "typer>=0.12",
    "pydantic>=2.7",
    "pyyaml>=6.0",
    "python-dateutil>=2.9",
]

[project.scripts]
lm = "lifemaint.cli:app"

[project.optional-dependencies]
dev = [
    "pytest>=8.0",
    "pytest-cov>=5.0",
    "ruff>=0.6",
    "pyright>=1.1",
    "types-pyyaml>=6.0",
    "types-python-dateutil>=2.9",
]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.ruff]
line-length = 100
src = ["src", "tests"]

[tool.ruff.lint]
select = ["E", "W", "F", "I", "B", "UP", "SIM", "RUF", "TID", "PL", "TCH"]

[tool.pyright]
include = ["src", "tests"]
typeCheckingMode = "strict"
pythonVersion = "3.12"

[tool.pytest.ini_options]
testpaths = ["tests"]
addopts = "-m 'not slow'"
markers = ["slow: slow tests", "integration: integration tests"]
```

- [ ] **Step 3: Create the package marker**

Create `src/lifemaint/__init__.py`:
```python
from __future__ import annotations

__all__: list[str] = []
```

- [ ] **Step 4: Install dependencies with uv**

Run: `uv sync --dev`
Expected: a `.venv` is created and `uv.lock` is written, no errors.

- [ ] **Step 5: Write a smoke test**

Create `tests/test_smoke.py`:
```python
from __future__ import annotations

import lifemaint


def test_package_imports() -> None:
    assert lifemaint.__name__ == "lifemaint"
```

- [ ] **Step 6: Run the smoke test + lint + types**

Run:
```bash
uv run pytest tests/test_smoke.py -v
uv run ruff check .
uv run pyright .
```
Expected: 1 passed; ruff reports "All checks passed!"; pyright reports 0 errors.

- [ ] **Step 7: Commit**

```bash
git add pyproject.toml .python-version uv.lock src/lifemaint/__init__.py tests/test_smoke.py
git commit -m "chore: scaffold lifemaint package with uv, ruff, pyright, pytest"
```

---

## Task 2: Exception hierarchy

**Files:**
- Create: `src/lifemaint/errors.py`
- Test: `tests/test_errors.py`

- [ ] **Step 1: Write the failing test**

Create `tests/test_errors.py`:
```python
from __future__ import annotations

import pytest

from lifemaint.errors import (
    DataFileError,
    LifemaintError,
    ScheduleParseError,
    UnknownVendorError,
)


def test_all_errors_subclass_base() -> None:
    for exc in (DataFileError, ScheduleParseError, UnknownVendorError):
        assert issubclass(exc, LifemaintError)


def test_unknown_vendor_error_names_the_offender() -> None:
    err = UnknownVendorError("clean-drains", "roto-rooter")
    assert "clean-drains" in str(err)
    assert "roto-rooter" in str(err)


def test_base_error_is_catchable_as_exception() -> None:
    with pytest.raises(LifemaintError):
        raise ScheduleParseError("bad interval")
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_errors.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.errors'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/errors.py`:
```python
from __future__ import annotations


class LifemaintError(Exception):
    """Base class for all lifemaint domain errors."""


class DataFileError(LifemaintError):
    """A data file (tasks/vendors/completions) is missing or malformed."""


class ScheduleParseError(LifemaintError):
    """An `every`/`on` recurrence spec could not be parsed."""


class UnknownVendorError(LifemaintError):
    """A task references a vendor id that is not defined in vendors.yaml."""

    def __init__(self, task_id: str, vendor_id: str) -> None:
        super().__init__(
            f"task {task_id!r} references unknown vendor {vendor_id!r}"
        )
        self.task_id = task_id
        self.vendor_id = vendor_id
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest tests/test_errors.py -v`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lifemaint/errors.py tests/test_errors.py
git commit -m "feat: add lifemaint exception hierarchy"
```

---

## Task 3: Relative schedules + interval parsing

**Files:**
- Create: `src/lifemaint/schedule.py`
- Test: `tests/test_schedule.py`

This task builds the `Relative` schedule and the interval parser. Fixed schedules come in Task 4.

**Interface defined here (used by every later task):**
- `parse_interval(text: str) -> relativedelta` — `"weekly" | "monthly" | "quarterly" | "yearly" | "N days/weeks/months/years"`.
- `class Relative` with `.next_due(anchor: date) -> date` (strictly after) and `.first_due(on_or_after: date) -> date` (inclusive — returns the arg).
- `Schedule` Protocol with both methods, so `status.py` is type-safe over either kind.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_schedule.py`:
```python
from __future__ import annotations

from datetime import date

import pytest

from lifemaint.errors import ScheduleParseError
from lifemaint.schedule import Relative, parse_interval


@pytest.mark.parametrize(
    "text,anchor,expected",
    [
        ("weekly", date(2026, 1, 1), date(2026, 1, 8)),
        ("monthly", date(2026, 1, 15), date(2026, 2, 15)),
        ("quarterly", date(2026, 1, 15), date(2026, 4, 15)),
        ("yearly", date(2026, 1, 15), date(2027, 1, 15)),
        ("6 months", date(2026, 1, 15), date(2026, 7, 15)),
        ("2 weeks", date(2026, 1, 1), date(2026, 1, 15)),
        ("10 days", date(2026, 1, 1), date(2026, 1, 11)),
        ("3 years", date(2026, 1, 1), date(2029, 1, 1)),
    ],
)
def test_relative_next_due_adds_interval(text: str, anchor: date, expected: date) -> None:
    assert Relative(parse_interval(text)).next_due(anchor) == expected


def test_relative_month_arithmetic_clamps_to_month_end() -> None:
    # Jan 31 + 1 month -> Feb 28 (dateutil relativedelta clamps).
    assert Relative(parse_interval("monthly")).next_due(date(2026, 1, 31)) == date(2026, 2, 28)


def test_relative_first_due_is_the_anchor_itself() -> None:
    # A never-done relative task is due on its start date (or today).
    assert Relative(parse_interval("monthly")).first_due(date(2026, 3, 1)) == date(2026, 3, 1)


@pytest.mark.parametrize("text", ["", "fortnightly", "2 fortnights", "monthlyy", "-1 days", "5"])
def test_parse_interval_rejects_garbage(text: str) -> None:
    with pytest.raises(ScheduleParseError):
        parse_interval(text)
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_schedule.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.schedule'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/schedule.py`:
```python
from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import date
from typing import Protocol, runtime_checkable

from dateutil.relativedelta import relativedelta

from lifemaint.errors import ScheduleParseError

_NAMED_INTERVALS: dict[str, relativedelta] = {
    "weekly": relativedelta(weeks=1),
    "monthly": relativedelta(months=1),
    "quarterly": relativedelta(months=3),
    "yearly": relativedelta(years=1),
}
_COUNT_PATTERN = re.compile(r"^(?P<n>\d+)\s+(?P<unit>days|weeks|months|years)$")


def parse_interval(text: str) -> relativedelta:
    key = text.strip().lower()
    if key in _NAMED_INTERVALS:
        return _NAMED_INTERVALS[key]
    match = _COUNT_PATTERN.match(key)
    if match is None:
        raise ScheduleParseError(
            f"cannot parse interval {text!r}; expected weekly/monthly/quarterly/"
            f"yearly or 'N days|weeks|months|years'"
        )
    n = int(match.group("n"))
    if n < 1:
        raise ScheduleParseError(f"interval count must be >= 1, got {n}")
    return relativedelta(**{match.group("unit"): n})


@runtime_checkable
class Schedule(Protocol):
    def next_due(self, anchor: date) -> date: ...
    def first_due(self, on_or_after: date) -> date: ...


@dataclass(frozen=True, slots=True)
class Relative:
    delta: relativedelta

    def next_due(self, anchor: date) -> date:
        return anchor + self.delta

    def first_due(self, on_or_after: date) -> date:
        return on_or_after
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_schedule.py -v`
Expected: all parametrized cases pass.

- [ ] **Step 5: Commit**

```bash
git add src/lifemaint/schedule.py tests/test_schedule.py
git commit -m "feat: add relative schedules and interval parsing"
```

---

## Task 4: Fixed (calendar-anchored) schedules + parse_schedule

**Files:**
- Modify: `src/lifemaint/schedule.py`
- Test: `tests/test_schedule.py` (extend)

**Interface added here:**
- `class Fixed` with `period` (`"yearly"` or `"monthly"`), `month: int | None`, `day: int`.
  - `.next_due(anchor)` → first scheduled occurrence strictly after `anchor`.
  - `.first_due(on_or_after)` → first scheduled occurrence on/after `on_or_after`.
  - Day clamped to month length (so `day=31` in Feb → Feb 28/29; `02-29` → Feb 28 in non-leap years).
- `parse_schedule(every: str, on: str | int | None) -> Schedule` — returns `Fixed` when `on` is set and `every` is `yearly`/`monthly`, else `Relative`.

- [ ] **Step 1: Write the failing tests (append to `tests/test_schedule.py`)**

```python
from lifemaint.schedule import Fixed, parse_schedule  # add to existing imports


def test_fixed_yearly_next_due_after_anchor() -> None:
    sched = Fixed(period="yearly", month=10, day=15)
    assert sched.next_due(date(2026, 6, 1)) == date(2026, 10, 15)
    assert sched.next_due(date(2026, 10, 15)) == date(2027, 10, 15)  # strictly after
    assert sched.next_due(date(2026, 11, 1)) == date(2027, 10, 15)


def test_fixed_yearly_first_due_is_inclusive() -> None:
    sched = Fixed(period="yearly", month=10, day=15)
    assert sched.first_due(date(2026, 10, 15)) == date(2026, 10, 15)  # inclusive
    assert sched.first_due(date(2026, 10, 16)) == date(2027, 10, 15)


def test_fixed_yearly_clamps_feb_29_in_non_leap_year() -> None:
    sched = Fixed(period="yearly", month=2, day=29)
    assert sched.next_due(date(2025, 1, 1)) == date(2025, 2, 28)  # 2025 not leap
    assert sched.next_due(date(2027, 3, 1)) == date(2028, 2, 29)  # 2028 leap


def test_fixed_monthly_next_due_and_clamp() -> None:
    sched = Fixed(period="monthly", month=None, day=31)
    assert sched.next_due(date(2026, 1, 15)) == date(2026, 1, 31)
    assert sched.next_due(date(2026, 1, 31)) == date(2026, 2, 28)  # clamps + strictly after
    assert sched.next_due(date(2026, 2, 28)) == date(2026, 3, 31)


def test_parse_schedule_builds_fixed_yearly_from_mmdd() -> None:
    sched = parse_schedule("yearly", "10-15")
    assert isinstance(sched, Fixed)
    assert (sched.period, sched.month, sched.day) == ("yearly", 10, 15)


def test_parse_schedule_builds_fixed_monthly_from_day_int() -> None:
    sched = parse_schedule("monthly", 1)
    assert isinstance(sched, Fixed)
    assert (sched.period, sched.month, sched.day) == ("monthly", None, 1)


def test_parse_schedule_without_on_is_relative() -> None:
    assert isinstance(parse_schedule("6 months", None), Relative)


@pytest.mark.parametrize(
    "every,on",
    [("weekly", "10-15"), ("yearly", "13-01"), ("yearly", "10-40"), ("monthly", 0), ("monthly", 32)],
)
def test_parse_schedule_rejects_invalid_fixed(every: str, on: str | int) -> None:
    with pytest.raises(ScheduleParseError):
        parse_schedule(every, on)
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_schedule.py -v`
Expected: FAIL with `ImportError: cannot import name 'Fixed'`.

- [ ] **Step 3: Write the implementation (append to `src/lifemaint/schedule.py`)**

Add imports at top (merge with existing): `import calendar` and `from typing import Literal`.

```python
def _clamp_day(year: int, month: int, day: int) -> date:
    last = calendar.monthrange(year, month)[1]
    return date(year, month, min(day, last))


@dataclass(frozen=True, slots=True)
class Fixed:
    period: Literal["yearly", "monthly"]
    day: int
    month: int | None = None  # required for yearly, None for monthly

    def _occurrence(self, year: int, month: int) -> date:
        return _clamp_day(year, month, self.day)

    def next_due(self, anchor: date) -> date:
        return self._search(anchor, inclusive=False)

    def first_due(self, on_or_after: date) -> date:
        return self._search(on_or_after, inclusive=True)

    def _search(self, ref: date, *, inclusive: bool) -> date:
        if self.period == "yearly":
            assert self.month is not None  # guaranteed by parse_schedule
            candidate = self._occurrence(ref.year, self.month)
            if candidate < ref or (candidate == ref and not inclusive):
                candidate = self._occurrence(ref.year + 1, self.month)
            return candidate
        # monthly: step month by month from ref's month
        year, month = ref.year, ref.month
        for _ in range(13):
            candidate = self._occurrence(year, month)
            if candidate > ref or (candidate == ref and inclusive):
                return candidate
            month += 1
            if month > 12:
                month, year = 1, year + 1
        raise ScheduleParseError("monthly schedule failed to resolve")  # unreachable


def parse_schedule(every: str, on: str | int | None) -> Schedule:
    if on is None:
        return Relative(parse_interval(every))
    period = every.strip().lower()
    if period == "yearly":
        if not isinstance(on, str):
            raise ScheduleParseError(f"yearly `on` must be 'MM-DD', got {on!r}")
        match = re.match(r"^(?P<m>\d{1,2})-(?P<d>\d{1,2})$", on.strip())
        if match is None:
            raise ScheduleParseError(f"yearly `on` must be 'MM-DD', got {on!r}")
        month, day = int(match.group("m")), int(match.group("d"))
        if not 1 <= month <= 12 or not 1 <= day <= 31:
            raise ScheduleParseError(f"invalid month/day in `on`: {on!r}")
        return Fixed(period="yearly", month=month, day=day)
    if period == "monthly":
        if not isinstance(on, int):
            raise ScheduleParseError(f"monthly `on` must be a day integer, got {on!r}")
        if not 1 <= on <= 31:
            raise ScheduleParseError(f"monthly `on` day must be 1..31, got {on}")
        return Fixed(period="monthly", day=on)
    raise ScheduleParseError(f"fixed schedules require every: yearly|monthly, got {every!r}")
```

> NOTE on the `assert`: this is guarding an invariant established by `parse_schedule` (a yearly `Fixed` always has `month`), used only to satisfy pyright's narrowing. The constitution bans `assert` for *runtime input validation* (it can be stripped by `-O`); this is a type-narrowing invariant, not input validation, and the real validation lives in `parse_schedule`. Acceptable.

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_schedule.py -v`
Expected: all pass.

- [ ] **Step 5: Lint + type-check**

Run: `uv run ruff check . && uv run pyright .`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/schedule.py tests/test_schedule.py
git commit -m "feat: add fixed calendar schedules and parse_schedule"
```

---

## Task 5: Pydantic boundary schema

**Files:**
- Create: `src/lifemaint/_schema.py`
- Test: `tests/test_schema.py`

These Pydantic models validate the raw YAML/JSONL at the file boundary (constitution: external boundary → Pydantic v2). They mirror the file shape exactly; conversion to domain types happens in Task 6.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_schema.py`:
```python
from __future__ import annotations

import pytest
from pydantic import ValidationError

from lifemaint._schema import RawCompletion, RawTask, RawVendor


def test_raw_task_minimal() -> None:
    t = RawTask.model_validate({"id": "groceries", "name": "Grocery shopping", "every": "weekly"})
    assert t.id == "groceries"
    assert t.on is None
    assert t.prep == []


def test_raw_task_full_fields() -> None:
    t = RawTask.model_validate(
        {
            "id": "blow-out-sprinklers",
            "name": "Blow out sprinklers",
            "every": "yearly",
            "on": "10-15",
            "lead_time": "2 weeks",
            "prep": ["Find the compressor"],
            "vendor": "green-lawn",
            "notes": "before first freeze",
            "start": "2026-01-15",
        }
    )
    assert t.on == "10-15"
    assert t.start is not None and t.start.year == 2026


def test_raw_task_rejects_unknown_field() -> None:
    with pytest.raises(ValidationError):
        RawTask.model_validate(
            {"id": "x", "name": "X", "every": "weekly", "frequency": "oops"}
        )


def test_raw_task_requires_id_name_every() -> None:
    with pytest.raises(ValidationError):
        RawTask.model_validate({"name": "no id", "every": "weekly"})


def test_raw_completion_defaults_via_and_optional_money() -> None:
    c = RawCompletion.model_validate({"id": "groceries", "done": "2026-06-05"})
    assert c.via == "manual"
    assert c.cost_cents is None
    assert c.by is None


def test_raw_completion_cost_cents_must_be_int() -> None:
    with pytest.raises(ValidationError):
        RawCompletion.model_validate({"id": "x", "done": "2026-06-05", "cost_cents": 12.5})


def test_raw_vendor_minimal() -> None:
    v = RawVendor.model_validate({"id": "green-lawn", "name": "Green Lawn"})
    assert v.phone is None
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_schema.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint._schema'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/_schema.py`:
```python
from __future__ import annotations

from datetime import date
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

Via = Literal["cli", "telegram", "web", "manual", "agent"]


class RawTask(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    every: str
    on: str | int | None = None
    lead_time: str | None = None
    prep: list[str] = Field(default_factory=list)
    vendor: str | None = None
    notes: str | None = None
    start: date | None = None


class RawVendor(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    phone: str | None = None
    email: str | None = None
    notes: str | None = None


class RawCompletion(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    done: date
    via: Via = "manual"
    by: str | None = None
    cost_cents: int | None = None
    note: str | None = None
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_schema.py -v`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/lifemaint/_schema.py tests/test_schema.py
git commit -m "feat: add pydantic boundary schema for data files"
```

---

## Task 6: Domain models + conversion

**Files:**
- Create: `src/lifemaint/models.py`
- Test: `tests/test_models.py`

Frozen domain dataclasses that flow internally, built from the raw schema. `lead_time` becomes a `relativedelta`; `every`/`on` become a `Schedule`.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_models.py`:
```python
from __future__ import annotations

from datetime import date

import pytest

from lifemaint._schema import RawCompletion, RawTask, RawVendor
from lifemaint.errors import ScheduleParseError
from lifemaint.models import Completion, Task, Vendor
from lifemaint.schedule import Fixed, Relative


def test_task_from_raw_relative() -> None:
    task = Task.from_raw(
        RawTask.model_validate(
            {"id": "gutters", "name": "Gutters", "every": "6 months", "lead_time": "2 weeks"}
        )
    )
    assert task.id == "gutters"
    assert isinstance(task.schedule, Relative)
    assert task.lead_time is not None
    assert task.prep == ()  # tuple, immutable


def test_task_from_raw_fixed_with_on() -> None:
    task = Task.from_raw(
        RawTask.model_validate(
            {"id": "sprinklers", "name": "Sprinklers", "every": "yearly", "on": "10-15"}
        )
    )
    assert isinstance(task.schedule, Fixed)


def test_task_from_raw_no_lead_time_is_none() -> None:
    task = Task.from_raw(RawTask.model_validate({"id": "g", "name": "G", "every": "weekly"}))
    assert task.lead_time is None


def test_task_from_raw_bad_interval_raises() -> None:
    with pytest.raises(ScheduleParseError):
        Task.from_raw(RawTask.model_validate({"id": "g", "name": "G", "every": "fortnightly"}))


def test_completion_from_raw_carries_money_and_meta() -> None:
    c = Completion.from_raw(
        RawCompletion.model_validate(
            {"id": "drains", "done": "2026-05-01", "by": "roto", "cost_cents": 28500}
        )
    )
    assert c.done == date(2026, 5, 1)
    assert c.cost_cents == 28500
    assert c.by == "roto"


def test_vendor_from_raw() -> None:
    v = Vendor.from_raw(RawVendor.model_validate({"id": "roto", "name": "Roto-Rooter"}))
    assert v.name == "Roto-Rooter"
    assert v.phone is None


def test_task_is_frozen() -> None:
    task = Task.from_raw(RawTask.model_validate({"id": "g", "name": "G", "every": "weekly"}))
    with pytest.raises(AttributeError):
        task.id = "other"  # type: ignore[misc]
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_models.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.models'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/models.py`:
```python
from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from typing import TYPE_CHECKING, Self

from lifemaint.schedule import Schedule, parse_interval, parse_schedule

if TYPE_CHECKING:
    from dateutil.relativedelta import relativedelta

    from lifemaint._schema import RawCompletion, RawTask, RawVendor


@dataclass(frozen=True, slots=True)
class Vendor:
    id: str
    name: str
    phone: str | None = None
    email: str | None = None
    notes: str | None = None

    @classmethod
    def from_raw(cls, raw: RawVendor) -> Self:
        return cls(id=raw.id, name=raw.name, phone=raw.phone, email=raw.email, notes=raw.notes)


@dataclass(frozen=True, slots=True)
class Task:
    id: str
    name: str
    schedule: Schedule
    lead_time: relativedelta | None = None
    prep: tuple[str, ...] = ()
    vendor: str | None = None
    notes: str | None = None
    start: date | None = None

    @classmethod
    def from_raw(cls, raw: RawTask) -> Self:
        lead = parse_interval(raw.lead_time) if raw.lead_time is not None else None
        return cls(
            id=raw.id,
            name=raw.name,
            schedule=parse_schedule(raw.every, raw.on),
            lead_time=lead,
            prep=tuple(raw.prep),
            vendor=raw.vendor,
            notes=raw.notes,
            start=raw.start,
        )


@dataclass(frozen=True, slots=True)
class Completion:
    id: str
    done: date
    via: str
    by: str | None = None
    cost_cents: int | None = None
    note: str | None = None

    @classmethod
    def from_raw(cls, raw: RawCompletion) -> Self:
        return cls(
            id=raw.id,
            done=raw.done,
            via=raw.via,
            by=raw.by,
            cost_cents=raw.cost_cents,
            note=raw.note,
        )
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_models.py -v`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/lifemaint/models.py tests/test_models.py
git commit -m "feat: add frozen domain models with raw->domain conversion"
```

---

## Task 7: Status engine (pure)

**Files:**
- Create: `src/lifemaint/status.py`
- Test: `tests/test_status.py`

The brain. Pure function: given tasks, completions, and `today`, produce a bucketed status per task. No I/O.

**Interface defined here:**
- `class Bucket(StrEnum)`: `OVERDUE`, `DUE`, `PREP`, `OK`.
- `@dataclass(frozen=True, slots=True) class TaskStatus`: `task: Task`, `last_done: date | None`, `next_due: date`, `prep_due: date | None`, `bucket: Bucket`.
- `compute_status(tasks: Sequence[Task], completions: Sequence[Completion], today: date) -> list[TaskStatus]`.

**Bucketing (mutually exclusive):**
- `next_due < today` → `OVERDUE`
- `next_due == today` → `DUE`
- `prep_due is not None and prep_due <= today < next_due` → `PREP`
- otherwise → `OK`

**Anchor selection:** `last_done` = latest completion `done` for that id (or `None`). If `last_done` is set → `schedule.next_due(last_done)`. Else → `schedule.first_due(task.start or today)`.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_status.py`:
```python
from __future__ import annotations

from datetime import date

import pytest

from lifemaint._schema import RawCompletion, RawTask
from lifemaint.models import Completion, Task
from lifemaint.status import Bucket, compute_status


def _task(**kw: object) -> Task:
    base = {"id": "t", "name": "T", "every": "monthly"}
    return Task.from_raw(RawTask.model_validate({**base, **kw}))


def _done(task_id: str, when: str) -> Completion:
    return Completion.from_raw(RawCompletion.model_validate({"id": task_id, "done": when}))


def test_never_done_no_start_is_due_today() -> None:
    [status] = compute_status([_task(id="t")], [], date(2026, 6, 6))
    assert status.bucket == Bucket.DUE
    assert status.next_due == date(2026, 6, 6)
    assert status.last_done is None


def test_relative_overdue_when_interval_elapsed() -> None:
    task = _task(id="t", every="monthly")
    [status] = compute_status([task], [_done("t", "2026-01-01")], date(2026, 6, 6))
    assert status.next_due == date(2026, 2, 1)
    assert status.bucket == Bucket.OVERDUE


def test_relative_ok_when_recently_done() -> None:
    task = _task(id="t", every="monthly")
    [status] = compute_status([task], [_done("t", "2026-06-01")], date(2026, 6, 6))
    assert status.next_due == date(2026, 7, 1)
    assert status.bucket == Bucket.OK


def test_latest_completion_wins_over_earlier() -> None:
    task = _task(id="t", every="monthly")
    completions = [_done("t", "2026-01-01"), _done("t", "2026-06-01")]
    [status] = compute_status([task], completions, date(2026, 6, 6))
    assert status.last_done == date(2026, 6, 1)


def test_prep_bucket_inside_lead_window() -> None:
    task = _task(id="t", every="yearly", on="10-15", lead_time="2 weeks")
    # never done, start before today so first occurrence is 2026-10-15
    [status] = compute_status([task], [], date(2026, 10, 5))
    assert status.next_due == date(2026, 10, 15)
    assert status.prep_due == date(2026, 10, 1)
    assert status.bucket == Bucket.PREP


def test_prep_not_yet_reached_is_ok() -> None:
    task = _task(id="t", every="yearly", on="10-15", lead_time="2 weeks")
    [status] = compute_status([task], [], date(2026, 9, 1))
    assert status.bucket == Bucket.OK


def test_due_today_exactly() -> None:
    task = _task(id="t", every="monthly")
    [status] = compute_status([task], [_done("t", "2026-05-06")], date(2026, 6, 6))
    assert status.next_due == date(2026, 6, 6)
    assert status.bucket == Bucket.DUE


def test_completions_for_other_tasks_are_ignored() -> None:
    task = _task(id="t", every="monthly")
    [status] = compute_status([task], [_done("other", "2026-06-01")], date(2026, 6, 6))
    assert status.last_done is None


@pytest.mark.parametrize(
    "start,expected_bucket",
    [("2026-12-01", Bucket.OK), ("2026-01-01", Bucket.OVERDUE)],
)
def test_relative_never_done_uses_start_anchor(start: str, expected_bucket: Bucket) -> None:
    task = _task(id="t", every="monthly", start=start)
    [status] = compute_status([task], [], date(2026, 6, 6))
    assert status.bucket == expected_bucket
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_status.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.status'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/status.py`:
```python
from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from enum import StrEnum
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Sequence

    from lifemaint.models import Completion, Task


class Bucket(StrEnum):
    OVERDUE = "overdue"
    DUE = "due"
    PREP = "prep"
    OK = "ok"


@dataclass(frozen=True, slots=True)
class TaskStatus:
    task: Task
    last_done: date | None
    next_due: date
    prep_due: date | None
    bucket: Bucket


def _latest_completion(task_id: str, completions: Sequence[Completion]) -> date | None:
    dates = [c.done for c in completions if c.id == task_id]
    return max(dates) if dates else None


def _bucket_for(next_due: date, prep_due: date | None, today: date) -> Bucket:
    if next_due < today:
        return Bucket.OVERDUE
    if next_due == today:
        return Bucket.DUE
    if prep_due is not None and prep_due <= today < next_due:
        return Bucket.PREP
    return Bucket.OK


def compute_status(
    tasks: Sequence[Task],
    completions: Sequence[Completion],
    today: date,
) -> list[TaskStatus]:
    results: list[TaskStatus] = []
    for task in tasks:
        last_done = _latest_completion(task.id, completions)
        if last_done is not None:
            next_due = task.schedule.next_due(last_done)
        else:
            next_due = task.schedule.first_due(task.start or today)
        prep_due = (next_due - task.lead_time) if task.lead_time is not None else None
        results.append(
            TaskStatus(
                task=task,
                last_done=last_done,
                next_due=next_due,
                prep_due=prep_due,
                bucket=_bucket_for(next_due, prep_due, today),
            )
        )
    return results
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_status.py -v`
Expected: all pass.

- [ ] **Step 5: Lint + type-check**

Run: `uv run ruff check . && uv run pyright .`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/status.py tests/test_status.py
git commit -m "feat: add pure status engine with overdue/due/prep/ok bucketing"
```

---

## Task 8: Store — load data files

**Files:**
- Create: `src/lifemaint/_store.py`
- Test: `tests/test_store.py`, `tests/conftest.py`

The DAO. Reads `tasks.yaml`, `vendors.yaml`, `completions.jsonl` from a data dir, validates via the Pydantic schema, converts to domain models, and validates vendor references. Malformed files raise `DataFileError` naming the offender.

**Interface defined here:**
- `@dataclass(frozen=True, slots=True) class DataDir`: holds `root: Path`; properties `tasks_path`, `vendors_path`, `completions_path`.
- `load_tasks(d: DataDir) -> list[Task]`
- `load_vendors(d: DataDir) -> list[Vendor]`
- `load_completions(d: DataDir) -> list[Completion]`
- `load_all(d: DataDir) -> tuple[list[Task], list[Vendor], list[Completion]]` — also checks every task `vendor` exists, raising `UnknownVendorError`.

- [ ] **Step 1: Add fixtures to `tests/conftest.py`**

Create `tests/conftest.py`:
```python
from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from lifemaint._store import DataDir

_TASKS = """\
- id: groceries
  name: Grocery shopping
  every: weekly
- id: clean-drains
  name: Clean out drains
  every: yearly
  lead_time: 2 weeks
  vendor: roto-rooter
- id: blow-out-sprinklers
  name: Blow out sprinklers
  every: yearly
  on: "10-15"
  lead_time: 2 weeks
"""

_VENDORS = """\
- id: roto-rooter
  name: Roto-Rooter
  phone: "555-123-4567"
"""

_COMPLETIONS = (
    '{"id": "groceries", "done": "2026-06-01", "via": "cli", "by": "self"}\n'
    '{"id": "clean-drains", "done": "2025-05-01", "via": "cli", "by": "roto-rooter", "cost_cents": 28500}\n'
)


@pytest.fixture
def data_dir(tmp_path: Path) -> DataDir:
    (tmp_path / "tasks.yaml").write_text(_TASKS)
    (tmp_path / "vendors.yaml").write_text(_VENDORS)
    (tmp_path / "completions.jsonl").write_text(_COMPLETIONS)
    return DataDir(root=tmp_path)


@pytest.fixture
def git_data_dir(data_dir: DataDir) -> DataDir:
    root = data_dir.root
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=root, check=True)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=root, check=True)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "seed"], cwd=root, check=True)
    return data_dir
```

- [ ] **Step 2: Write the failing tests**

Create `tests/test_store.py`:
```python
from __future__ import annotations

from datetime import date
from pathlib import Path

import pytest

from lifemaint._store import DataDir, load_all, load_completions, load_tasks, load_vendors
from lifemaint.errors import DataFileError, UnknownVendorError


def test_load_tasks_parses_all(data_dir: DataDir) -> None:
    tasks = load_tasks(data_dir)
    assert [t.id for t in tasks] == ["groceries", "clean-drains", "blow-out-sprinklers"]


def test_load_vendors(data_dir: DataDir) -> None:
    vendors = load_vendors(data_dir)
    assert vendors[0].id == "roto-rooter"
    assert vendors[0].phone == "555-123-4567"


def test_load_completions(data_dir: DataDir) -> None:
    completions = load_completions(data_dir)
    assert len(completions) == 2
    assert completions[1].cost_cents == 28500
    assert completions[0].done == date(2026, 6, 1)


def test_load_all_returns_three_lists(data_dir: DataDir) -> None:
    tasks, vendors, completions = load_all(data_dir)
    assert len(tasks) == 3
    assert len(vendors) == 1
    assert len(completions) == 2


def test_load_all_rejects_unknown_vendor_reference(tmp_path: Path) -> None:
    (tmp_path / "tasks.yaml").write_text(
        '- id: x\n  name: X\n  every: weekly\n  vendor: ghost\n'
    )
    (tmp_path / "vendors.yaml").write_text("[]\n")
    (tmp_path / "completions.jsonl").write_text("")
    with pytest.raises(UnknownVendorError) as exc:
        load_all(DataDir(root=tmp_path))
    assert exc.value.vendor_id == "ghost"


def test_malformed_yaml_raises_datafileerror_naming_file(tmp_path: Path) -> None:
    (tmp_path / "tasks.yaml").write_text("- id: x\n  name: X\n  every: fortnightly\n")
    (tmp_path / "vendors.yaml").write_text("[]\n")
    (tmp_path / "completions.jsonl").write_text("")
    with pytest.raises(DataFileError) as exc:
        load_tasks(DataDir(root=tmp_path))
    assert "tasks.yaml" in str(exc.value)


def test_missing_files_are_treated_as_empty(tmp_path: Path) -> None:
    tasks, vendors, completions = load_all(DataDir(root=tmp_path))
    assert tasks == [] and vendors == [] and completions == []


def test_malformed_jsonl_line_raises_with_line_number(tmp_path: Path) -> None:
    (tmp_path / "completions.jsonl").write_text('{"id": "x", "done": "2026-01-01"}\nNOT JSON\n')
    with pytest.raises(DataFileError) as exc:
        load_completions(DataDir(root=tmp_path))
    assert "line 2" in str(exc.value)
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `uv run pytest tests/test_store.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint._store'`.

- [ ] **Step 4: Write the implementation**

Create `src/lifemaint/_store.py`:
```python
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

import yaml
from pydantic import ValidationError

from lifemaint._schema import RawCompletion, RawTask, RawVendor
from lifemaint.errors import DataFileError, LifemaintError, UnknownVendorError
from lifemaint.models import Completion, Task, Vendor

if TYPE_CHECKING:
    from collections.abc import Iterable

_TASKS_FILE = "tasks.yaml"
_VENDORS_FILE = "vendors.yaml"
_COMPLETIONS_FILE = "completions.jsonl"


@dataclass(frozen=True, slots=True)
class DataDir:
    root: Path

    @property
    def tasks_path(self) -> Path:
        return self.root / _TASKS_FILE

    @property
    def vendors_path(self) -> Path:
        return self.root / _VENDORS_FILE

    @property
    def completions_path(self) -> Path:
        return self.root / _COMPLETIONS_FILE


def _load_yaml_list(path: Path) -> list[dict[str, object]]:
    if not path.exists():
        return []
    try:
        raw = yaml.safe_load(path.read_text()) or []
    except yaml.YAMLError as e:
        raise DataFileError(f"{path.name}: invalid YAML: {e}") from e
    if not isinstance(raw, list):
        raise DataFileError(f"{path.name}: expected a list at the top level")
    return raw


def load_tasks(d: DataDir) -> list[Task]:
    tasks: list[Task] = []
    for i, item in enumerate(_load_yaml_list(d.tasks_path)):
        try:
            tasks.append(Task.from_raw(RawTask.model_validate(item)))
        except (ValidationError, LifemaintError) as e:
            raise DataFileError(f"{_TASKS_FILE}: entry {i} ({item!r}): {e}") from e
    return tasks


def load_vendors(d: DataDir) -> list[Vendor]:
    vendors: list[Vendor] = []
    for i, item in enumerate(_load_yaml_list(d.vendors_path)):
        try:
            vendors.append(Vendor.from_raw(RawVendor.model_validate(item)))
        except ValidationError as e:
            raise DataFileError(f"{_VENDORS_FILE}: entry {i} ({item!r}): {e}") from e
    return vendors


def load_completions(d: DataDir) -> list[Completion]:
    path = d.completions_path
    if not path.exists():
        return []
    completions: list[Completion] = []
    for lineno, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            obj = json.loads(line)
            completions.append(Completion.from_raw(RawCompletion.model_validate(obj)))
        except (json.JSONDecodeError, ValidationError) as e:
            raise DataFileError(f"{_COMPLETIONS_FILE}: line {lineno}: {e}") from e
    return completions


def _check_vendor_refs(tasks: Iterable[Task], vendors: Iterable[Vendor]) -> None:
    known = {v.id for v in vendors}
    for task in tasks:
        if task.vendor is not None and task.vendor not in known:
            raise UnknownVendorError(task.id, task.vendor)


def load_all(d: DataDir) -> tuple[list[Task], list[Vendor], list[Completion]]:
    tasks = load_tasks(d)
    vendors = load_vendors(d)
    completions = load_completions(d)
    _check_vendor_refs(tasks, vendors)
    return tasks, vendors, completions
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `uv run pytest tests/test_store.py -v`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/_store.py tests/test_store.py tests/conftest.py
git commit -m "feat: add data-dir store with validated loading of all files"
```

---

## Task 9: Store — append completion + git commit

**Files:**
- Modify: `src/lifemaint/_store.py`
- Test: `tests/test_store.py` (extend)

**Interface added here:**
- `append_completion(d: DataDir, completion: Completion) -> None` — appends one JSON line (ISO date, omitting `None` fields), creating the file if needed.
- `commit(d: DataDir, message: str) -> bool` — `git add -A && git commit` in the data dir; returns `False` (non-fatal) when there is nothing to commit or the dir is not a git repo; never raises on those.

- [ ] **Step 1: Write the failing tests (append to `tests/test_store.py`)**

```python
import subprocess  # add to existing imports

from lifemaint._store import append_completion, commit  # add to existing imports
from lifemaint.models import Completion  # add to existing imports


def test_append_completion_writes_one_json_line(data_dir: DataDir) -> None:
    before = len(load_completions(data_dir))
    append_completion(
        data_dir,
        Completion(id="groceries", done=date(2026, 6, 6), via="cli", by="self"),
    )
    after = load_completions(data_dir)
    assert len(after) == before + 1
    assert after[-1].id == "groceries"
    assert after[-1].done == date(2026, 6, 6)


def test_append_completion_omits_none_fields(data_dir: DataDir) -> None:
    append_completion(
        data_dir, Completion(id="groceries", done=date(2026, 6, 6), via="cli")
    )
    last_line = data_dir.completions_path.read_text().splitlines()[-1]
    assert "cost_cents" not in last_line
    assert "note" not in last_line


def test_append_creates_file_when_missing(tmp_path: Path) -> None:
    d = DataDir(root=tmp_path)
    append_completion(d, Completion(id="x", done=date(2026, 1, 1), via="cli"))
    assert d.completions_path.exists()
    assert load_completions(d)[0].id == "x"


def test_commit_persists_change(git_data_dir: DataDir) -> None:
    append_completion(
        git_data_dir, Completion(id="groceries", done=date(2026, 6, 6), via="cli")
    )
    assert commit(git_data_dir, "complete groceries") is True
    log = subprocess.run(
        ["git", "log", "--oneline"], cwd=git_data_dir.root, capture_output=True, text=True, check=True
    )
    assert "complete groceries" in log.stdout


def test_commit_is_noop_when_nothing_changed(git_data_dir: DataDir) -> None:
    assert commit(git_data_dir, "no changes") is False


def test_commit_returns_false_when_not_a_git_repo(data_dir: DataDir) -> None:
    assert commit(data_dir, "x") is False
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_store.py -k "append or commit" -v`
Expected: FAIL with `ImportError: cannot import name 'append_completion'`.

- [ ] **Step 3: Write the implementation (append to `src/lifemaint/_store.py`)**

Add `import subprocess` to the imports block. Then append:
```python
def append_completion(d: DataDir, completion: Completion) -> None:
    record: dict[str, object] = {
        "id": completion.id,
        "done": completion.done.isoformat(),
        "via": completion.via,
    }
    if completion.by is not None:
        record["by"] = completion.by
    if completion.cost_cents is not None:
        record["cost_cents"] = completion.cost_cents
    if completion.note is not None:
        record["note"] = completion.note
    d.completions_path.parent.mkdir(parents=True, exist_ok=True)
    with d.completions_path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(record) + "\n")


def commit(d: DataDir, message: str) -> bool:
    if not (d.root / ".git").exists():
        return False
    subprocess.run(["git", "add", "-A"], cwd=d.root, check=True)
    result = subprocess.run(
        ["git", "commit", "-m", message], cwd=d.root, capture_output=True, text=True
    )
    # Non-zero with "nothing to commit" is normal and non-fatal.
    return result.returncode == 0
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_store.py -v`
Expected: all pass.

- [ ] **Step 5: Lint + type-check**

Run: `uv run ruff check . && uv run pyright .`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/_store.py tests/test_store.py
git commit -m "feat: append completions and auto-commit to the data repo"
```

---

## Task 10: Service — read operations (list/due/history/vendors/export)

**Files:**
- Create: `src/lifemaint/service.py`
- Test: `tests/test_service.py`

The controller. Orchestrates store + status into the operations the CLI needs. Returns domain/value objects (no JSON, no printing). `today` is injectable for tests.

**Interface defined here:**
- `@dataclass(frozen=True, slots=True) class Service`: holds `data_dir: DataDir`.
- `list_tasks(self, *, query: str | None = None, only: Bucket | set[Bucket] | None = None, today: date) -> list[TaskStatus]` — search across id/name/notes/vendor; optional bucket filter.
- `due(self, *, today: date) -> list[TaskStatus]` — buckets OVERDUE, DUE, PREP only.
- `history(self, *, task_id: str | None = None, since: date | None = None) -> list[Completion]`.
- `vendors(self) -> list[Vendor]`.
- `export(self, *, today: date) -> dict[str, object]` — denormalised tasks⋈status⋈completions⋈vendors as JSON-ready primitives, with a `schema_version`.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_service.py`:
```python
from __future__ import annotations

from datetime import date

from lifemaint._store import DataDir
from lifemaint.service import Service
from lifemaint.status import Bucket


def test_list_tasks_returns_all_with_status(data_dir: DataDir) -> None:
    statuses = Service(data_dir).list_tasks(today=date(2026, 6, 6))
    assert {s.task.id for s in statuses} == {"groceries", "clean-drains", "blow-out-sprinklers"}


def test_list_tasks_search_matches_name(data_dir: DataDir) -> None:
    statuses = Service(data_dir).list_tasks(query="drain", today=date(2026, 6, 6))
    assert [s.task.id for s in statuses] == ["clean-drains"]


def test_list_tasks_search_matches_vendor(data_dir: DataDir) -> None:
    statuses = Service(data_dir).list_tasks(query="roto", today=date(2026, 6, 6))
    assert [s.task.id for s in statuses] == ["clean-drains"]


def test_list_tasks_filter_by_overdue_bucket(data_dir: DataDir) -> None:
    # clean-drains last done 2025-05-01, yearly -> due 2026-05-01 -> overdue on 2026-06-06
    statuses = Service(data_dir).list_tasks(only={Bucket.OVERDUE}, today=date(2026, 6, 6))
    assert [s.task.id for s in statuses] == ["clean-drains"]


def test_due_includes_overdue_due_prep_excludes_ok(data_dir: DataDir) -> None:
    statuses = Service(data_dir).due(today=date(2026, 6, 6))
    ids = {s.task.id for s in statuses}
    # groceries weekly last done 2026-06-01 -> due 2026-06-08 -> OK, excluded
    assert "groceries" not in ids
    assert "clean-drains" in ids  # overdue


def test_history_all(data_dir: DataDir) -> None:
    assert len(Service(data_dir).history()) == 2


def test_history_filtered_by_task(data_dir: DataDir) -> None:
    rows = Service(data_dir).history(task_id="clean-drains")
    assert len(rows) == 1
    assert rows[0].cost_cents == 28500


def test_history_filtered_since(data_dir: DataDir) -> None:
    rows = Service(data_dir).history(since=date(2026, 1, 1))
    assert [r.id for r in rows] == ["groceries"]


def test_vendors(data_dir: DataDir) -> None:
    vendors = Service(data_dir).vendors()
    assert [v.id for v in vendors] == ["roto-rooter"]


def test_export_has_schema_version_and_joined_rows(data_dir: DataDir) -> None:
    payload = Service(data_dir).export(today=date(2026, 6, 6))
    assert payload["schema_version"] == 1
    tasks = payload["tasks"]
    assert isinstance(tasks, list)
    drains = next(t for t in tasks if t["id"] == "clean-drains")
    assert drains["bucket"] == "overdue"
    assert drains["vendor"]["name"] == "Roto-Rooter"
    assert drains["next_due"] == "2026-05-01"
    assert drains["completions"][0]["cost_cents"] == 28500
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_service.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.service'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/service.py`:
```python
from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

from lifemaint._store import load_all
from lifemaint.status import Bucket, TaskStatus, compute_status

if TYPE_CHECKING:
    from datetime import date

    from lifemaint._store import DataDir
    from lifemaint.models import Completion, Vendor

_DUE_BUCKETS = {Bucket.OVERDUE, Bucket.DUE, Bucket.PREP}


def _matches(status: TaskStatus, query: str) -> bool:
    q = query.lower()
    task = status.task
    haystacks = [task.id, task.name, task.notes or "", task.vendor or ""]
    return any(q in field.lower() for field in haystacks)


@dataclass(frozen=True, slots=True)
class Service:
    data_dir: DataDir

    def list_tasks(
        self,
        *,
        today: date,
        query: str | None = None,
        only: Bucket | set[Bucket] | None = None,
    ) -> list[TaskStatus]:
        tasks, _vendors, completions = load_all(self.data_dir)
        statuses = compute_status(tasks, completions, today)
        if query is not None:
            statuses = [s for s in statuses if _matches(s, query)]
        if only is not None:
            wanted = {only} if isinstance(only, Bucket) else only
            statuses = [s for s in statuses if s.bucket in wanted]
        return statuses

    def due(self, *, today: date) -> list[TaskStatus]:
        return self.list_tasks(today=today, only=_DUE_BUCKETS)

    def history(
        self, *, task_id: str | None = None, since: date | None = None
    ) -> list[Completion]:
        _tasks, _vendors, completions = load_all(self.data_dir)
        rows = completions
        if task_id is not None:
            rows = [c for c in rows if c.id == task_id]
        if since is not None:
            rows = [c for c in rows if c.done >= since]
        return rows

    def vendors(self) -> list[Vendor]:
        _tasks, vendors, _completions = load_all(self.data_dir)
        return vendors

    def export(self, *, today: date) -> dict[str, object]:
        tasks, vendors, completions = load_all(self.data_dir)
        statuses = compute_status(tasks, completions, today)
        vendor_by_id = {v.id: v for v in vendors}
        rows: list[dict[str, object]] = []
        for status in statuses:
            task = status.task
            vendor = vendor_by_id.get(task.vendor) if task.vendor else None
            rows.append(
                {
                    "id": task.id,
                    "name": task.name,
                    "bucket": status.bucket.value,
                    "last_done": status.last_done.isoformat() if status.last_done else None,
                    "next_due": status.next_due.isoformat(),
                    "prep_due": status.prep_due.isoformat() if status.prep_due else None,
                    "vendor": None
                    if vendor is None
                    else {"id": vendor.id, "name": vendor.name, "phone": vendor.phone},
                    "completions": [
                        {
                            "done": c.done.isoformat(),
                            "via": c.via,
                            "by": c.by,
                            "cost_cents": c.cost_cents,
                            "note": c.note,
                        }
                        for c in completions
                        if c.id == task.id
                    ],
                }
            )
        return {"schema_version": 1, "generated_for": today.isoformat(), "tasks": rows}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_service.py -v`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/lifemaint/service.py tests/test_service.py
git commit -m "feat: add service read operations (list/due/history/vendors/export)"
```

---

## Task 11: Service — complete + reports

**Files:**
- Modify: `src/lifemaint/service.py`
- Test: `tests/test_service.py` (extend)

**Interface added here:**
- `complete(self, task_id: str, *, done: date, via: str = "cli", by: str = "self", cost_cents: int | None = None, note: str | None = None, do_commit: bool = True) -> Completion` — validates the task id exists, appends, optionally commits. Raises `DataFileError` for an unknown task id.
- `report(self, kind: ReportKind, *, today: date) -> dict[str, object]` where `ReportKind` is `StrEnum{SPEND_BY_TASK, PER_YEAR, OVERDUE_COUNT}`.
  - `SPEND_BY_TASK`: `{task_id: total_cents}` over all completions.
  - `PER_YEAR`: `{year: total_cents}`.
  - `OVERDUE_COUNT`: `{"overdue": n, "due": n, "prep": n}` from today's status.

- [ ] **Step 1: Write the failing tests (append to `tests/test_service.py`)**

```python
import pytest  # add to existing imports

from lifemaint.errors import DataFileError  # add to existing imports
from lifemaint.service import ReportKind  # add to existing imports


def test_complete_appends_and_is_visible(git_data_dir: DataDir) -> None:
    svc = Service(git_data_dir)
    svc.complete("groceries", done=date(2026, 6, 6), cost_cents=4200, note="weekly run")
    rows = svc.history(task_id="groceries")
    assert rows[-1].done == date(2026, 6, 6)
    assert rows[-1].cost_cents == 4200


def test_complete_unknown_task_raises(data_dir: DataDir) -> None:
    with pytest.raises(DataFileError):
        Service(data_dir).complete("nope", done=date(2026, 6, 6), do_commit=False)


def test_complete_resets_next_due(git_data_dir: DataDir) -> None:
    svc = Service(git_data_dir)
    # clean-drains overdue before completion
    before = next(s for s in svc.list_tasks(today=date(2026, 6, 6)) if s.task.id == "clean-drains")
    assert before.bucket == Bucket.OVERDUE
    svc.complete("clean-drains", done=date(2026, 6, 6))
    after = next(s for s in svc.list_tasks(today=date(2026, 6, 6)) if s.task.id == "clean-drains")
    assert after.next_due == date(2027, 6, 6)
    assert after.bucket == Bucket.OK


def test_report_spend_by_task(data_dir: DataDir) -> None:
    report = Service(data_dir).report(ReportKind.SPEND_BY_TASK, today=date(2026, 6, 6))
    assert report["spend_by_task"] == {"clean-drains": 28500}


def test_report_per_year(data_dir: DataDir) -> None:
    report = Service(data_dir).report(ReportKind.PER_YEAR, today=date(2026, 6, 6))
    assert report["per_year"] == {"2025": 28500}


def test_report_overdue_count(data_dir: DataDir) -> None:
    report = Service(data_dir).report(ReportKind.OVERDUE_COUNT, today=date(2026, 6, 6))
    assert report["overdue"] == 1
    assert report["due"] == 0
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_service.py -k "complete or report" -v`
Expected: FAIL with `ImportError: cannot import name 'ReportKind'`.

- [ ] **Step 3: Write the implementation (modify `src/lifemaint/service.py`)**

Adjust imports (to avoid `F811` redefinition):
- Add `from enum import StrEnum` at the top.
- Extend the existing runtime import to `from lifemaint._store import append_completion, commit, load_all`.
- Add the runtime import `from lifemaint.errors import DataFileError`.
- **Move** `Completion` out of the `if TYPE_CHECKING:` block into a runtime import: `from lifemaint.models import Completion` (it is now constructed at runtime). Leave `Vendor` under `TYPE_CHECKING`.

Add the enum near the top (after imports, before `Service`):
```python
class ReportKind(StrEnum):
    SPEND_BY_TASK = "spend-by-task"
    PER_YEAR = "per-year"
    OVERDUE_COUNT = "overdue-count"
```
Add these methods to `Service`:
```python
    def complete(
        self,
        task_id: str,
        *,
        done: date,
        via: str = "cli",
        by: str = "self",
        cost_cents: int | None = None,
        note: str | None = None,
        do_commit: bool = True,
    ) -> Completion:
        tasks, _vendors, _completions = load_all(self.data_dir)
        if not any(t.id == task_id for t in tasks):
            raise DataFileError(f"unknown task id {task_id!r}")
        completion = Completion(
            id=task_id, done=done, via=via, by=by, cost_cents=cost_cents, note=note
        )
        append_completion(self.data_dir, completion)
        if do_commit:
            commit(self.data_dir, f"complete {task_id} on {done.isoformat()}")
        return completion

    def report(self, kind: ReportKind, *, today: date) -> dict[str, object]:
        tasks, _vendors, completions = load_all(self.data_dir)
        if kind is ReportKind.SPEND_BY_TASK:
            spend: dict[str, int] = {}
            for c in completions:
                if c.cost_cents is not None:
                    spend[c.id] = spend.get(c.id, 0) + c.cost_cents
            return {"spend_by_task": spend}
        if kind is ReportKind.PER_YEAR:
            per_year: dict[str, int] = {}
            for c in completions:
                if c.cost_cents is not None:
                    key = str(c.done.year)
                    per_year[key] = per_year.get(key, 0) + c.cost_cents
            return {"per_year": per_year}
        statuses = compute_status(tasks, completions, today)
        counts = {b.value: 0 for b in (Bucket.OVERDUE, Bucket.DUE, Bucket.PREP)}
        for s in statuses:
            if s.bucket.value in counts:
                counts[s.bucket.value] += 1
        return counts
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_service.py -v`
Expected: all pass.

- [ ] **Step 5: Lint + type-check**

Run: `uv run ruff check . && uv run pyright .`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/service.py tests/test_service.py
git commit -m "feat: add complete and report operations to service"
```

---

## Task 12: CLI (transport)

**Files:**
- Create: `src/lifemaint/cli.py`
- Test: `tests/test_cli.py`

Typer app. Each command resolves the data dir, calls the service, and renders text or `--json`. `today` defaults to the real current date but is overridable via `--today` (so tests are deterministic). `--cost` is accepted in dollars and converted to integer cents.

**Commands:** `list`, `due`, `done`, `history`, `vendors`, `export`, `report`.

- [ ] **Step 1: Write the failing tests**

Create `tests/test_cli.py`:
```python
from __future__ import annotations

import json

from typer.testing import CliRunner

from lifemaint._store import DataDir
from lifemaint.cli import app

runner = CliRunner()


def _env(data_dir: DataDir) -> dict[str, str]:
    return {"LM_DATA_DIR": str(data_dir.root)}


def test_list_json_lists_all_tasks(data_dir: DataDir) -> None:
    result = runner.invoke(app, ["list", "--today", "2026-06-06", "--json"], env=_env(data_dir))
    assert result.exit_code == 0
    payload = json.loads(result.stdout)
    assert {row["id"] for row in payload} == {"groceries", "clean-drains", "blow-out-sprinklers"}


def test_list_search_filters(data_dir: DataDir) -> None:
    result = runner.invoke(
        app, ["list", "-q", "drain", "--today", "2026-06-06", "--json"], env=_env(data_dir)
    )
    payload = json.loads(result.stdout)
    assert [row["id"] for row in payload] == ["clean-drains"]


def test_due_excludes_ok_tasks(data_dir: DataDir) -> None:
    result = runner.invoke(app, ["due", "--today", "2026-06-06", "--json"], env=_env(data_dir))
    ids = {row["id"] for row in json.loads(result.stdout)}
    assert "clean-drains" in ids
    assert "groceries" not in ids


def test_done_records_completion_with_cost_in_dollars(git_data_dir: DataDir) -> None:
    result = runner.invoke(
        app,
        ["done", "groceries", "--cost", "42.00", "--today", "2026-06-06"],
        env=_env(git_data_dir),
    )
    assert result.exit_code == 0
    hist = runner.invoke(
        app, ["history", "--id", "groceries", "--json"], env=_env(git_data_dir)
    )
    rows = json.loads(hist.stdout)
    assert rows[-1]["cost_cents"] == 4200


def test_done_unknown_task_exits_nonzero(data_dir: DataDir) -> None:
    result = runner.invoke(
        app, ["done", "nope", "--today", "2026-06-06", "--no-commit"], env=_env(data_dir)
    )
    assert result.exit_code != 0


def test_export_json_has_schema_version(data_dir: DataDir) -> None:
    result = runner.invoke(app, ["export", "--today", "2026-06-06"], env=_env(data_dir))
    payload = json.loads(result.stdout)
    assert payload["schema_version"] == 1


def test_report_spend_by_task_json(data_dir: DataDir) -> None:
    result = runner.invoke(
        app, ["report", "spend-by-task", "--today", "2026-06-06", "--json"], env=_env(data_dir)
    )
    payload = json.loads(result.stdout)
    assert payload["spend_by_task"] == {"clean-drains": 28500}


def test_vendors_json(data_dir: DataDir) -> None:
    result = runner.invoke(app, ["vendors", "--json"], env=_env(data_dir))
    payload = json.loads(result.stdout)
    assert payload[0]["id"] == "roto-rooter"


def test_malformed_data_exits_nonzero_with_message(tmp_path: object) -> None:
    import pathlib

    p = pathlib.Path(str(tmp_path))
    (p / "tasks.yaml").write_text("- id: x\n  name: X\n  every: fortnightly\n")
    result = runner.invoke(app, ["list", "--today", "2026-06-06"], env={"LM_DATA_DIR": str(p)})
    assert result.exit_code != 0
    assert "tasks.yaml" in result.output
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_cli.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'lifemaint.cli'`.

- [ ] **Step 3: Write the implementation**

Create `src/lifemaint/cli.py`:
```python
from __future__ import annotations

import json
import os
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Annotated

import typer

from lifemaint._store import DataDir
from lifemaint.errors import LifemaintError
from lifemaint.service import ReportKind, Service
from lifemaint.status import Bucket, TaskStatus

app = typer.Typer(help="Track and complete recurring maintenance tasks.", no_args_is_help=True)

TodayOpt = Annotated[str | None, typer.Option(help="Override today as YYYY-MM-DD (testing).")]
JsonOpt = Annotated[bool, typer.Option("--json", help="Emit JSON instead of text.")]


def _data_dir() -> DataDir:
    env = os.environ.get("LM_DATA_DIR")
    root = Path(env) if env else Path.cwd() / "data"
    return DataDir(root=root)


def _today(value: str | None) -> date:
    return date.fromisoformat(value) if value else date.today()


def _service() -> Service:
    return Service(_data_dir())


def _fail(message: str) -> None:
    typer.echo(message, err=True)
    raise typer.Exit(code=1)


def _status_rows(statuses: list[TaskStatus]) -> list[dict[str, object]]:
    return [
        {
            "id": s.task.id,
            "name": s.task.name,
            "bucket": s.bucket.value,
            "last_done": s.last_done.isoformat() if s.last_done else None,
            "next_due": s.next_due.isoformat(),
            "prep_due": s.prep_due.isoformat() if s.prep_due else None,
        }
        for s in statuses
    ]


def _print_statuses(statuses: list[TaskStatus], *, as_json: bool) -> None:
    if as_json:
        typer.echo(json.dumps(_status_rows(statuses)))
        return
    if not statuses:
        typer.echo("Nothing to show.")
        return
    for s in statuses:
        marker = {Bucket.OVERDUE: "!!", Bucket.DUE: "->", Bucket.PREP: "..", Bucket.OK: "  "}[s.bucket]
        typer.echo(f"{marker} {s.task.id:<22} {s.bucket.value:<8} next: {s.next_due.isoformat()}")


@app.command(name="list")
def list_cmd(
    today: TodayOpt = None,
    as_json: JsonOpt = False,
    query: Annotated[str | None, typer.Option("-q", "--query", help="Search id/name/notes/vendor.")] = None,
    due_only: Annotated[bool, typer.Option("--due", help="Only OVERDUE or DUE.")] = False,
    overdue_only: Annotated[bool, typer.Option("--overdue", help="Only OVERDUE.")] = False,
) -> None:
    only: set[Bucket] | None = None
    if overdue_only:
        only = {Bucket.OVERDUE}
    elif due_only:
        only = {Bucket.OVERDUE, Bucket.DUE}
    try:
        statuses = _service().list_tasks(today=_today(today), query=query, only=only)
    except LifemaintError as e:
        _fail(str(e))
        return
    _print_statuses(statuses, as_json=as_json)


@app.command()
def due(today: TodayOpt = None, as_json: JsonOpt = False) -> None:
    try:
        statuses = _service().due(today=_today(today))
    except LifemaintError as e:
        _fail(str(e))
        return
    _print_statuses(statuses, as_json=as_json)


@app.command()
def done(
    task_id: Annotated[str, typer.Argument(help="Task id to mark complete.")],
    today: TodayOpt = None,
    on: Annotated[str | None, typer.Option(help="Completion date YYYY-MM-DD (default today).")] = None,
    by: Annotated[str, typer.Option(help="'self' or a vendor id.")] = "self",
    cost: Annotated[str | None, typer.Option(help="Cost in dollars, e.g. 42.00.")] = None,
    note: Annotated[str | None, typer.Option(help="Free-text note.")] = None,
    commit: Annotated[bool, typer.Option("--commit/--no-commit", help="Git-commit the change.")] = True,
) -> None:
    done_date = _today(on) if on else _today(today)
    cost_cents: int | None = None
    if cost is not None:
        try:
            cost_cents = int((Decimal(cost) * 100).to_integral_value())
        except InvalidOperation:
            _fail(f"invalid --cost {cost!r}; expected a dollar amount like 42.00")
            return
    try:
        completion = _service().complete(
            task_id, done=done_date, via="cli", by=by, cost_cents=cost_cents, note=note,
            do_commit=commit,
        )
    except LifemaintError as e:
        _fail(str(e))
        return
    typer.echo(f"Recorded: {completion.id} done {completion.done.isoformat()} (by {completion.by}).")


@app.command()
def history(
    task_id: Annotated[str | None, typer.Option("--id", help="Filter by task id.")] = None,
    since: Annotated[str | None, typer.Option(help="Only on/after YYYY-MM-DD.")] = None,
    as_json: JsonOpt = False,
) -> None:
    since_date = date.fromisoformat(since) if since else None
    try:
        rows = _service().history(task_id=task_id, since=since_date)
    except LifemaintError as e:
        _fail(str(e))
        return
    payload = [
        {
            "id": c.id,
            "done": c.done.isoformat(),
            "via": c.via,
            "by": c.by,
            "cost_cents": c.cost_cents,
            "note": c.note,
        }
        for c in rows
    ]
    if as_json:
        typer.echo(json.dumps(payload))
        return
    for row in payload:
        typer.echo(f"{row['done']}  {row['id']:<22} by {row['by']}  {row['note'] or ''}")


@app.command()
def vendors(as_json: JsonOpt = False) -> None:
    try:
        items = _service().vendors()
    except LifemaintError as e:
        _fail(str(e))
        return
    payload = [
        {"id": v.id, "name": v.name, "phone": v.phone, "email": v.email, "notes": v.notes}
        for v in items
    ]
    if as_json:
        typer.echo(json.dumps(payload))
        return
    for v in items:
        typer.echo(f"{v.id:<16} {v.name}  {v.phone or ''}")


@app.command()
def export(today: TodayOpt = None) -> None:
    try:
        payload = _service().export(today=_today(today))
    except LifemaintError as e:
        _fail(str(e))
        return
    typer.echo(json.dumps(payload, indent=2))


@app.command()
def report(
    kind: Annotated[ReportKind, typer.Argument(help="Which summary.")],
    today: TodayOpt = None,
    as_json: JsonOpt = False,
) -> None:
    try:
        payload = _service().report(kind, today=_today(today))
    except LifemaintError as e:
        _fail(str(e))
        return
    if as_json:
        typer.echo(json.dumps(payload))
        return
    for key, value in payload.items():
        typer.echo(f"{key}: {value}")
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_cli.py -v`
Expected: all pass.

- [ ] **Step 5: Full suite + lint + type-check**

Run:
```bash
uv run pytest
uv run ruff check .
uv run pyright .
```
Expected: all tests pass; ruff clean; pyright 0 errors.

- [ ] **Step 6: Commit**

```bash
git add src/lifemaint/cli.py tests/test_cli.py
git commit -m "feat: add typer CLI for list/due/done/history/vendors/export/report"
```

---

## Task 13: Public surface, example data, and README

**Files:**
- Modify: `src/lifemaint/__init__.py`
- Create: `data/tasks.yaml`, `data/vendors.yaml`, `data/completions.jsonl`, `README.md`

- [ ] **Step 1: Export the public surface**

Replace `src/lifemaint/__init__.py`:
```python
from __future__ import annotations

from lifemaint.errors import (
    DataFileError,
    LifemaintError,
    ScheduleParseError,
    UnknownVendorError,
)
from lifemaint.service import ReportKind, Service
from lifemaint.status import Bucket, TaskStatus, compute_status

__all__ = [
    "Bucket",
    "DataFileError",
    "LifemaintError",
    "ReportKind",
    "ScheduleParseError",
    "Service",
    "TaskStatus",
    "UnknownVendorError",
    "compute_status",
]
```

- [ ] **Step 2: Verify the public imports resolve**

Run: `uv run python -c "import lifemaint; print(sorted(lifemaint.__all__))"`
Expected: prints the list above with no import error.

- [ ] **Step 3: Create example data files**

Create `data/tasks.yaml`:
```yaml
- id: groceries
  name: Grocery shopping
  every: weekly

- id: clean-gutters
  name: Clean out gutters
  every: 6 months
  lead_time: 2 weeks
  prep:
    - Check ladder is sound
    - Buy gutter scoop if missing
  notes: Back side clogs worst.

- id: blow-out-sprinklers
  name: Blow out sprinkler lines
  every: yearly
  on: "10-15"
  lead_time: 2 weeks
  vendor: green-lawn

- id: clean-drains
  name: Clean out drains
  every: yearly
  lead_time: 2 weeks
  vendor: roto-rooter
```

Create `data/vendors.yaml`:
```yaml
- id: roto-rooter
  name: Roto-Rooter
  phone: "555-123-4567"
  notes: Ask for Dave; flat rate for main line.

- id: green-lawn
  name: Green Lawn Irrigation
  phone: "555-987-6543"
```

Create an empty `data/completions.jsonl`:
```bash
: > data/completions.jsonl
```

- [ ] **Step 4: Smoke-test the CLI against the example data**

Run:
```bash
uv run lm list --today 2026-06-06
uv run lm due --today 2026-06-06 --json
```
Expected: `list` prints four tasks; `due` prints JSON (several tasks due/overdue against a never-completed dataset).

- [ ] **Step 5: Write the README**

Create `README.md`:
```markdown
# life-maintenance (`lm`)

Track and complete recurring home/life maintenance tasks. Git files are the
source of truth; the `lm` CLI is the canonical interface (everything supports
`--json`). See `docs/specs/2026-06-06-life-maintenance-tracker-design.md`.

## Setup

```bash
uv sync --dev
```

## Data

Set `LM_DATA_DIR` to the git repo holding your data (defaults to `./data`):

- `tasks.yaml` — task definitions (relative `every:` or fixed `every: yearly` + `on:`).
- `vendors.yaml` — service contacts referenced by task `vendor:`.
- `completions.jsonl` — append-only log; `lm done` writes here and git-commits.

## Commands

```bash
lm list [-q TERM] [--due] [--overdue] [--json]
lm due [--json]
lm done <id> [--by V] [--cost 42.00] [--note "..."] [--on YYYY-MM-DD] [--no-commit]
lm history [--id X] [--since YYYY-MM-DD] [--json]
lm vendors [--json]
lm export
lm report {spend-by-task|per-year|overdue-count} [--json]
```

## Development

```bash
uv run pytest
uv run ruff check .
uv run pyright .
```
```

- [ ] **Step 6: Final full verification**

Run:
```bash
uv run pytest
uv run ruff check .
uv run pyright .
```
Expected: all tests pass; ruff clean; pyright 0 errors.

- [ ] **Step 7: Commit**

```bash
git add src/lifemaint/__init__.py data/ README.md
git commit -m "feat: public surface, example data, and README"
```

---

## Done — Phase 1 complete

The `lm` CLI now tracks tasks (relative + fixed schedules), records completions with vendor/cost into a git-committed append-only log, surfaces what's due/overdue/prep, searches, and reports — all backed by a pure, exhaustively-tested engine. This is the foundation a Phase 2 notifier (likely a scheduled agent calling `lm due --json`) will consume.
```
