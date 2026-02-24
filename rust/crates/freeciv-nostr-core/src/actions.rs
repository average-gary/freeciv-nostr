//! Action type definitions and JSON schemas for Freeciv packet types.
//!
//! Defines an enum and structs covering all client-to-server (CS) packet types
//! that represent player actions. These correspond to the `PACKET_*` definitions
//! in Freeciv's `packets.def`.

use serde::{Deserialize, Serialize};

/// Freeciv packet type IDs for client-to-server actions.
/// These correspond to the PACKET_* definitions in packets.def.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PacketType(pub u16);

impl PacketType {
    // Unit actions
    pub const UNIT_SSCS_SET: Self = Self(71);
    pub const UNIT_ORDERS: Self = Self(73);
    pub const UNIT_SERVER_SIDE_AGENT_SET: Self = Self(74);
    pub const UNIT_ACTION_QUERY: Self = Self(82);
    pub const UNIT_TYPE_UPGRADE: Self = Self(83);
    pub const UNIT_DO_ACTION: Self = Self(84);
    pub const UNIT_GET_ACTIONS: Self = Self(87);
    pub const UNIT_CHANGE_ACTIVITY: Self = Self(222);

    // City actions
    pub const CITY_SELL: Self = Self(33);
    pub const CITY_BUY: Self = Self(34);
    pub const CITY_CHANGE: Self = Self(35);
    pub const CITY_WORKLIST: Self = Self(36);
    pub const CITY_MAKE_SPECIALIST: Self = Self(37);
    pub const CITY_MAKE_WORKER: Self = Self(38);
    pub const CITY_CHANGE_SPECIALIST: Self = Self(39);
    pub const CITY_RENAME: Self = Self(40);
    pub const CITY_OPTIONS_REQ: Self = Self(41);
    pub const CITY_REFRESH: Self = Self(42);
    pub const CITY_NAME_SUGGESTION_REQ: Self = Self(43);
    pub const CITY_RALLY_POINT: Self = Self(138);
    pub const WORKER_TASK: Self = Self(241);

    // Diplomacy
    pub const DIPLOMACY_INIT_MEETING_REQ: Self = Self(95);
    pub const DIPLOMACY_CANCEL_MEETING_REQ: Self = Self(97);
    pub const DIPLOMACY_CREATE_CLAUSE_REQ: Self = Self(99);
    pub const DIPLOMACY_REMOVE_CLAUSE_REQ: Self = Self(101);
    pub const DIPLOMACY_ACCEPT_TREATY_REQ: Self = Self(103);
    pub const DIPLOMACY_CANCEL_PACT: Self = Self(105);

    // Research/Government
    pub const PLAYER_RATES: Self = Self(53);
    pub const PLAYER_CHANGE_GOVERNMENT: Self = Self(54);
    pub const PLAYER_RESEARCH: Self = Self(55);
    pub const PLAYER_TECH_GOAL: Self = Self(56);
    pub const PLAYER_PLACE_INFRA: Self = Self(61);
    pub const PLAYER_MULTIPLIER: Self = Self(242);

    // Misc
    pub const PLAYER_READY: Self = Self(11);
    pub const CHAT_MSG_REQ: Self = Self(26);
    pub const PLAYER_PHASE_DONE: Self = Self(52);
    pub const REPORT_REQ: Self = Self(111);
    pub const SPACESHIP_LAUNCH: Self = Self(135);
    pub const SPACESHIP_PLACE: Self = Self(136);
    pub const VOTE_SUBMIT: Self = Self(189);

