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

/*
 * Deterministic game state hashing for Nostr-based multiplayer verification.
 *
 * Computes a SHA-256 hash over the full game state in a canonical order.
 * The hash is computed at turn boundaries so all players can independently
 * verify they share the same game state.
 *
 * The SHA-256 implementation is self-contained (no external dependencies),
 * following the same pattern as utility/md5.c.
 */

#ifdef HAVE_CONFIG_H
#include <fc_config.h>
#endif

#include <stdlib.h>
#include <string.h>

/* utility */
#include "mem.h"
#include "rand.h"

/* common */
#include "city.h"
#include "game.h"
#include "government.h"
#include "improvement.h"
#include "map.h"
#include "player.h"
#include "requirements.h"
#include "research.h"
#include "tech.h"
#include "terrain.h"
#include "tile.h"
#include "unit.h"
#include "unitlist.h"
#include "unittype.h"
#include "world_object.h"

#include "state_hash.h"

/***********************************************************************
 *  Self-contained SHA-256 implementation
 *
 *  Based on FIPS PUB 180-4. Public domain.
 *  Streaming interface: sha256_init / sha256_update / sha256_final.
 ***********************************************************************/

struct sha256_ctx {
  uint32_t state[8];
  uint64_t count;            /* Total number of bytes processed */
  uint8_t  buf[64];          /* Partial block buffer */
};

/* SHA-256 round constants */
static const uint32_t K256[64] = {
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
  0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
  0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
  0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
  0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
};

#define RR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
#define CH(x, y, z)  (((x) & (y)) ^ (~(x) & (z)))
#define MAJ(x, y, z) (((x) & (y)) ^ ((x) & (z)) ^ ((y) & (z)))
#define EP0(x) (RR(x, 2) ^ RR(x, 13) ^ RR(x, 22))
#define EP1(x) (RR(x, 6) ^ RR(x, 11) ^ RR(x, 25))
#define SIG0(x) (RR(x, 7) ^ RR(x, 18) ^ ((x) >> 3))
#define SIG1(x) (RR(x, 17) ^ RR(x, 19) ^ ((x) >> 10))

/**********************************************************************//**
  Process one 64-byte block.
**************************************************************************/
static void sha256_transform(uint32_t state[8], const uint8_t block[64])
{
  uint32_t a, b, c, d, e, f, g, h;
  uint32_t W[64];
  int i;

  /* Prepare message schedule */
  for (i = 0; i < 16; i++) {
    W[i] = ((uint32_t)block[i * 4] << 24)
         | ((uint32_t)block[i * 4 + 1] << 16)
         | ((uint32_t)block[i * 4 + 2] << 8)
         | ((uint32_t)block[i * 4 + 3]);
  }
  for (i = 16; i < 64; i++) {
    W[i] = SIG1(W[i - 2]) + W[i - 7] + SIG0(W[i - 15]) + W[i - 16];
  }

  a = state[0]; b = state[1]; c = state[2]; d = state[3];
  e = state[4]; f = state[5]; g = state[6]; h = state[7];

  for (i = 0; i < 64; i++) {
    uint32_t t1 = h + EP1(e) + CH(e, f, g) + K256[i] + W[i];
    uint32_t t2 = EP0(a) + MAJ(a, b, c);

    h = g; g = f; f = e; e = d + t1;
    d = c; c = b; b = a; a = t1 + t2;
  }

  state[0] += a; state[1] += b; state[2] += c; state[3] += d;
  state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}

/**********************************************************************//**
  Initialize SHA-256 context.
**************************************************************************/
static void sha256_init(struct sha256_ctx *ctx)
{
  ctx->state[0] = 0x6a09e667;
  ctx->state[1] = 0xbb67ae85;
  ctx->state[2] = 0x3c6ef372;
  ctx->state[3] = 0xa54ff53a;
  ctx->state[4] = 0x510e527f;
  ctx->state[5] = 0x9b05688c;
  ctx->state[6] = 0x1f83d9ab;
  ctx->state[7] = 0x5be0cd19;
  ctx->count = 0;
  memset(ctx->buf, 0, sizeof(ctx->buf));
}

