//! Action validation for the lockstep protocol.
//!
//! Each node independently validates incoming player actions before applying
//! them to local game state. Validation includes schema checks, basic
//! consistency checks, and consensus-based rejection.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use freeciv_nostr_core::actions::{PacketType, PlayerAction};

/// Result of validating a single action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationResult {
    /// Action is valid and should be applied.
    Valid,
    /// Action has an invalid payload schema.
    InvalidSchema(String),
    /// Action references a nonexistent entity (unit, city, etc.).
    InvalidTarget { entity_type: String, entity_id: i32 },
    /// Action is not permitted for this player (wrong owner, etc.).
    NotPermitted(String),
    /// Action violates game rules (insufficient resources, illegal move, etc.).
    RuleViolation(String),
    /// Action is structurally valid but impossible given current state (indicates desync).
    Impossible(String),
}

impl ValidationResult {
    /// Returns `true` if the action is valid.
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid)
    }

    /// Returns `true` if the action is invalid.
    pub fn is_invalid(&self) -> bool {
        !self.is_valid()
    }

    /// Whether this is a clear rule violation (as opposed to a possible desync).
    pub fn is_definite_violation(&self) -> bool {
        matches!(
            self,
            ValidationResult::InvalidSchema(_)
                | ValidationResult::NotPermitted(_)
                | ValidationResult::RuleViolation(_)
        )
    }

    /// Whether this could be caused by desync rather than cheating.
    pub fn is_possible_desync(&self) -> bool {
        matches!(
            self,
            ValidationResult::InvalidTarget { .. } | ValidationResult::Impossible(_)
        )
    }
}

/// Validate a [`PlayerAction`]'s payload against its expected schema.
///
/// This is a pure Rust check — no game state needed.
pub fn validate_schema(action: &PlayerAction) -> ValidationResult {
    match action.validate_payload() {
        Ok(()) => ValidationResult::Valid,
        Err(e) => ValidationResult::InvalidSchema(e),
    }
}

/// Validate basic structural constraints on an action.
///
/// Checks that are possible without full game state:
/// - [`PacketType`] is a known type (not "unknown")
pub fn validate_structure(action: &PlayerAction) -> ValidationResult {
    // Check packet type is known
    if action.packet_type.name() == "unknown" {
        return ValidationResult::InvalidSchema(format!(
            "unknown packet type: {}",
            action.packet_type.0
        ));
    }
    ValidationResult::Valid
}

/// Summary of validation for a batch of actions from one player.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchValidationResult {
    /// Player's public key (hex).
    pub player_pubkey: String,
    /// Turn number.
    pub turn: u32,
    /// Results for each action (in order).
    pub results: Vec<(PacketType, ValidationResult)>,
}

impl BatchValidationResult {
    /// Returns `true` if all actions in the batch are valid.
    pub fn all_valid(&self) -> bool {
        self.results.iter().all(|(_pt, r)| r.is_valid())
    }

    /// Returns the invalid actions.
    pub fn invalid_actions(&self) -> Vec<&(PacketType, ValidationResult)> {
        self.results
            .iter()
            .filter(|(_pt, r)| r.is_invalid())
            .collect()
    }
}

/// Validate a batch of actions from a single player.
///
/// Performs schema validation and structural checks for each action.
/// Game-state-dependent validation (target existence, rule compliance)
/// must be performed by the C game engine via FFI callbacks.
pub fn validate_action_batch(
    player_pubkey: &str,
    turn: u32,
    actions: &[PlayerAction],
) -> BatchValidationResult {
    let results: Vec<(PacketType, ValidationResult)> = actions
        .iter()
        .map(|action| {
            let schema_result = validate_schema(action);
            if schema_result.is_invalid() {
                return (action.packet_type, schema_result);
            }
            let struct_result = validate_structure(action);
            if struct_result.is_invalid() {
                return (action.packet_type, struct_result);
            }
            (action.packet_type, ValidationResult::Valid)
        })
        .collect();

    BatchValidationResult {
        player_pubkey: player_pubkey.to_string(),
        turn,
        results,
    }
}

/// Consensus-based rejection: collect validation results from multiple nodes
/// and determine if an action should be definitively rejected.
#[derive(Debug)]
pub struct ConsensusValidator {
    /// Total number of nodes.
    num_nodes: usize,
    /// Per-action validation results from each node.
    /// Key: (player_pubkey, action_index), Value: Vec of (node_pubkey, result)
    votes: HashMap<(String, usize), Vec<(String, ValidationResult)>>,
}

