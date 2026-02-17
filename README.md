freeciv-nostr
=============

[![Build Status](https://github.com/average-gary/freeciv-nostr/workflows/ci/badge.svg)](https://github.com/average-gary/freeciv-nostr/actions?query=workflow%3Aci)
[![License: GPL v2](https://img.shields.io/badge/License-GPL%20v2-blue.svg)](https://www.gnu.org/licenses/old-licenses/gpl-2.0.en.html)

A fork of [Freeciv](https://github.com/freeciv/freeciv) that replaces centralized client-server networking with decentralized peer-to-peer communication using [Nostr](https://nostr.com/) for cryptographic game state verification via hash chains and [Iroh](https://iroh.computer/) for direct P2P transport.

## Goals

- **Deterministic lockstep verification** - all players independently verify game state via hash chains published as Nostr events
- **secp256k1 identity** - players are identified by Nostr keypairs (npub/nsec)
- **NIP-46 remote signing** - support for external signer applications to manage keys
- **Iroh QUIC transport** - direct peer-to-peer connections with NAT traversal, replacing the traditional Freeciv client-server socket layer

## Project Board

Track progress at: https://github.com/users/average-gary/projects/2

## Building

This fork builds identically to upstream Freeciv. See the [doc](doc) directory for build instructions and documentation.

## License

This project is licensed under the GNU General Public License v2. It is a fork of [freeciv/freeciv](https://github.com/freeciv/freeciv) and retains the same license.