/**********************************************************************//**
  Feed data into the hash.
**************************************************************************/
static void sha256_update(struct sha256_ctx *ctx,
                          const uint8_t *data, size_t len)
{
  size_t buf_used = (size_t)(ctx->count & 63);
  size_t i = 0;

  ctx->count += len;

  /* Fill partial block */
  if (buf_used > 0) {
    size_t space = 64 - buf_used;

    if (len < space) {
      memcpy(ctx->buf + buf_used, data, len);
      return;
    }
    memcpy(ctx->buf + buf_used, data, space);
    sha256_transform(ctx->state, ctx->buf);
    i = space;
  }

  /* Process full blocks */
  for (; i + 64 <= len; i += 64) {
    sha256_transform(ctx->state, data + i);
  }

  /* Store remainder */
  if (i < len) {
    memcpy(ctx->buf, data + i, len - i);
  }
}

/**********************************************************************//**
  Finalize and output the 32-byte hash.
**************************************************************************/
static void sha256_final(struct sha256_ctx *ctx, uint8_t hash[32])
{
  size_t buf_used = (size_t)(ctx->count & 63);
  uint64_t bits = ctx->count * 8;
  int i;

  /* Padding: append 0x80 then zeros */
  ctx->buf[buf_used++] = 0x80;

  if (buf_used > 56) {
    memset(ctx->buf + buf_used, 0, 64 - buf_used);
    sha256_transform(ctx->state, ctx->buf);
    buf_used = 0;
  }
  memset(ctx->buf + buf_used, 0, 56 - buf_used);

  /* Append bit length in big-endian */
  ctx->buf[56] = (uint8_t)(bits >> 56);
  ctx->buf[57] = (uint8_t)(bits >> 48);
  ctx->buf[58] = (uint8_t)(bits >> 40);
  ctx->buf[59] = (uint8_t)(bits >> 32);
  ctx->buf[60] = (uint8_t)(bits >> 24);
  ctx->buf[61] = (uint8_t)(bits >> 16);
  ctx->buf[62] = (uint8_t)(bits >> 8);
  ctx->buf[63] = (uint8_t)(bits);
  sha256_transform(ctx->state, ctx->buf);

  /* Output hash in big-endian */
  for (i = 0; i < 8; i++) {
    hash[i * 4]     = (uint8_t)(ctx->state[i] >> 24);
    hash[i * 4 + 1] = (uint8_t)(ctx->state[i] >> 16);
    hash[i * 4 + 2] = (uint8_t)(ctx->state[i] >> 8);
    hash[i * 4 + 3] = (uint8_t)(ctx->state[i]);
  }
}

/***********************************************************************
 *  Hash feeding helpers
 *
 *  All multi-byte integers are fed in big-endian to ensure identical
 *  hashes across architectures.
 ***********************************************************************/

/**********************************************************************//**
  Feed a 32-bit integer in big-endian.
**************************************************************************/
static void hash_feed_int32(struct sha256_ctx *ctx, int32_t val)
{
  uint32_t u = (uint32_t)val;
  uint8_t buf[4];

  buf[0] = (uint8_t)(u >> 24);
  buf[1] = (uint8_t)(u >> 16);
  buf[2] = (uint8_t)(u >> 8);
  buf[3] = (uint8_t)(u);
  sha256_update(ctx, buf, 4);
}

/**********************************************************************//**
  Feed a 16-bit integer in big-endian.
**************************************************************************/
static void hash_feed_int16(struct sha256_ctx *ctx, int16_t val)
{
  uint16_t u = (uint16_t)val;
  uint8_t buf[2];

  buf[0] = (uint8_t)(u >> 8);
  buf[1] = (uint8_t)(u);
  sha256_update(ctx, buf, 2);
}