    /// Returns the human-readable name for this packet type.
    pub fn name(&self) -> &'static str {
        match self.0 {
            // Unit actions
            71 => "unit_sscs_set",
            73 => "unit_orders",
            74 => "unit_server_side_agent_set",
            82 => "unit_action_query",
            83 => "unit_type_upgrade",
            84 => "unit_do_action",
            87 => "unit_get_actions",
            222 => "unit_change_activity",
            // City actions
            33 => "city_sell",
            34 => "city_buy",
            35 => "city_change",
            36 => "city_worklist",
            37 => "city_make_specialist",
            38 => "city_make_worker",
            39 => "city_change_specialist",
            40 => "city_rename",
            41 => "city_options_req",
            42 => "city_refresh",
            43 => "city_name_suggestion_req",
            138 => "city_rally_point",
            241 => "worker_task",
            // Diplomacy
            95 => "diplomacy_init_meeting_req",
            97 => "diplomacy_cancel_meeting_req",
            99 => "diplomacy_create_clause_req",
            101 => "diplomacy_remove_clause_req",
            103 => "diplomacy_accept_treaty_req",
            105 => "diplomacy_cancel_pact",
            // Research/Government
            53 => "player_rates",
            54 => "player_change_government",
            55 => "player_research",
            56 => "player_tech_goal",
            61 => "player_place_infra",
            242 => "player_multiplier",
            // Misc
            11 => "player_ready",
            26 => "chat_msg_req",
            52 => "player_phase_done",
            111 => "report_req",
            135 => "spaceship_launch",
            136 => "spaceship_place",
            189 => "vote_submit",
            _ => "unknown",
        }
    }

    /// Returns true if this packet type is a game-state-mutating action
    /// (as opposed to a query/request or signaling that doesn't change state).
    pub fn is_state_mutating(&self) -> bool {
        // These are queries/informational requests that don't mutate game state:
        // - unit_action_query (82): asks what actions are available
        // - unit_get_actions (87): asks what actions are available
        // - city_name_suggestion_req (43): asks for a name suggestion
        // - city_refresh (42): requests UI refresh
        // - report_req (111): requests a report
        // - chat_msg_req (26): does not mutate game state
        // - player_ready (11): signaling, not state mutation
        // - player_phase_done (52): signaling, not state mutation
        !matches!(self.0, 82 | 87 | 43 | 42 | 111 | 26 | 11 | 52) && ALL_ACTION_TYPES.contains(self)
    }

    /// Returns the action category.
    pub fn category(&self) -> ActionCategory {
        match self.0 {
            71 | 73 | 74 | 82 | 83 | 84 | 87 | 222 => ActionCategory::Unit,
            33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 41 | 42 | 43 | 138 | 241 => {
                ActionCategory::City
            }
            95 | 97 | 99 | 101 | 103 | 105 => ActionCategory::Diplomacy,
            55 | 56 => ActionCategory::Research,
            53 | 54 | 61 | 242 => ActionCategory::Government,
            135 | 136 => ActionCategory::Spaceship,
            26 => ActionCategory::Chat,
            11 | 52 | 111 | 189 => ActionCategory::Meta,
            _ => ActionCategory::Meta,
        }
    }
}

impl std::fmt::Display for PacketType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.name(), self.0)
    }
}

/// Action category grouping for packet types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionCategory {
    Unit,
    City,
    Diplomacy,
    Research,
    Government,
    Spaceship,
    Chat,
    /// Meta actions: phase_done, ready, report_req, vote_submit
    Meta,
}

/// A player action with its packet type and opaque payload.
/// The payload is the raw packet fields serialized as JSON.
/// Phase 2+ will define strongly-typed payloads per packet type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerAction {
    /// The Freeciv packet type.
    pub packet_type: PacketType,
    /// Turn number when this action occurred.
    pub turn: u32,
    /// Game phase (for commit-reveal in concurrent mode).
    pub phase: u32,
    /// Sequence number within this player's action chain.
    pub sequence: u64,
    /// The previous event ID in this player's chain (hex-encoded, or empty for first).
    pub prev_event_id: String,
    /// Raw packet fields as JSON value. Strongly-typed payloads come in Phase 2+.
    pub payload: serde_json::Value,
}

