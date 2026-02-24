/***********************************************************************
 Freeciv - Copyright (C) 1996 - A Conditions of GPL
   This program is free software; you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation; either version 2, or (at your option)
   any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.
***********************************************************************/

#ifndef FC__STATE_HASH_H
#define FC__STATE_HASH_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>

/* Size of a SHA-256 hash in bytes. */
#define STATE_HASH_SIZE 32

/*
 * Compute a deterministic SHA-256 hash of the full game state.
 *
 * The hash covers (in canonical order):
 *   1. Global game info: turn, year, phase, global warming/cooling,
 *      global advances, great wonder owners
 *   2. RNG state: full Knuth lagged-Fibonacci table
 *   3. Map tiles: terrain, extras, ownership, continent, altitude
 *      (iterated in tile index order)
 *   4. Players: government, economy, diplomacy, wonders, multipliers
 *      (iterated in player number order)
 *   5. Cities: size, production, buildings, stocks, specialists
 *      (per player, sorted by city ID)
 *   6. Units: type, position, HP, moves, orders
 *      (per player, sorted by unit ID)
 *   7. Research: current research, bulbs, invention states
 *
 * The output is a 32-byte SHA-256 hash written to `hash_out`.
 * Returns 0 on success, nonzero on error.
 *
 * This function is safe to call from any thread, but the game state
 * must not be mutated concurrently (call during turn boundary).
 */
int fc_compute_state_hash(uint8_t hash_out[STATE_HASH_SIZE]);

/*
 * Compute the state hash and return it as a 64-character hex string.
 * The caller must provide a buffer of at least 65 bytes (64 hex + NUL).
 * Returns 0 on success, nonzero on error.
 */
int fc_compute_state_hash_hex(char *hex_out, size_t hex_out_size);

#ifdef __cplusplus
}
#endif

#endif /* FC__STATE_HASH_H */
