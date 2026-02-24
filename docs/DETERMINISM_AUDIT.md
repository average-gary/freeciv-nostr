# Freeciv Determinism Audit Report

**Issue**: #4 — Phase 0.4: Determinism Verification
**Date**: 2026-02-23
**Status**: Audit complete, fixes pending

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

### H1: `startpos_hash` Pointer-Based Iteration

- **File**: `server/srv_main.c:2649-2655, 2818-2834`
- **Impact**: Nation assignment order during `generate_players()`
- **Root cause**: `startpos_hash` uses `struct startpos *` as key with no custom
  hash function. Default is `(intptr_t)key` — hashing by raw pointer address.
  Iteration at lines 2818-2834 assigns nations using `fc_rand(++i)` tiebreaker.
  Different memory layouts → different iteration order → different nation assignments
  → different game outcomes.
- **Fix**: Add a deterministic hash function based on `startpos_tile()` index.
  Or convert to an array sorted by tile index.

### H2: Wall-Clock Time in Unit Action Timing

- **File**: `server/unittools.c:5060, 5083`
- **Impact**: Whether units can act depends on `time(nullptr)` when `unitwaittime > 0`
- **Root cause**: `unit_can_do_action_now()` reads system time and compares against
  `punit->server.action_timestamp`. `unit_did_action()` stores `time(nullptr)`.
  Two runs with different execution speeds will allow/deny actions at different points.
- **Workaround**: Set `unitwaittime 0` for deterministic testing.
- **Fix**: Use game-turn-based timing instead of wall-clock.

---

## MEDIUM Severity

Could cause different outcomes under certain conditions (cross-platform, different
compiler flags, edge cases).

### M1: Float Arithmetic in City Migration

- **File**: `server/cityturn.c:4083-4158`
- **Impact**: Migration decisions between cities
- **Root cause**: `city_migration_score()` uses `float` with `exp()`. Results
  are compared between cities. FP non-determinism across compilers/optimization
  levels could cause different migration decisions.
- **Fix**: Convert to `double` or integer arithmetic.

### M2: Float Arithmetic in City Illness

- **File**: `common/city.c:2877-2888, 2923-2924`
- **Impact**: Plague probability
- **Root cause**: `get_trade_illness()` uses `float` with `sqrt()`.
  `city_illness_calc()` uses `exp()`. Results cast to `int` affect plague check.
- **Fix**: Convert to integer arithmetic or use `double`.

### M3: Double Arithmetic in Combat Win Chance

- **File**: `common/combat.c:334-406, 868-869`
- **Impact**: Which unit is chosen as defender
- **Root cause**: `win_chance()` uses `pow()`. Result multiplied by 100000 and
  truncated to `int` at `get_defender()`. Edge cases could select different defender.
- **Fix**: Ensure consistent compiler flags; consider integer scaling.

### M4: Double Arithmetic in CM Tile Sorting

- **File**: `common/aicore/cm.c:892-912, 1226-1227, 1770-1808`
- **Impact**: Citizen tile assignment in city manager
- **Root cause**: `estimate_fitness()` returns `double`, fed into `qsort()`
  comparator with 0.5 epsilon. Near-equal values could sort differently.
- **Fix**: Use integer-scaled comparisons.

### M5: Float in Auto-Explorer

- **File**: `server/advisors/autoexplorer.c:306-372`
- **Impact**: Exploration target selection
- **Root cause**: Uses `log()` for goodness calculation. Affects which tile a
  unit explores.
- **Fix**: Integer approximation or consistent compiler flags.

### M6: Float/Double in AI Evaluations

- **Files**:
  - `ai/default/daidomestic.c:381, 558`
  - `ai/default/daimilitary.c:93-115`
  - `ai/default/daidiplomacy.c:1163, 1621-1622`
  - `ai/default/daieffects.c:729`
  - `ai/default/daiunit.c:365, 509`
  - `ai/default/daidiplomat.c:568`
- **Impact**: AI build orders, military targets, diplomatic stance
- **Root cause**: AI "want" calculations use `double`/`float` with `pow()`,
  `ceil()`, etc. Results feed into integer comparisons for decisions.
- **Fix**: Compile with `-ffp-contract=off -fno-fast-math`; long-term, convert
  critical paths to integer arithmetic.

### M7: `nation_hash` Pointer-Based Iteration

- **File**: `common/nation.h:89-92`
- **Impact**: Save file nation list ordering (not game state directly)
- **Root cause**: Uses `struct nation_type *` as key, pointer-based hashing.
- **Fix**: Sort nations by index before serialization.

### M8: Phase Timer / Turn Timeout

- **File**: `server/sernet.c:731-742, 928-934`
- **Impact**: Premature turn ending when `timeout > 0`
- **Root cause**: Wall-clock timer determines turn timeout.
- **Workaround**: Set `timeout 0` or `timeout -1` for deterministic testing.

---

## LOW Severity

Unlikely to affect autogame outcomes, or display-only.

### L1: Unstable qsort in Reports

- **File**: `server/report.c:308-312, 356, 433, 1778`
- **Impact**: Display-only (historian reports, top cities, endgame scores)
- **Fix**: Add player ID tiebreaker.

### L2: Unstable qsort in Island Ordering

- **File**: `server/generator/startpos.c:252-257, 387`
- **Impact**: Start position assignment order when islands have equal goodness
- **Fix**: Add island index tiebreaker.

### L3: `tile_hash` Pointer-Based Hashing

- **File**: `common/tile.h:79-82`
- **Impact**: Only used in `#ifdef SANITY_CHECKING` debug code
- **Fix**: Not needed.

### L4: `ruler_title_hash` Pointer-Based Hashing

- **File**: `common/government.h:36-39`
- **Impact**: Display-only (ruler title lookup)
- **Fix**: Not needed.

### L5: `fc_malloc` Without Zeroing

- **Files**: `server/voting.c:353`, `server/unittools.c:3541`,
  `server/unithand.c:1465`, `server/gamehand.c:350-351`,
  `server/cityturn.c:3406,3503`, `server/advisors/advchoice.c:61`
- **Impact**: Potential garbage in struct fields if initialization is incomplete
- **Fix**: Use `fc_calloc()` as safety net.

### L6: Timer Value in Save Files

- **File**: `server/savegame/savegame3.c:2282-2286`
- **Impact**: Non-identical save files (wall-clock value embedded)
- **Fix**: Write a fixed value for deterministic saves.

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

## Recommended Fix Priority

1. **Fix H1** (`startpos_hash` pointer-based iteration) — likely root cause of
   any observed non-determinism in nation assignment
2. **Ensure test config** uses `unitwaittime 0`, `timeout -1` (workaround for H2, M8)
3. **Compile flags**: `-ffp-contract=off -fno-fast-math` for deterministic FP (mitigates M1-M6)
4. **Fix M1/M2**: Convert `city_migration_score()` and `city_illness_calc()` from
   `float` to `double` or integer
5. **Add tiebreakers** to unstable `qsort` comparators (L1, L2)
6. **Replace `fc_malloc` with `fc_calloc`** for struct allocations (L5)