/// All CS action packet types.
pub const ALL_ACTION_TYPES: &[PacketType] = &[
    // Unit actions
    PacketType::UNIT_SSCS_SET,
    PacketType::UNIT_ORDERS,
    PacketType::UNIT_SERVER_SIDE_AGENT_SET,
    PacketType::UNIT_ACTION_QUERY,
    PacketType::UNIT_TYPE_UPGRADE,
    PacketType::UNIT_DO_ACTION,
    PacketType::UNIT_GET_ACTIONS,
    PacketType::UNIT_CHANGE_ACTIVITY,
    // City actions
    PacketType::CITY_SELL,
    PacketType::CITY_BUY,
    PacketType::CITY_CHANGE,
    PacketType::CITY_WORKLIST,
    PacketType::CITY_MAKE_SPECIALIST,
    PacketType::CITY_MAKE_WORKER,
    PacketType::CITY_CHANGE_SPECIALIST,
    PacketType::CITY_RENAME,
    PacketType::CITY_OPTIONS_REQ,
    PacketType::CITY_REFRESH,
    PacketType::CITY_NAME_SUGGESTION_REQ,
    PacketType::CITY_RALLY_POINT,
    PacketType::WORKER_TASK,
    // Diplomacy
    PacketType::DIPLOMACY_INIT_MEETING_REQ,
    PacketType::DIPLOMACY_CANCEL_MEETING_REQ,
    PacketType::DIPLOMACY_CREATE_CLAUSE_REQ,
    PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ,
    PacketType::DIPLOMACY_ACCEPT_TREATY_REQ,
    PacketType::DIPLOMACY_CANCEL_PACT,
    // Research/Government
    PacketType::PLAYER_RATES,
    PacketType::PLAYER_CHANGE_GOVERNMENT,
    PacketType::PLAYER_RESEARCH,
    PacketType::PLAYER_TECH_GOAL,
    PacketType::PLAYER_PLACE_INFRA,
    PacketType::PLAYER_MULTIPLIER,
    // Misc
    PacketType::PLAYER_READY,
    PacketType::CHAT_MSG_REQ,
    PacketType::PLAYER_PHASE_DONE,
    PacketType::REPORT_REQ,
    PacketType::SPACESHIP_LAUNCH,
    PacketType::SPACESHIP_PLACE,
    PacketType::VOTE_SUBMIT,
];

