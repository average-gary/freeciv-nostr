# Freeciv Determinism Audit Report

**Issue**: #4 — Phase 0.4: Determinism Verification
**Date**: 2026-02-23
**Updated**: 2026-02-25
**Status**: All actionable fixes applied

## Overview

This document catalogs all sources of non-determinism in the Freeciv game engine
that would cause two identical runs (same seeds, same inputs) to produce different
game states. Findings are organized by category and severity.

## Tooling

- `tests/compare_traces.py` — Compares two packet trace files byte-by-byte,
  ignoring timestamps and connection IDs.
- `tests/run_determinism_check.sh` — Runs two identical autogames and compares
  their packet traces.
- `.github/workflows/determinism.yml` — CI job that runs the determinism check.

## Test Configuration

For deterministic replay, the autogame must use:
```
set gameseed 42
set mapseed 42
set timeout -1
set unitwaittime 0
```

---

## HIGH Severity

These are likely to cause different game outcomes between identical runs.

### H1: `startpos_hash` Pointer-Based Iteration — FIXED (prior PR)

- **File**: `server/srv_main.c:2649-2655, 2818-2834`
- **Impact**: Nation assignment order during `generate_players()`
- **Root cause**: `startpos_hash` uses `struct startpos *` as key with no custom
  hash function. Default is `(intptr_t)key` — hashing by raw pointer address.
  Iteration at lines 2818-2834 assigns nations using `fc_rand(++i)` tiebreaker.
  Different memory layouts → different iteration order → different nation assignments
  → different game outcomes.
- **Fix**: Added a deterministic hash function based on `startpos_tile()` index.

### H2: Wall-Clock Time in Unit Action Timing — WORKAROUND

- **File**: `server/unittools.c:5060, 5083`
- **Impact**: Whether units can act depends on `time(nullptr)` when `unitwaittime > 0`
- **Root cause**: `unit_can_do_action_now()` reads system time and compares against
  `punit->server.action_timestamp`. `unit_did_action()` stores `time(nullptr)`.
  Two runs with different execution speeds will allow/deny actions at different points.
- **Workaround**: Set `unitwaittime 0` for deterministic testing.
- **Future fix**: Use game-turn-based timing instead of wall-clock.

---

## MEDIUM Severity

Could cause different outcomes under certain conditions (cross-platform, different
compiler flags, edge cases).

### M1: Float Arithmetic in City Migration — FIXED

- **File**: `server/cityturn.c:4083-4158`, `common/city.h`
- **Impact**: Migration decisions between cities
- **Root cause**: `city_migration_score()` used `float` with `exp()`. Results
  compared between cities. FP non-determinism across compilers/optimization
  levels could cause different migration decisions.
- **Fix**: Converted `float` → `double` in `city_migration_score()` return type,
  local variables, and the `migration_score` field in `struct city`.

### M2: Float Arithmetic in City Illness — FIXED

- **File**: `common/city.c:2877-2888, 2923-2924`
- **Impact**: Plague probability
- **Root cause**: `get_trade_illness()` used `float` with `sqrt()`.
  `city_illness_calc()` used intermediate `float`. Results cast to `int` affect
  plague check.
- **Fix**: Converted `float` → `double` in `get_trade_illness()` and
  `city_illness_calc()`.

### M3: Double Arithmetic in Combat Win Chance — FIXED

- **File**: `common/combat.c:868-869`
- **Impact**: Which unit is chosen as defender
- **Root cause**: `win_chance()` uses `pow()`. Result multiplied by 100000 and
  truncated to `int` via cast at `get_defender()`. Edge cases could select different
  defender.
- **Fix**: Changed `(int)` cast to `lround()` for proper rounding instead of
  truncation.

### M4: Double Arithmetic in CM Tile Sorting — FIXED

- **File**: `common/aicore/cm.c:892-912, 948`
- **Impact**: Citizen tile assignment in city manager
- **Root cause**: `estimate_fitness()` returns `double`, fed into `qsort()`
  comparator. The comparator subtracted two `double` values and returned the
  result as `int`, causing truncation (e.g., 0.3 → 0 → unstable sort).
- **Fix**: Changed `return valueb - valuea` to proper comparison:
  `return (valueb > valuea) ? 1 : (valueb < valuea) ? -1 : 0`.

### M5: Double in Auto-Explorer — NO CODE CHANGE NEEDED

- **File**: `server/advisors/autoexplorer.c:306-372`
- **Impact**: Exploration target selection
- **Root cause**: Uses `log()` for goodness calculation. All variables are already
  `double` (not `float`). The risk is only cross-platform `log()` divergence.
- **Mitigation**: Compiler flags (`-ffp-contract=off -fno-fast-math`) ensure
  deterministic FP. No `float` → `double` conversion needed.

### M6: Float/Double in AI Evaluations — FIXED

- **Files**:
  - `ai/default/daimilitary.c:1414, 1627` — `float finishing_factor` → `double`
  - `ai/default/daidiplomat.c:568` — `(float)` cast → `(double)` cast
  - `ai/default/daidiplomacy.c:1621-1622` — `float aggr_sr, max_sr` → `double`
  - `ai/default/daidomestic.c:381` — `(float)income` cast → `(double)income`