/// Decision from consensus validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsensusDecision {
    /// All nodes agree the action is valid.
    Accept,
    /// N-1 or more nodes reject — definitively invalid.
    Reject {
        rejection_count: usize,
        total: usize,
    },
    /// Some nodes reject, but not enough for consensus — possible desync.
    Disputed {
        rejection_count: usize,
        total: usize,
    },
    /// Not enough votes yet.
    Pending { votes_received: usize, total: usize },
}

impl ConsensusValidator {
    /// Create a new consensus validator for the given number of nodes.
    pub fn new(num_nodes: usize) -> Self {
        Self {
            num_nodes,
            votes: HashMap::new(),
        }
    }

    /// Submit a validation result from a node.
    pub fn submit_vote(
        &mut self,
        player_pubkey: &str,
        action_index: usize,
        node_pubkey: &str,
        result: ValidationResult,
    ) {
        let key = (player_pubkey.to_string(), action_index);
        self.votes
            .entry(key)
            .or_default()
            .push((node_pubkey.to_string(), result));
    }

    /// Get the consensus decision for a specific action.
    pub fn decide(&self, player_pubkey: &str, action_index: usize) -> ConsensusDecision {
        let key = (player_pubkey.to_string(), action_index);
        let votes = match self.votes.get(&key) {
            Some(v) => v,
            None => {
                return ConsensusDecision::Pending {
                    votes_received: 0,
                    total: self.num_nodes,
                };
            }
        };

        if votes.len() < self.num_nodes {
            return ConsensusDecision::Pending {
                votes_received: votes.len(),
                total: self.num_nodes,
            };
        }

        let rejection_count = votes.iter().filter(|(_, r)| r.is_invalid()).count();

        if rejection_count == 0 {
            ConsensusDecision::Accept
        } else if rejection_count >= self.num_nodes.saturating_sub(1).max(1) {
            ConsensusDecision::Reject {
                rejection_count,
                total: self.num_nodes,
            }
        } else {
            ConsensusDecision::Disputed {
                rejection_count,
                total: self.num_nodes,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeciv_nostr_core::actions::PacketType;

    /// Helper to build a `PlayerAction` with a given packet type and JSON payload.
    fn make_action(packet_type: PacketType, payload: serde_json::Value) -> PlayerAction {
        PlayerAction {
            packet_type,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload,
        }
    }

    // -- ValidationResult methods -----------------------------------------

    #[test]
    fn validation_result_valid_methods() {
        let r = ValidationResult::Valid;
        assert!(r.is_valid());
        assert!(!r.is_invalid());
        assert!(!r.is_definite_violation());
        assert!(!r.is_possible_desync());
    }

    #[test]
    fn validation_result_invalid_schema_methods() {
        let r = ValidationResult::InvalidSchema("bad".into());
        assert!(!r.is_valid());
        assert!(r.is_invalid());
        assert!(r.is_definite_violation());
        assert!(!r.is_possible_desync());
    }

    #[test]
    fn validation_result_invalid_target_methods() {
        let r = ValidationResult::InvalidTarget {
            entity_type: "unit".into(),
            entity_id: 42,
        };
        assert!(!r.is_valid());
        assert!(r.is_invalid());
        assert!(!r.is_definite_violation());
        assert!(r.is_possible_desync());
    }

    #[test]
    fn validation_result_not_permitted_methods() {
        let r = ValidationResult::NotPermitted("wrong owner".into());
        assert!(!r.is_valid());
        assert!(r.is_invalid());
        assert!(r.is_definite_violation());
        assert!(!r.is_possible_desync());
    }

    #[test]
    fn validation_result_rule_violation_methods() {
        let r = ValidationResult::RuleViolation("no gold".into());
        assert!(!r.is_valid());
        assert!(r.is_invalid());
        assert!(r.is_definite_violation());
        assert!(!r.is_possible_desync());
    }

    #[test]
    fn validation_result_impossible_methods() {
        let r = ValidationResult::Impossible("unit already dead".into());
        assert!(!r.is_valid());
        assert!(r.is_invalid());
        assert!(!r.is_definite_violation());
        assert!(r.is_possible_desync());
    }

    // -- Schema validation ------------------------------------------------

    #[test]
    fn validate_schema_valid_unit_do_action() {
        let action = make_action(
            PacketType::UNIT_DO_ACTION,
            serde_json::json!({
                "unit_id": 1,
                "target_id": 2,
                "sub_target": 0,
                "action_type": 3
            }),
        );
        assert_eq!(validate_schema(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_schema_invalid_payload() {
        let action = make_action(
            PacketType::UNIT_DO_ACTION,
            serde_json::json!({"wrong_field": true}),
        );
        let result = validate_schema(&action);
        assert!(matches!(result, ValidationResult::InvalidSchema(_)));
    }

    #[test]
    fn validate_schema_valid_city_buy() {
        let action = make_action(PacketType::CITY_BUY, serde_json::json!({"city_id": 10}));
        assert_eq!(validate_schema(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_schema_valid_player_ready() {
        let action = make_action(
            PacketType::PLAYER_READY,
            serde_json::json!({"is_ready": true}),
        );
        assert_eq!(validate_schema(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_schema_valid_chat_msg() {
        let action = make_action(
            PacketType::CHAT_MSG_REQ,
            serde_json::json!({"message": "hello"}),
        );
        assert_eq!(validate_schema(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_schema_valid_spaceship_launch() {
        let action = make_action(PacketType::SPACESHIP_LAUNCH, serde_json::json!({}));
        assert_eq!(validate_schema(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_schema_unknown_packet_type() {
        let action = make_action(PacketType(9999), serde_json::json!({}));
        let result = validate_schema(&action);
        assert!(matches!(result, ValidationResult::InvalidSchema(_)));
    }

    // -- Structural validation --------------------------------------------

    #[test]
    fn validate_structure_known_packet_type() {
        let action = make_action(PacketType::UNIT_ORDERS, serde_json::json!({}));
        assert_eq!(validate_structure(&action), ValidationResult::Valid);
    }

    #[test]
    fn validate_structure_unknown_packet_type() {
        let action = make_action(PacketType(9999), serde_json::json!({}));
        let result = validate_structure(&action);
        assert!(matches!(result, ValidationResult::InvalidSchema(_)));
        if let ValidationResult::InvalidSchema(msg) = result {
            assert!(msg.contains("9999"));
        }
    }

    // -- Batch validation -------------------------------------------------

    #[test]
    fn batch_all_valid() {
        let actions = vec![
            make_action(
                PacketType::UNIT_DO_ACTION,
                serde_json::json!({
                    "unit_id": 1, "target_id": 2, "sub_target": 0, "action_type": 3
                }),
            ),
            make_action(PacketType::CITY_BUY, serde_json::json!({"city_id": 10})),
        ];
        let result = validate_action_batch("alice", 1, &actions);
        assert!(result.all_valid());
        assert_eq!(result.results.len(), 2);
        assert!(result.invalid_actions().is_empty());
        assert_eq!(result.player_pubkey, "alice");
        assert_eq!(result.turn, 1);
    }

    #[test]
    fn batch_mixed_valid_invalid() {
        let actions = vec![
            make_action(PacketType::CITY_BUY, serde_json::json!({"city_id": 10})),
            make_action(
                PacketType::UNIT_DO_ACTION,
                serde_json::json!({"wrong": true}),
            ),
            make_action(PacketType(9999), serde_json::json!({})),
        ];
        let result = validate_action_batch("bob", 5, &actions);
        assert!(!result.all_valid());
        assert_eq!(result.results.len(), 3);
        assert_eq!(result.invalid_actions().len(), 2);

        // First action is valid
        assert!(result.results[0].1.is_valid());
        // Second has invalid schema (wrong fields)
        assert!(matches!(
            result.results[1].1,
            ValidationResult::InvalidSchema(_)
        ));
        // Third has unknown packet type (structural check catches it first via schema)
        assert!(matches!(
            result.results[2].1,
            ValidationResult::InvalidSchema(_)
        ));
    }

    #[test]
    fn batch_empty_actions() {
        let result = validate_action_batch("alice", 1, &[]);
        assert!(result.all_valid());
        assert!(result.results.is_empty());
        assert!(result.invalid_actions().is_empty());
    }

    // -- Consensus validation ---------------------------------------------

    #[test]
    fn consensus_all_accept() {
        let mut cv = ConsensusValidator::new(3);
        for i in 0..3 {
            cv.submit_vote("alice", 0, &format!("node{i}"), ValidationResult::Valid);
        }
        assert_eq!(cv.decide("alice", 0), ConsensusDecision::Accept);
    }

    #[test]
    fn consensus_all_reject() {
        let mut cv = ConsensusValidator::new(3);
        for i in 0..3 {
            cv.submit_vote(
                "alice",
                0,
                &format!("node{i}"),
                ValidationResult::InvalidSchema("bad".into()),
            );
        }
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Reject {
                rejection_count: 3,
                total: 3
            }
        ));
    }

    #[test]
    fn consensus_n_minus_1_reject() {
        let mut cv = ConsensusValidator::new(3);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        cv.submit_vote(
            "alice",
            0,
            "node1",
            ValidationResult::InvalidSchema("bad".into()),
        );
        cv.submit_vote(
            "alice",
            0,
            "node2",
            ValidationResult::RuleViolation("cheat".into()),
        );
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Reject {
                rejection_count: 2,
                total: 3
            }
        ));
    }

    #[test]
    fn consensus_disputed() {
        let mut cv = ConsensusValidator::new(4);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        cv.submit_vote("alice", 0, "node1", ValidationResult::Valid);
        cv.submit_vote(
            "alice",
            0,
            "node2",
            ValidationResult::InvalidSchema("bad".into()),
        );
        cv.submit_vote("alice", 0, "node3", ValidationResult::Valid);
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Disputed {
                rejection_count: 1,
                total: 4
            }
        ));
    }

    #[test]
    fn consensus_pending_no_votes() {
        let cv = ConsensusValidator::new(3);
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Pending {
                votes_received: 0,
                total: 3
            }
        ));
    }

    #[test]
    fn consensus_pending_partial_votes() {
        let mut cv = ConsensusValidator::new(3);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Pending {
                votes_received: 1,
                total: 3
            }
        ));
    }

    #[test]
    fn consensus_separate_actions() {
        let mut cv = ConsensusValidator::new(2);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        cv.submit_vote("alice", 0, "node1", ValidationResult::Valid);
        cv.submit_vote(
            "alice",
            1,
            "node0",
            ValidationResult::InvalidSchema("bad".into()),
        );
        cv.submit_vote(
            "alice",
            1,
            "node1",
            ValidationResult::InvalidSchema("bad".into()),
        );
        assert_eq!(cv.decide("alice", 0), ConsensusDecision::Accept);
        assert!(matches!(
            cv.decide("alice", 1),
            ConsensusDecision::Reject { .. }
        ));
    }

    #[test]
    fn consensus_separate_players() {
        let mut cv = ConsensusValidator::new(2);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        cv.submit_vote("alice", 0, "node1", ValidationResult::Valid);
        cv.submit_vote(
            "bob",
            0,
            "node0",
            ValidationResult::RuleViolation("cheat".into()),
        );
        cv.submit_vote(
            "bob",
            0,
            "node1",
            ValidationResult::RuleViolation("cheat".into()),
        );
        assert_eq!(cv.decide("alice", 0), ConsensusDecision::Accept);
        assert!(matches!(
            cv.decide("bob", 0),
            ConsensusDecision::Reject { .. }
        ));
    }

    #[test]
    fn consensus_two_nodes_one_reject() {
        // With 2 nodes, N-1 = 1, so a single rejection meets the threshold.
        let mut cv = ConsensusValidator::new(2);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        cv.submit_vote(
            "alice",
            0,
            "node1",
            ValidationResult::InvalidSchema("bad".into()),
        );
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Reject {
                rejection_count: 1,
                total: 2
            }
        ));
    }

    #[test]
    fn consensus_single_node() {
        // With 1 node, N-1 = 0, so max(0, 1) = 1. A single rejection should reject.
        let mut cv = ConsensusValidator::new(1);
        cv.submit_vote(
            "alice",
            0,
            "node0",
            ValidationResult::InvalidSchema("bad".into()),
        );
        let decision = cv.decide("alice", 0);
        assert!(matches!(
            decision,
            ConsensusDecision::Reject {
                rejection_count: 1,
                total: 1
            }
        ));
    }

    #[test]
    fn consensus_single_node_accept() {
        let mut cv = ConsensusValidator::new(1);
        cv.submit_vote("alice", 0, "node0", ValidationResult::Valid);
        assert_eq!(cv.decide("alice", 0), ConsensusDecision::Accept);
    }

    // -- Serialization round-trips ----------------------------------------

    #[test]
    fn validation_result_serde_roundtrip_valid() {
        let r = ValidationResult::Valid;
        let json = serde_json::to_string(&r).unwrap();
        let back: ValidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn validation_result_serde_roundtrip_invalid_schema() {
        let r = ValidationResult::InvalidSchema("missing field".into());
        let json = serde_json::to_string(&r).unwrap();
        let back: ValidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn validation_result_serde_roundtrip_invalid_target() {
        let r = ValidationResult::InvalidTarget {
            entity_type: "city".into(),
            entity_id: 42,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ValidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn validation_result_serde_roundtrip_all_variants() {
        let variants = vec![
            ValidationResult::Valid,
            ValidationResult::InvalidSchema("bad".into()),
            ValidationResult::InvalidTarget {
                entity_type: "unit".into(),
                entity_id: 1,
            },
            ValidationResult::NotPermitted("wrong owner".into()),
            ValidationResult::RuleViolation("no gold".into()),
            ValidationResult::Impossible("dead unit".into()),
        ];
        for r in &variants {
            let json = serde_json::to_string(r).unwrap();
            let back: ValidationResult = serde_json::from_str(&json).unwrap();
            assert_eq!(r, &back);
        }
    }

    #[test]
    fn batch_validation_result_serde_roundtrip() {
        let batch = BatchValidationResult {
            player_pubkey: "alice".into(),
            turn: 5,
            results: vec![
                (PacketType::UNIT_DO_ACTION, ValidationResult::Valid),
                (
                    PacketType::CITY_BUY,
                    ValidationResult::InvalidSchema("bad".into()),
                ),
            ],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let back: BatchValidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(batch.player_pubkey, back.player_pubkey);
        assert_eq!(batch.turn, back.turn);
        assert_eq!(batch.results.len(), back.results.len());
        assert_eq!(batch.results[0], back.results[0]);
        assert_eq!(batch.results[1], back.results[1]);
    }

    // -- All 40 packet types pass schema with correct payloads -----------

    #[test]
    fn validate_schema_all_known_packet_types() {
        use freeciv_nostr_core::actions::*;

        let test_cases: Vec<(PacketType, serde_json::Value)> = vec![
            (
                PacketType::UNIT_SSCS_SET,
                serde_json::json!({"unit_id": 1, "type_": 2, "value": 3}),
            ),
            (
                PacketType::UNIT_ORDERS,
                serde_json::json!({
                    "unit_id": 1, "length": 0, "repeat": false,
                    "vigilant": false, "orders": []
                }),
            ),
            (
                PacketType::UNIT_SERVER_SIDE_AGENT_SET,
                serde_json::json!({"unit_id": 1, "agent": 0}),
            ),
            (
                PacketType::UNIT_ACTION_QUERY,
                serde_json::json!({"unit_id": 1, "target_id": 2, "action_type": 0}),
            ),
            (
                PacketType::UNIT_TYPE_UPGRADE,
                serde_json::json!({"unit_type": 1}),
            ),
            (
                PacketType::UNIT_DO_ACTION,
                serde_json::json!({
                    "unit_id": 1, "target_id": 2, "sub_target": 0, "action_type": 3
                }),
            ),
            (
                PacketType::UNIT_GET_ACTIONS,
                serde_json::json!({
                    "unit_id": 1, "target_unit_id": 2, "target_city_id": 3,
                    "target_tile_id": 4, "disturb_player": false
                }),
            ),
            (
                PacketType::UNIT_CHANGE_ACTIVITY,
                serde_json::json!({"unit_id": 1, "activity": 2, "target": 3}),
            ),
            (
                PacketType::CITY_SELL,
                serde_json::json!({"city_id": 1, "build_id": 2}),
            ),
            (PacketType::CITY_BUY, serde_json::json!({"city_id": 1})),
            (
                PacketType::CITY_CHANGE,
                serde_json::json!({
                    "city_id": 1, "production_kind": 0, "production_value": 1
                }),
            ),
            (
                PacketType::CITY_WORKLIST,
                serde_json::json!({"city_id": 1, "worklist": []}),
            ),
            (
                PacketType::CITY_MAKE_SPECIALIST,
                serde_json::json!({"city_id": 1, "tile_id": 2}),
            ),
            (
                PacketType::CITY_MAKE_WORKER,
                serde_json::json!({"city_id": 1, "tile_id": 2}),
            ),
            (
                PacketType::CITY_CHANGE_SPECIALIST,
                serde_json::json!({"city_id": 1, "from": 0, "to": 1}),
            ),
            (
                PacketType::CITY_RENAME,
                serde_json::json!({"city_id": 1, "name": "Rome"}),
            ),
            (
                PacketType::CITY_OPTIONS_REQ,
                serde_json::json!({"city_id": 1, "options": 0}),
            ),
            (PacketType::CITY_REFRESH, serde_json::json!({"city_id": 1})),
            (
                PacketType::CITY_NAME_SUGGESTION_REQ,
                serde_json::json!({"unit_id": 1}),
            ),
            (
                PacketType::CITY_RALLY_POINT,
                serde_json::json!({
                    "city_id": 1, "length": 0, "persistent": false,
                    "vigilant": false, "orders": []
                }),
            ),
            (
                PacketType::WORKER_TASK,
                serde_json::json!({
                    "city_id": 1, "tile_id": 2, "activity": 0, "target": 0, "want": 0
                }),
            ),
            (
                PacketType::DIPLOMACY_INIT_MEETING_REQ,
                serde_json::json!({"counterpart": 1}),
            ),
            (
                PacketType::DIPLOMACY_CANCEL_MEETING_REQ,
                serde_json::json!({"counterpart": 1}),
            ),
            (
                PacketType::DIPLOMACY_CREATE_CLAUSE_REQ,
                serde_json::json!({
                    "counterpart": 1, "giver": 0, "clause_type": 0, "value": 100
                }),
            ),
            (
                PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ,
                serde_json::json!({
                    "counterpart": 1, "giver": 0, "clause_type": 0, "value": 100
                }),
            ),
            (
                PacketType::DIPLOMACY_ACCEPT_TREATY_REQ,
                serde_json::json!({"counterpart": 1}),
            ),
            (
                PacketType::DIPLOMACY_CANCEL_PACT,
                serde_json::json!({"other_player_id": 1, "clause_type": 0}),
            ),
            (
                PacketType::PLAYER_RATES,
                serde_json::json!({"tax": 30, "luxury": 30, "science": 40}),
            ),
            (
                PacketType::PLAYER_CHANGE_GOVERNMENT,
                serde_json::json!({"government": 1}),
            ),
            (PacketType::PLAYER_RESEARCH, serde_json::json!({"tech": 10})),
            (
                PacketType::PLAYER_TECH_GOAL,
                serde_json::json!({"tech": 20}),
            ),
            (
                PacketType::PLAYER_PLACE_INFRA,
                serde_json::json!({"tile": 50, "extra": 3}),
            ),
            (
                PacketType::PLAYER_MULTIPLIER,
                serde_json::json!({"multiplier": 1, "value": 5}),
            ),
            (
                PacketType::PLAYER_READY,
                serde_json::json!({"is_ready": true}),
            ),
            (
                PacketType::CHAT_MSG_REQ,
                serde_json::json!({"message": "hi"}),
            ),
            (
                PacketType::PLAYER_PHASE_DONE,
                serde_json::json!({"turn": 1}),
            ),
            (
                PacketType::REPORT_REQ,
                serde_json::json!({"report_type": 0}),
            ),
            (PacketType::SPACESHIP_LAUNCH, serde_json::json!({})),
            (
                PacketType::SPACESHIP_PLACE,
                serde_json::json!({"place_type": 1, "num": 2}),
            ),
            (
                PacketType::VOTE_SUBMIT,
                serde_json::json!({"vote_no": 1, "value": 1}),
            ),
        ];

        assert_eq!(
            test_cases.len(),
            40,
            "should cover all 40 known packet types"
        );

        for (pt, ref payload) in test_cases {
            let action = make_action(pt, payload.clone());
            let result = validate_schema(&action);
            assert_eq!(
                result,
                ValidationResult::Valid,
                "schema validation failed for {}",
                pt
            );
        }
    }
}