/// Subset of action types that actually mutate game state.
/// Excludes queries (`unit_action_query`, `unit_get_actions`,
/// `city_name_suggestion_req`, `city_refresh`, `report_req`) and
/// signaling/non-mutating types (`chat_msg_req`, `player_ready`,
/// `player_phase_done`).
pub const STATE_MUTATING_TYPES: &[PacketType] = &[
    // Unit actions (minus queries)
    PacketType::UNIT_SSCS_SET,
    PacketType::UNIT_ORDERS,
    PacketType::UNIT_SERVER_SIDE_AGENT_SET,
    PacketType::UNIT_TYPE_UPGRADE,
    PacketType::UNIT_DO_ACTION,
    PacketType::UNIT_CHANGE_ACTIVITY,
    // City actions (minus refresh and name suggestion)
    PacketType::CITY_SELL,
    PacketType::CITY_BUY,
    PacketType::CITY_CHANGE,
    PacketType::CITY_WORKLIST,
    PacketType::CITY_MAKE_SPECIALIST,
    PacketType::CITY_MAKE_WORKER,
    PacketType::CITY_CHANGE_SPECIALIST,
    PacketType::CITY_RENAME,
    PacketType::CITY_OPTIONS_REQ,
    PacketType::CITY_RALLY_POINT,
    PacketType::WORKER_TASK,
    // Diplomacy
    PacketType::DIPLOMACY_INIT_MEETING_REQ,
    PacketType::DIPLOMACY_CANCEL_MEETING_REQ,
    PacketType::DIPLOMACY_CREATE_CLAUSE_REQ,
    PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ,
    PacketType::DIPLOMACY_ACCEPT_TREATY_REQ,
    PacketType::DIPLOMACY_CANCEL_PACT,
    // Research/Government
    PacketType::PLAYER_RATES,
    PacketType::PLAYER_CHANGE_GOVERNMENT,
    PacketType::PLAYER_RESEARCH,
    PacketType::PLAYER_TECH_GOAL,
    PacketType::PLAYER_PLACE_INFRA,
    PacketType::PLAYER_MULTIPLIER,
    // Misc (minus report_req, chat_msg_req, player_ready, player_phase_done)
    PacketType::SPACESHIP_LAUNCH,
    PacketType::SPACESHIP_PLACE,
    PacketType::VOTE_SUBMIT,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_action_types_have_names() {
        for pt in ALL_ACTION_TYPES {
            assert_ne!(
                pt.name(),
                "unknown",
                "PacketType({}) should have a name",
                pt.0
            );
        }
    }

    #[test]
    fn packet_type_name_lookup_all_types() {
        // Unit actions
        assert_eq!(PacketType::UNIT_SSCS_SET.name(), "unit_sscs_set");
        assert_eq!(PacketType::UNIT_ORDERS.name(), "unit_orders");
        assert_eq!(
            PacketType::UNIT_SERVER_SIDE_AGENT_SET.name(),
            "unit_server_side_agent_set"
        );
        assert_eq!(PacketType::UNIT_ACTION_QUERY.name(), "unit_action_query");
        assert_eq!(PacketType::UNIT_TYPE_UPGRADE.name(), "unit_type_upgrade");
        assert_eq!(PacketType::UNIT_DO_ACTION.name(), "unit_do_action");
        assert_eq!(PacketType::UNIT_GET_ACTIONS.name(), "unit_get_actions");
        assert_eq!(
            PacketType::UNIT_CHANGE_ACTIVITY.name(),
            "unit_change_activity"
        );
        // City actions
        assert_eq!(PacketType::CITY_SELL.name(), "city_sell");
        assert_eq!(PacketType::CITY_BUY.name(), "city_buy");
        assert_eq!(PacketType::CITY_CHANGE.name(), "city_change");
        assert_eq!(PacketType::CITY_WORKLIST.name(), "city_worklist");
        assert_eq!(
            PacketType::CITY_MAKE_SPECIALIST.name(),
            "city_make_specialist"
        );
        assert_eq!(PacketType::CITY_MAKE_WORKER.name(), "city_make_worker");
        assert_eq!(
            PacketType::CITY_CHANGE_SPECIALIST.name(),
            "city_change_specialist"
        );
        assert_eq!(PacketType::CITY_RENAME.name(), "city_rename");
        assert_eq!(PacketType::CITY_OPTIONS_REQ.name(), "city_options_req");
        assert_eq!(PacketType::CITY_REFRESH.name(), "city_refresh");
        assert_eq!(
            PacketType::CITY_NAME_SUGGESTION_REQ.name(),
            "city_name_suggestion_req"
        );
        assert_eq!(PacketType::CITY_RALLY_POINT.name(), "city_rally_point");
        assert_eq!(PacketType::WORKER_TASK.name(), "worker_task");
        // Diplomacy
        assert_eq!(
            PacketType::DIPLOMACY_INIT_MEETING_REQ.name(),
            "diplomacy_init_meeting_req"
        );
        assert_eq!(
            PacketType::DIPLOMACY_CANCEL_MEETING_REQ.name(),
            "diplomacy_cancel_meeting_req"
        );
        assert_eq!(
            PacketType::DIPLOMACY_CREATE_CLAUSE_REQ.name(),
            "diplomacy_create_clause_req"
        );
        assert_eq!(
            PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ.name(),
            "diplomacy_remove_clause_req"
        );
        assert_eq!(
            PacketType::DIPLOMACY_ACCEPT_TREATY_REQ.name(),
            "diplomacy_accept_treaty_req"
        );
        assert_eq!(
            PacketType::DIPLOMACY_CANCEL_PACT.name(),
            "diplomacy_cancel_pact"
        );
        // Research/Government
        assert_eq!(PacketType::PLAYER_RATES.name(), "player_rates");
        assert_eq!(
            PacketType::PLAYER_CHANGE_GOVERNMENT.name(),
            "player_change_government"
        );
        assert_eq!(PacketType::PLAYER_RESEARCH.name(), "player_research");
        assert_eq!(PacketType::PLAYER_TECH_GOAL.name(), "player_tech_goal");
        assert_eq!(PacketType::PLAYER_PLACE_INFRA.name(), "player_place_infra");
        assert_eq!(PacketType::PLAYER_MULTIPLIER.name(), "player_multiplier");
        // Misc
        assert_eq!(PacketType::PLAYER_READY.name(), "player_ready");
        assert_eq!(PacketType::CHAT_MSG_REQ.name(), "chat_msg_req");
        assert_eq!(PacketType::PLAYER_PHASE_DONE.name(), "player_phase_done");
        assert_eq!(PacketType::REPORT_REQ.name(), "report_req");
        assert_eq!(PacketType::SPACESHIP_LAUNCH.name(), "spaceship_launch");
        assert_eq!(PacketType::SPACESHIP_PLACE.name(), "spaceship_place");
        assert_eq!(PacketType::VOTE_SUBMIT.name(), "vote_submit");
    }

    #[test]
    fn unknown_packet_type_returns_unknown() {
        assert_eq!(PacketType(9999).name(), "unknown");
    }

    #[test]
    fn is_state_mutating_returns_correct_values() {
        // State-mutating
        assert!(PacketType::UNIT_ORDERS.is_state_mutating());
        assert!(PacketType::UNIT_DO_ACTION.is_state_mutating());
        assert!(PacketType::CITY_SELL.is_state_mutating());
        assert!(PacketType::CITY_BUY.is_state_mutating());
        assert!(PacketType::DIPLOMACY_ACCEPT_TREATY_REQ.is_state_mutating());
        assert!(PacketType::PLAYER_RESEARCH.is_state_mutating());
        assert!(PacketType::PLAYER_CHANGE_GOVERNMENT.is_state_mutating());
        assert!(PacketType::SPACESHIP_LAUNCH.is_state_mutating());
        assert!(PacketType::VOTE_SUBMIT.is_state_mutating());

        // Non-state-mutating (queries/requests)
        assert!(!PacketType::UNIT_ACTION_QUERY.is_state_mutating());
        assert!(!PacketType::UNIT_GET_ACTIONS.is_state_mutating());
        assert!(!PacketType::CITY_NAME_SUGGESTION_REQ.is_state_mutating());
        assert!(!PacketType::CITY_REFRESH.is_state_mutating());
        assert!(!PacketType::REPORT_REQ.is_state_mutating());

        // Non-state-mutating (signaling / non-mutating)
        assert!(!PacketType::CHAT_MSG_REQ.is_state_mutating());
        assert!(!PacketType::PLAYER_READY.is_state_mutating());
        assert!(!PacketType::PLAYER_PHASE_DONE.is_state_mutating());

        // Unknown type is not state-mutating (not in ALL_ACTION_TYPES)
        assert!(!PacketType(9999).is_state_mutating());
    }

    #[test]
    fn state_mutating_types_is_subset_of_all() {
        for pt in STATE_MUTATING_TYPES {
            assert!(
                ALL_ACTION_TYPES.contains(pt),
                "{} should be in ALL_ACTION_TYPES",
                pt
            );
        }
    }

    #[test]
    fn state_mutating_types_excludes_non_mutating() {
        let non_mutating = [
            PacketType::UNIT_ACTION_QUERY,
            PacketType::UNIT_GET_ACTIONS,
            PacketType::CITY_NAME_SUGGESTION_REQ,
            PacketType::CITY_REFRESH,
            PacketType::REPORT_REQ,
            PacketType::CHAT_MSG_REQ,
            PacketType::PLAYER_READY,
            PacketType::PLAYER_PHASE_DONE,
        ];
        for q in &non_mutating {
            assert!(
                !STATE_MUTATING_TYPES.contains(q),
                "{} should NOT be in STATE_MUTATING_TYPES",
                q
            );
        }
    }

    #[test]
    fn all_action_types_count() {
        // 8 unit + 13 city + 6 diplomacy + 6 research/gov + 7 misc = 40
        assert_eq!(ALL_ACTION_TYPES.len(), 40);
    }

    #[test]
    fn state_mutating_types_count() {
        // 40 total - 5 queries - 3 signaling (chat, ready, phase_done) = 32
        assert_eq!(STATE_MUTATING_TYPES.len(), 32);
    }

    #[test]
    fn category_classification() {
        assert_eq!(PacketType::UNIT_ORDERS.category(), ActionCategory::Unit);
        assert_eq!(PacketType::UNIT_DO_ACTION.category(), ActionCategory::Unit);
        assert_eq!(PacketType::CITY_SELL.category(), ActionCategory::City);
        assert_eq!(
            PacketType::CITY_RALLY_POINT.category(),
            ActionCategory::City
        );
        assert_eq!(PacketType::WORKER_TASK.category(), ActionCategory::City);
        assert_eq!(
            PacketType::DIPLOMACY_INIT_MEETING_REQ.category(),
            ActionCategory::Diplomacy
        );
        assert_eq!(
            PacketType::PLAYER_RESEARCH.category(),
            ActionCategory::Research
        );
        assert_eq!(
            PacketType::PLAYER_TECH_GOAL.category(),
            ActionCategory::Research
        );
        assert_eq!(
            PacketType::PLAYER_CHANGE_GOVERNMENT.category(),
            ActionCategory::Government
        );
        assert_eq!(
            PacketType::PLAYER_RATES.category(),
            ActionCategory::Government
        );
        assert_eq!(
            PacketType::PLAYER_MULTIPLIER.category(),
            ActionCategory::Government
        );
        assert_eq!(
            PacketType::PLAYER_PLACE_INFRA.category(),
            ActionCategory::Government
        );
        assert_eq!(
            PacketType::SPACESHIP_LAUNCH.category(),
            ActionCategory::Spaceship
        );
        assert_eq!(
            PacketType::SPACESHIP_PLACE.category(),
            ActionCategory::Spaceship
        );
        assert_eq!(PacketType::CHAT_MSG_REQ.category(), ActionCategory::Chat);
        assert_eq!(
            PacketType::PLAYER_PHASE_DONE.category(),
            ActionCategory::Meta
        );
        assert_eq!(PacketType::PLAYER_READY.category(), ActionCategory::Meta);
        assert_eq!(PacketType::REPORT_REQ.category(), ActionCategory::Meta);
        assert_eq!(PacketType::VOTE_SUBMIT.category(), ActionCategory::Meta);
    }

    #[test]
    fn player_action_serialization_roundtrip() {
        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 10,
            phase: 0,
            sequence: 42,
            prev_event_id: "abc123".to_string(),
            payload: serde_json::json!({
                "unit_id": 5,
                "orders": [{"order": "move", "dir": 3}]
            }),
        };
        let json = serde_json::to_string(&action).expect("serialize");
        let deserialized: PlayerAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(action, deserialized);
    }

    #[test]
    fn player_action_first_in_chain() {
        let action = PlayerAction {
            packet_type: PacketType::PLAYER_READY,
            turn: 0,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let json = serde_json::to_string(&action).expect("serialize");
        let deserialized: PlayerAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.prev_event_id, "");
        assert_eq!(deserialized.sequence, 0);
    }

    #[test]
    fn packet_type_serde_transparent() {
        // serde(transparent) means it serializes as the inner u16
        let pt = PacketType::UNIT_ORDERS;
        let json = serde_json::to_string(&pt).expect("serialize");
        assert_eq!(json, "73");
        let deserialized: PacketType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, pt);
    }

    #[test]
    fn packet_type_display() {
        assert_eq!(format!("{}", PacketType::UNIT_ORDERS), "unit_orders(73)");
        assert_eq!(format!("{}", PacketType(9999)), "unknown(9999)");
    }
}