/**********************************************************************//**
  Feed a boolean as a single byte.
**************************************************************************/
static void hash_feed_bool(struct sha256_ctx *ctx, bool val)
{
  uint8_t b = val ? 1 : 0;

  sha256_update(ctx, &b, 1);
}

/**********************************************************************//**
  Feed a single byte.
**************************************************************************/
static void hash_feed_byte(struct sha256_ctx *ctx, uint8_t val)
{
  sha256_update(ctx, &val, 1);
}

/**********************************************************************//**
  Feed a raw byte array.
**************************************************************************/
static void hash_feed_bytes(struct sha256_ctx *ctx,
                            const uint8_t *data, size_t len)
{
  sha256_update(ctx, data, len);
}

/**********************************************************************//**
  Integer comparison for qsort (ascending).
**************************************************************************/
static int compare_ints(const void *a, const void *b)
{
  int ia = *(const int *)a;
  int ib = *(const int *)b;

  return (ia > ib) - (ia < ib);
}

/***********************************************************************
 *  Section 1: Global game info
 ***********************************************************************/

/**********************************************************************//**
  Hash global game parameters: turn, year, phase, warming/cooling, etc.
**************************************************************************/
static void hash_global_game_info(struct sha256_ctx *ctx)
{
  /* Section marker for unambiguous domain separation */
  hash_feed_byte(ctx, 0x01);

  hash_feed_int32(ctx, game.info.turn);
  hash_feed_int32(ctx, game.info.year);
  hash_feed_int32(ctx, game.info.phase);

  /* Global warming / nuclear winter accumulators */
  hash_feed_int32(ctx, game.info.globalwarming);
  hash_feed_int32(ctx, game.info.heating);
  hash_feed_int32(ctx, game.info.warminglevel);
  hash_feed_int32(ctx, game.info.nuclearwinter);
  hash_feed_int32(ctx, game.info.cooling);
  hash_feed_int32(ctx, game.info.coolinglevel);

  /* Global advances known (bool array) */
  hash_feed_int32(ctx, game.info.global_advance_count);

  /* Great wonder owners (array indexed by building ID) */
  {
    int i;

    for (i = 0; i < B_LAST; i++) {
      hash_feed_int16(ctx, (int16_t)game.info.great_wonder_owners[i]);
    }
  }
}

/***********************************************************************
 *  Section 2: RNG state
 ***********************************************************************/

/**********************************************************************//**
  Hash the full random number generator state.
  This is critical for verifying deterministic replay.
**************************************************************************/
static void hash_rng_state(struct sha256_ctx *ctx)
{
  RANDOM_STATE rstate;
  int i;

  hash_feed_byte(ctx, 0x02);

  if (!fc_rand_is_init()) {
    /* RNG not initialized — hash a sentinel */
    hash_feed_bool(ctx, FALSE);
    return;
  }

  hash_feed_bool(ctx, TRUE);
  rstate = fc_rand_state();

  for (i = 0; i < 56; i++) {
    hash_feed_int32(ctx, (int32_t)rstate.v[i]);
  }
  hash_feed_int32(ctx, rstate.j);
  hash_feed_int32(ctx, rstate.k);
  hash_feed_int32(ctx, rstate.x);
}

/***********************************************************************
 *  Section 3: Map tiles
 ***********************************************************************/