- **Impact**: AI build orders, military targets, diplomatic stance
- **Root cause**: AI "want" calculations used `float` (7 significant digits) with
  `pow()`, `ceil()`, `sqrt()`. Using `double` (15 significant digits) reduces
  cross-platform divergence.
- **Fix**: Changed all `float` types/casts to `double` in AI evaluation code.

### M7: `nation_hash` Pointer-Based Iteration — FIXED

- **File**: `server/savegame/savegame3.c:3226-3238`
- **Impact**: Save file nation list ordering (not game state directly)
- **Root cause**: Uses `struct nation_type *` as key, pointer-based hashing.
- **Fix**: Added insertion sort by `nation_number()` before serialization.

### M8: Phase Timer / Turn Timeout — WORKAROUND

- **File**: `server/sernet.c:731-742, 928-934`
- **Impact**: Premature turn ending when `timeout > 0`
- **Root cause**: Wall-clock timer determines turn timeout.
- **Workaround**: Set `timeout 0` or `timeout -1` for deterministic testing.
  This is enforced in the test configuration above.

---

## LOW Severity

Unlikely to affect autogame outcomes, or display-only.

### L1: Unstable qsort in Reports — FIXED

- **File**: `server/report.c:308-312`
- **Impact**: Display-only (historian reports, top cities, endgame scores)
- **Fix**: Added `player_number()` tiebreaker to `secompare()`.

### L2: Unstable qsort in Island Ordering — ALREADY FIXED (upstream)

- **File**: `server/generator/startpos.c:252-257, 387`
- **Impact**: Start position assignment order when islands have equal goodness
- **Fix**: Already has island index tiebreaker in upstream code.

### L3: `tile_hash` Pointer-Based Hashing — NO FIX NEEDED

- **File**: `common/tile.h:79-82`
- **Impact**: Only used in `#ifdef SANITY_CHECKING` debug code
- **Fix**: Not needed — never affects game state.

### L4: Ruler Title Hash Pointer-Based Hashing — NO FIX NEEDED

- **File**: `common/government.h:36-39`
- **Impact**: Display-only (ruler title lookup)
- **Fix**: Not needed — never affects game state.

### L5: `fc_malloc` Without Zeroing — FIXED

- **Files**: `server/voting.c:353`, `server/unittools.c:3916`,
  `server/unithand.c:1465,6422,6426`
- **Impact**: Potential garbage in struct fields if initialization is incomplete
- **Fix**: Changed `fc_malloc()` → `fc_calloc(1, ...)` at highest-risk locations.

### L6: Timer Value in Save Files — FIXED

- **File**: `server/savegame/savegame3.c:2282-2287`
- **Impact**: Non-identical save files (wall-clock value embedded)
- **Fix**: Removed `timer_read_seconds()` from save output; now writes only
  `game.server.additional_phase_seconds` (deterministic component).

---

## Safe Areas (Confirmed Deterministic)

| Area | File | Notes |
|------|------|-------|
| PRNG implementation | `utility/rand.c` | Mitchell-Moore generator, fully deterministic from seed |
| Seed initialization | `server/srv_main.c:210-228` | Explicit seed → deterministic |
| Map generation | `server/generator/mapgen.c:1270-1403` | Re-seeds from game PRNG, restores state |
| `shuffle_players()` | `server/plrhand.c:2386-2406` | Fisher-Yates with `fc_rand()` |
| Lua random | `common/scriptcore/api_common_utilities.c:40-49` | Uses `fc_rand()` |
| AI trait init | `ai/aitraits.c:49` | Uses `fc_rand()` |
| AI scheduling | `ai/default/daicity.c:957, 2083` | Uses `fc_rand()` |
| TEX AI module | `ai/tex/*.c` | Delegates to default AI, no own randomness |
| `compare_iter_index` | `common/city.c:339-361` | Never returns 0, stable |
| `startpos_hash` in map.c | `common/map.c:50-54` | Uses `tile_hash_key()` → deterministic |
| `unit_virtual_create` | `common/unit.c:1694` | Uses `fc_calloc()`, zeroed |

---

## Summary of Changes

| ID | Severity | Status | Fix |
|----|----------|--------|-----|
| H1 | HIGH | FIXED | Deterministic startpos_hash (prior PR) |
| H2 | HIGH | WORKAROUND | Set `unitwaittime 0` |
| M1 | MEDIUM | FIXED | `float` → `double` in city migration |
| M2 | MEDIUM | FIXED | `float` → `double` in city illness |
| M3 | MEDIUM | FIXED | `(int)` → `lround()` in combat defender |
| M4 | MEDIUM | FIXED | Proper int comparison in CM qsort |
| M5 | MEDIUM | N/A | Already uses `double`; compiler flags suffice |
| M6 | MEDIUM | FIXED | `float` → `double` in AI evaluations |
| M7 | MEDIUM | FIXED | Nation hash sorted before serialization |
| M8 | MEDIUM | WORKAROUND | Set `timeout -1` |
| L1 | LOW | FIXED | Player ID tiebreaker in report qsort |
| L2 | LOW | FIXED | Already has tiebreaker (upstream) |
| L3 | LOW | N/A | Debug-only code |
| L4 | LOW | N/A | Display-only code |
| L5 | LOW | FIXED | `fc_malloc` → `fc_calloc` |
| L6 | LOW | FIXED | Deterministic timer in saves |