/**********************************************************************//**
  Hash all map tiles in index order.
  For each tile: terrain ID, extras bitvector, owner, continent, altitude.
**************************************************************************/
static void hash_map_tiles(struct sha256_ctx *ctx)
{
  hash_feed_byte(ctx, 0x03);

  /* Map dimensions for context */
  hash_feed_int32(ctx, wld.map.xsize);
  hash_feed_int32(ctx, wld.map.ysize);
  hash_feed_int32(ctx, wld.map.topology_id);

  whole_map_iterate(&(wld.map), ptile) {
    struct terrain *pterrain = tile_terrain(ptile);

    /* Terrain — use terrain number for stable ID across pointer changes */
    if (pterrain != NULL) {
      hash_feed_int16(ctx, (int16_t)terrain_number(pterrain));
    } else {
      hash_feed_int16(ctx, -1);
    }

    /* Extras bitvector — feed raw bytes */
    hash_feed_bytes(ctx, (const uint8_t *)&ptile->extras,
                    sizeof(ptile->extras));

    /* Owner (player number, or -1 for unowned) */
    if (ptile->owner != NULL) {
      hash_feed_int16(ctx, (int16_t)player_number(ptile->owner));
    } else {
      hash_feed_int16(ctx, -1);
    }

    /* Extras owner */
    if (ptile->extras_owner != NULL) {
      hash_feed_int16(ctx, (int16_t)player_number(ptile->extras_owner));
    } else {
      hash_feed_int16(ctx, -1);
    }

    hash_feed_int16(ctx, (int16_t)ptile->continent);
    hash_feed_int32(ctx, ptile->altitude);

    /* Worked status — which city (if any) is working this tile */
    if (ptile->worked != NULL) {
      hash_feed_int32(ctx, ptile->worked->id);
    } else {
      hash_feed_int32(ctx, -1);
    }

    /* Infrastructure placement in progress */
    if (ptile->placing != NULL) {
      hash_feed_int32(ctx, extra_number(ptile->placing));
    } else {
      hash_feed_int32(ctx, -1);
    }
    hash_feed_int32(ctx, ptile->infra_turns);
  } whole_map_iterate_end;
}

/***********************************************************************
 *  Section 4: Players
 ***********************************************************************/

/**********************************************************************//**
  Hash per-player state: government, economy, diplomacy, wonders,
  multipliers.  Players are iterated in player number order (stable).
**************************************************************************/
static void hash_players(struct sha256_ctx *ctx)
{
  hash_feed_byte(ctx, 0x04);
  hash_feed_int32(ctx, player_count());

  players_iterate(pplayer) {
    int plr_no = player_number(pplayer);

    hash_feed_int32(ctx, plr_no);
    hash_feed_bool(ctx, pplayer->is_alive);

    /* Government */
    if (pplayer->government != NULL) {
      hash_feed_int16(ctx, (int16_t)government_number(pplayer->government));
    } else {
      hash_feed_int16(ctx, -1);
    }

    /* Economy */
    hash_feed_int32(ctx, pplayer->economic.gold);
    hash_feed_int32(ctx, pplayer->economic.tax);
    hash_feed_int32(ctx, pplayer->economic.science);
    hash_feed_int32(ctx, pplayer->economic.luxury);
    hash_feed_int32(ctx, pplayer->economic.infra_points);

    /* Revolution */
    hash_feed_int32(ctx, pplayer->revolution_finishes);
    hash_feed_int32(ctx, pplayer->primary_capital_id);

    /* Culture */
    hash_feed_int32(ctx, pplayer->history);

    /* Turns alive */
    hash_feed_int32(ctx, pplayer->turns_alive);

    /* Wonders: city ID for each wonder owned by this player */
    {
      int i;

      for (i = 0; i < B_LAST; i++) {
        hash_feed_int32(ctx, pplayer->wonders[i]);
      }
    }

    /* Diplomacy with every other player */
    players_iterate(pother) {
      struct player_diplstate *ds = player_diplstate_get(pplayer, pother);

      hash_feed_byte(ctx, (uint8_t)ds->type);
      hash_feed_byte(ctx, (uint8_t)ds->max_state);
      hash_feed_int32(ctx, ds->first_contact_turn);
      hash_feed_byte(ctx, (uint8_t)ds->turns_left);
      hash_feed_byte(ctx, (uint8_t)ds->has_reason_to_cancel);
      hash_feed_byte(ctx, (uint8_t)ds->contact_turns_left);
    } players_iterate_end;
  } players_iterate_end;
}

/***********************************************************************
 *  Section 5: Cities
 ***********************************************************************/

/**********************************************************************//**
  Hash all cities, grouped by player (player number order).
  Within each player, cities are sorted by ID for determinism.
**************************************************************************/
static void hash_cities(struct sha256_ctx *ctx)
{
  hash_feed_byte(ctx, 0x05);

  players_iterate(pplayer) {
    int ncities = city_list_size(pplayer->cities);
    int *city_ids;
    int i;

    hash_feed_int32(ctx, player_number(pplayer));
    hash_feed_int32(ctx, ncities);

    if (ncities == 0) {
      continue;
    }

    /* Collect and sort city IDs for deterministic order */
    city_ids = fc_malloc(ncities * sizeof(int));
    i = 0;
    city_list_iterate(pplayer->cities, pcity) {
      city_ids[i++] = pcity->id;
    } city_list_iterate_end;

    qsort(city_ids, ncities, sizeof(int), compare_ints);

    for (i = 0; i < ncities; i++) {
      struct city *pcity = player_city_by_number(pplayer, city_ids[i]);
      int j;

      if (pcity == NULL) {
        continue;  /* Shouldn't happen, but be safe */
      }

      hash_feed_int32(ctx, pcity->id);

      /* Location */
      if (pcity->tile != NULL) {
        hash_feed_int32(ctx, tile_index(pcity->tile));
      } else {
        hash_feed_int32(ctx, -1);
      }

      /* Size */
      hash_feed_int32(ctx, (int32_t)city_size_get(pcity));

      /* Capital status */
      hash_feed_byte(ctx, (uint8_t)pcity->capital);

      /* Specialists */
      for (j = 0; j < SP_MAX; j++) {
        hash_feed_int32(ctx, (int32_t)pcity->specialists[j]);
      }

      /* Production outputs */
      for (j = 0; j < O_LAST; j++) {
        hash_feed_int32(ctx, pcity->surplus[j]);
        hash_feed_int32(ctx, pcity->prod[j]);
        hash_feed_int32(ctx, pcity->waste[j]);
      }

      /* Stocks */
      hash_feed_int32(ctx, pcity->food_stock);
      hash_feed_int32(ctx, pcity->shield_stock);

      /* Turn states */
      hash_feed_int32(ctx, pcity->airlift);
      hash_feed_bool(ctx, pcity->did_buy);
      hash_feed_bool(ctx, pcity->did_sell);
      hash_feed_bool(ctx, pcity->was_happy);
      hash_feed_int32(ctx, pcity->anarchy);
      hash_feed_int32(ctx, pcity->rapture);
      hash_feed_int32(ctx, pcity->turn_founded);
      hash_feed_int32(ctx, pcity->turn_last_built);

      /* Shield carry-overs */
      hash_feed_int32(ctx, pcity->before_change_shields);
      hash_feed_int32(ctx, pcity->caravan_shields);
      hash_feed_int32(ctx, pcity->disbanded_shields);
      hash_feed_int32(ctx, pcity->last_turns_shield_surplus);

      /* Buildings (B_LAST entries: turn built or I_NEVER/I_DESTROYED) */
      for (j = 0; j < B_LAST; j++) {
        hash_feed_int32(ctx, pcity->built[j].turn);
      }

      /* Current production target */
      hash_feed_int32(ctx, pcity->production.kind);
      hash_feed_int32(ctx, universal_number(&pcity->production));

      /* City radius */
      hash_feed_int32(ctx, pcity->city_radius_sq);

      /* Culture */
      hash_feed_int32(ctx, pcity->history);

      /* Steal count (diplomats) */
      hash_feed_int32(ctx, pcity->steal);

      /* Plague turn */
      hash_feed_int32(ctx, pcity->turn_plague);
    }

    free(city_ids);
  } players_iterate_end;
}

/***********************************************************************
 *  Section 6: Units
 ***********************************************************************/

/**********************************************************************//**
  Hash all units, grouped by player (player number order).
  Within each player, units are sorted by ID for determinism.
**************************************************************************/
static void hash_units(struct sha256_ctx *ctx)
{
  hash_feed_byte(ctx, 0x06);

  players_iterate(pplayer) {
    int nunits = unit_list_size(pplayer->units);
    int *unit_ids;
    int i;

    hash_feed_int32(ctx, player_number(pplayer));
    hash_feed_int32(ctx, nunits);

    if (nunits == 0) {
      continue;
    }

    /* Collect and sort unit IDs for deterministic order */
    unit_ids = fc_malloc(nunits * sizeof(int));
    i = 0;
    unit_list_iterate(pplayer->units, punit) {
      unit_ids[i++] = punit->id;
    } unit_list_iterate_end;

    qsort(unit_ids, nunits, sizeof(int), compare_ints);

    for (i = 0; i < nunits; i++) {
      struct unit *punit = player_unit_by_number(pplayer, unit_ids[i]);

      if (punit == NULL) {
        continue;  /* Shouldn't happen, but be safe */
      }

      hash_feed_int32(ctx, punit->id);

      /* Unit type — use type number for stable ID */
      hash_feed_int32(ctx, (int32_t)utype_number(unit_type_get(punit)));

      /* Location */
      if (punit->tile != NULL) {
        hash_feed_int32(ctx, tile_index(punit->tile));
      } else {
        hash_feed_int32(ctx, -1);
      }

      /* Core unit state */
      hash_feed_int32(ctx, punit->hp);
      hash_feed_int32(ctx, punit->veteran);
      hash_feed_int32(ctx, punit->moves_left);
      hash_feed_int32(ctx, punit->fuel);
      hash_feed_int32(ctx, punit->homecity);

      /* Activity */
      hash_feed_int32(ctx, (int32_t)punit->activity);
      hash_feed_int32(ctx, punit->activity_count);

      /* Activity target */
      if (punit->activity_target != NULL) {
        hash_feed_int32(ctx, extra_number(punit->activity_target));
      } else {
        hash_feed_int32(ctx, -1);
      }

      /* SSA controller */
      hash_feed_byte(ctx, (uint8_t)punit->ssa_controller);

      /* Movement flags */
      hash_feed_bool(ctx, punit->moved);
      hash_feed_bool(ctx, punit->paradropped);
      hash_feed_bool(ctx, punit->done_moving);
      hash_feed_bool(ctx, punit->stay);

      /* Transport */
      if (punit->transporter != NULL) {
        hash_feed_int32(ctx, punit->transporter->id);
      } else {
        hash_feed_int32(ctx, -1);
      }

      /* Upkeep */
      {
        int j;

        for (j = 0; j < O_LAST; j++) {
          hash_feed_int32(ctx, punit->upkeep[j]);
        }
      }

      /* Orders */
      hash_feed_bool(ctx, punit->has_orders);
      if (punit->has_orders) {
        int j;

        hash_feed_int32(ctx, punit->orders.length);
        hash_feed_int32(ctx, punit->orders.index);
        hash_feed_bool(ctx, punit->orders.repeat);
        hash_feed_bool(ctx, punit->orders.vigilant);

        for (j = 0; j < punit->orders.length; j++) {
          hash_feed_int32(ctx, (int32_t)punit->orders.list[j].order);
          hash_feed_int32(ctx, (int32_t)punit->orders.list[j].dir);
          hash_feed_int32(ctx, (int32_t)punit->orders.list[j].activity);
          hash_feed_int32(ctx, punit->orders.list[j].target);
          hash_feed_int32(ctx, punit->orders.list[j].sub_target);
          hash_feed_int32(ctx, punit->orders.list[j].action);
        }
      }

      /* Birth/form turn */
      hash_feed_int32(ctx, punit->birth_turn);
      hash_feed_int32(ctx, punit->current_form_turn);

      /* Battlegroup */
      hash_feed_int32(ctx, punit->battlegroup);

      /* Nationality */
      if (punit->nationality != NULL) {
        hash_feed_int32(ctx, player_number(punit->nationality));
      } else {
        hash_feed_int32(ctx, -1);
      }
    }

    free(unit_ids);
  } players_iterate_end;
}

/***********************************************************************
 *  Section 7: Research
 ***********************************************************************/

/**********************************************************************//**
  Hash all research states, iterated by research number order.
**************************************************************************/
static void hash_research(struct sha256_ctx *ctx)
{
  hash_feed_byte(ctx, 0x07);

  researches_iterate(presearch) {
    Tech_type_id j;
    Tech_type_id num_techs = advance_count();

    hash_feed_int32(ctx, research_number(presearch));

    hash_feed_int32(ctx, (int32_t)presearch->researching);
    hash_feed_int32(ctx, presearch->bulbs_researched);
    hash_feed_int32(ctx, presearch->techs_researched);
    hash_feed_int32(ctx, presearch->future_tech);
    hash_feed_int32(ctx, (int32_t)presearch->tech_goal);

    /* Saved research target (for turn-change penalty tracking) */
    hash_feed_int32(ctx, (int32_t)presearch->researching_saved);
    hash_feed_int32(ctx, presearch->bulbs_researching_saved);
    hash_feed_int32(ctx, presearch->free_bulbs);

    /* Per-tech invention state */
    hash_feed_int32(ctx, (int32_t)num_techs);
    for (j = 0; j < num_techs; j++) {
      hash_feed_byte(ctx,
                     (uint8_t)research_invention_state(presearch, j));
    }
  } researches_iterate_end;
}

/***********************************************************************
 *  Public API
 ***********************************************************************/

/**********************************************************************//**
  Compute a deterministic SHA-256 hash of the full game state.
  Returns 0 on success, nonzero on error.
**************************************************************************/
int fc_compute_state_hash(uint8_t hash_out[STATE_HASH_SIZE])
{
  struct sha256_ctx ctx;

  if (hash_out == NULL) {
    return -1;
  }

  sha256_init(&ctx);

  /* Version marker — if the canonical format ever changes, bump this.
   * This ensures hashes from different versions are never confused. */
  hash_feed_byte(&ctx, 0x01);  /* Format version 1 */

  /* Hash each section in canonical order */
  hash_global_game_info(&ctx);
  hash_rng_state(&ctx);
  hash_map_tiles(&ctx);
  hash_players(&ctx);
  hash_cities(&ctx);
  hash_units(&ctx);
  hash_research(&ctx);

  sha256_final(&ctx, hash_out);

  return 0;
}

/**********************************************************************//**
  Compute the state hash and return it as a hex string.
  The caller must provide a buffer of at least 65 bytes.
  Returns 0 on success, nonzero on error.
**************************************************************************/
int fc_compute_state_hash_hex(char *hex_out, size_t hex_out_size)
{
  static const char hex_chars[] = "0123456789abcdef";
  uint8_t hash[STATE_HASH_SIZE];
  int ret;
  int i;

  if (hex_out == NULL || hex_out_size < 65) {
    return -1;
  }

  ret = fc_compute_state_hash(hash);
  if (ret != 0) {
    return ret;
  }

  for (i = 0; i < STATE_HASH_SIZE; i++) {
    hex_out[i * 2]     = hex_chars[(hash[i] >> 4) & 0x0f];
    hex_out[i * 2 + 1] = hex_chars[hash[i] & 0x0f];
  }
  hex_out[64] = '\0';

  return 0;
}
