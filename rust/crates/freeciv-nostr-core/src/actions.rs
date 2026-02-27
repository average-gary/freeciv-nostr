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

// ── Strongly-typed payload structs ──────────────────────────────────────────

// ── Unit payloads ──────────────────────────────────────────────────────────

/// Payload for UNIT_DO_ACTION (packet 84).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitDoAction {
    pub unit_id: i32,
    pub target_id: i32,
    pub sub_target: i32,
    pub action_type: i32,
}

/// Payload for UNIT_ORDERS (packet 73).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitOrders {
    pub unit_id: i32,
    pub length: i32,
    pub repeat: bool,
    pub vigilant: bool,
    pub orders: Vec<UnitOrder>,
}

/// A single order within a [`UnitOrders`] or [`CityRallyPoint`] sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitOrder {
    pub order: i32,
    pub activity: i32,
    pub target: i32,
    pub sub_target: i32,
    pub action: i32,
    pub dir: i32,
}

/// Payload for UNIT_SSCS_SET (packet 71).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitSscsSet {
    pub unit_id: i32,
    pub type_: i32,
    pub value: i32,
}

/// Payload for UNIT_SERVER_SIDE_AGENT_SET (packet 74).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitServerSideAgentSet {
    pub unit_id: i32,
    pub agent: i32,
}

/// Payload for UNIT_TYPE_UPGRADE (packet 83).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitTypeUpgrade {
    pub unit_type: i32,
}

/// Payload for UNIT_CHANGE_ACTIVITY (packet 222).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitChangeActivity {
    pub unit_id: i32,
    pub activity: i32,
    pub target: i32,
}

/// Payload for UNIT_ACTION_QUERY (packet 82).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitActionQuery {
    pub unit_id: i32,
    pub target_id: i32,
    pub action_type: i32,
}

/// Payload for UNIT_GET_ACTIONS (packet 87).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitGetActions {
    pub unit_id: i32,
    pub target_unit_id: i32,
    pub target_city_id: i32,
    pub target_tile_id: i32,
    pub disturb_player: bool,
}

// ── City payloads ──────────────────────────────────────────────────────────

/// Payload for CITY_SELL (packet 33).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitySell {
    pub city_id: i32,
    pub build_id: i32,
}

/// Payload for CITY_BUY (packet 34).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityBuy {
    pub city_id: i32,
}

/// Payload for CITY_CHANGE (packet 35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityChange {
    pub city_id: i32,
    pub production_kind: i32,
    pub production_value: i32,
}

/// Payload for CITY_WORKLIST (packet 36).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityWorklist {
    pub city_id: i32,
    pub worklist: Vec<WorklistEntry>,
}

/// A single entry in a [`CityWorklist`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorklistEntry {
    pub kind: i32,
    pub value: i32,
}

/// Payload for CITY_MAKE_SPECIALIST (packet 37).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityMakeSpecialist {
    pub city_id: i32,
    pub tile_id: i32,
}

/// Payload for CITY_MAKE_WORKER (packet 38).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityMakeWorker {
    pub city_id: i32,
    pub tile_id: i32,
}

/// Payload for CITY_CHANGE_SPECIALIST (packet 39).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityChangeSpecialist {
    pub city_id: i32,
    pub from: i32,
    pub to: i32,
}

/// Payload for CITY_RENAME (packet 40).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityRename {
    pub city_id: i32,
    pub name: String,
}

/// Payload for CITY_OPTIONS_REQ (packet 41).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityOptionsReq {
    pub city_id: i32,
    pub options: i32,
}

/// Payload for CITY_REFRESH (packet 42).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityRefresh {
    pub city_id: i32,
}

/// Payload for CITY_NAME_SUGGESTION_REQ (packet 43).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityNameSuggestionReq {
    pub unit_id: i32,
}

/// Payload for CITY_RALLY_POINT (packet 138).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityRallyPoint {
    pub city_id: i32,
    pub length: i32,
    pub persistent: bool,
    pub vigilant: bool,
    pub orders: Vec<UnitOrder>,
}

/// Payload for WORKER_TASK (packet 241).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerTask {
    pub city_id: i32,
    pub tile_id: i32,
    pub activity: i32,
    pub target: i32,
    pub want: i32,
}

// ── Diplomacy payloads ─────────────────────────────────────────────────────

/// Payload for DIPLOMACY_INIT_MEETING_REQ (packet 95).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyInitMeetingReq {
    pub counterpart: i32,
}

/// Payload for DIPLOMACY_CANCEL_MEETING_REQ (packet 97).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyCancelMeetingReq {
    pub counterpart: i32,
}

/// Payload for DIPLOMACY_CREATE_CLAUSE_REQ (packet 99).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyCreateClauseReq {
    pub counterpart: i32,
    pub giver: i32,
    pub clause_type: i32,
    pub value: i32,
}

/// Payload for DIPLOMACY_REMOVE_CLAUSE_REQ (packet 101).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyRemoveClauseReq {
    pub counterpart: i32,
    pub giver: i32,
    pub clause_type: i32,
    pub value: i32,
}

/// Payload for DIPLOMACY_ACCEPT_TREATY_REQ (packet 103).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyAcceptTreatyReq {
    pub counterpart: i32,
}

/// Payload for DIPLOMACY_CANCEL_PACT (packet 105).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiplomacyCancelPact {
    pub other_player_id: i32,
    pub clause_type: i32,
}

// ── Research / Government payloads ─────────────────────────────────────────

/// Payload for PLAYER_RATES (packet 53).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerRates {
    pub tax: i32,
    pub luxury: i32,
    pub science: i32,
}

/// Payload for PLAYER_CHANGE_GOVERNMENT (packet 54).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerChangeGovernment {
    pub government: i32,
}

/// Payload for PLAYER_RESEARCH (packet 55).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerResearch {
    pub tech: i32,
}

/// Payload for PLAYER_TECH_GOAL (packet 56).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerTechGoal {
    pub tech: i32,
}

/// Payload for PLAYER_PLACE_INFRA (packet 61).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPlaceInfra {
    pub tile: i32,
    pub extra: i32,
}

/// Payload for PLAYER_MULTIPLIER (packet 242).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerMultiplier {
    pub multiplier: i32,
    pub value: i32,
}

// ── Misc payloads ──────────────────────────────────────────────────────────

/// Payload for PLAYER_READY (packet 11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerReady {
    pub is_ready: bool,
}

/// Payload for CHAT_MSG_REQ (packet 26).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMsgReq {
    pub message: String,
}

/// Payload for PLAYER_PHASE_DONE (packet 52).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPhaseDone {
    pub turn: i32,
}

/// Payload for REPORT_REQ (packet 111).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportReq {
    pub report_type: i32,
}

/// Payload for SPACESHIP_LAUNCH (packet 135).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceshipLaunch {}

/// Payload for SPACESHIP_PLACE (packet 136).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceshipPlace {
    pub place_type: i32,
    pub num: i32,
}

/// Payload for VOTE_SUBMIT (packet 189).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoteSubmit {
    pub vote_no: i32,
    pub value: i32,
}

// ── PlayerAction typed-payload helpers ─────────────────────────────────────

impl PlayerAction {
    /// Attempt to deserialize the payload into a strongly-typed struct.
    ///
    /// Returns `Err` if the payload JSON doesn't match the expected schema `T`.
    ///
    /// # Example
    /// ```
    /// # use freeciv_nostr_core::actions::*;
    /// let action = PlayerAction {
    ///     packet_type: PacketType::UNIT_DO_ACTION,
    ///     turn: 1, phase: 0, sequence: 0,
    ///     prev_event_id: String::new(),
    ///     payload: serde_json::json!({"unit_id": 1, "target_id": 2, "sub_target": 0, "action_type": 3}),
    /// };
    /// let typed: UnitDoAction = action.typed_payload().unwrap();
    /// assert_eq!(typed.unit_id, 1);
    /// ```
    pub fn typed_payload<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.payload.clone())
    }

    /// Validate that the payload matches the expected schema for this packet type.
    ///
    /// Returns `Ok(())` if the payload can be deserialized into the expected type,
    /// or `Err` with a description of the mismatch.
    pub fn validate_payload(&self) -> Result<(), String> {
        let pt = self.packet_type;
        match pt {
            // Unit payloads
            pt if pt == PacketType::UNIT_DO_ACTION => self
                .typed_payload::<UnitDoAction>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_ORDERS => self
                .typed_payload::<UnitOrders>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_SSCS_SET => self
                .typed_payload::<UnitSscsSet>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_SERVER_SIDE_AGENT_SET => self
                .typed_payload::<UnitServerSideAgentSet>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_TYPE_UPGRADE => self
                .typed_payload::<UnitTypeUpgrade>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_CHANGE_ACTIVITY => self
                .typed_payload::<UnitChangeActivity>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_ACTION_QUERY => self
                .typed_payload::<UnitActionQuery>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::UNIT_GET_ACTIONS => self
                .typed_payload::<UnitGetActions>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // City payloads
            pt if pt == PacketType::CITY_SELL => self
                .typed_payload::<CitySell>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_BUY => self
                .typed_payload::<CityBuy>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_CHANGE => self
                .typed_payload::<CityChange>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_WORKLIST => self
                .typed_payload::<CityWorklist>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_MAKE_SPECIALIST => self
                .typed_payload::<CityMakeSpecialist>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_MAKE_WORKER => self
                .typed_payload::<CityMakeWorker>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_CHANGE_SPECIALIST => self
                .typed_payload::<CityChangeSpecialist>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_RENAME => self
                .typed_payload::<CityRename>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_OPTIONS_REQ => self
                .typed_payload::<CityOptionsReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_REFRESH => self
                .typed_payload::<CityRefresh>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_NAME_SUGGESTION_REQ => self
                .typed_payload::<CityNameSuggestionReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CITY_RALLY_POINT => self
                .typed_payload::<CityRallyPoint>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::WORKER_TASK => self
                .typed_payload::<WorkerTask>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // Diplomacy payloads
            pt if pt == PacketType::DIPLOMACY_INIT_MEETING_REQ => self
                .typed_payload::<DiplomacyInitMeetingReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::DIPLOMACY_CANCEL_MEETING_REQ => self
                .typed_payload::<DiplomacyCancelMeetingReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::DIPLOMACY_CREATE_CLAUSE_REQ => self
                .typed_payload::<DiplomacyCreateClauseReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ => self
                .typed_payload::<DiplomacyRemoveClauseReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::DIPLOMACY_ACCEPT_TREATY_REQ => self
                .typed_payload::<DiplomacyAcceptTreatyReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::DIPLOMACY_CANCEL_PACT => self
                .typed_payload::<DiplomacyCancelPact>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // Research / Government payloads
            pt if pt == PacketType::PLAYER_RATES => self
                .typed_payload::<PlayerRates>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_CHANGE_GOVERNMENT => self
                .typed_payload::<PlayerChangeGovernment>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_RESEARCH => self
                .typed_payload::<PlayerResearch>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_TECH_GOAL => self
                .typed_payload::<PlayerTechGoal>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_PLACE_INFRA => self
                .typed_payload::<PlayerPlaceInfra>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_MULTIPLIER => self
                .typed_payload::<PlayerMultiplier>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            // Misc payloads
            pt if pt == PacketType::PLAYER_READY => self
                .typed_payload::<PlayerReady>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::CHAT_MSG_REQ => self
                .typed_payload::<ChatMsgReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::PLAYER_PHASE_DONE => self
                .typed_payload::<PlayerPhaseDone>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::REPORT_REQ => self
                .typed_payload::<ReportReq>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::SPACESHIP_LAUNCH => self
                .typed_payload::<SpaceshipLaunch>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::SPACESHIP_PLACE => self
                .typed_payload::<SpaceshipPlace>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            pt if pt == PacketType::VOTE_SUBMIT => self
                .typed_payload::<VoteSubmit>()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            _ => Err(format!("unknown packet type: {}", self.packet_type)),
        }
    }
}

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

    // ── Typed payload round-trip tests ─────────────────────────────────────

    /// Helper to build a `PlayerAction` from a typed payload.
    fn make_action<T: Serialize>(packet_type: PacketType, payload: &T) -> PlayerAction {
        PlayerAction {
            packet_type,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::to_value(payload).unwrap(),
        }
    }

    // ── Unit payloads ──────────────────────────────────────────────────────

    #[test]
    fn typed_payload_unit_do_action_roundtrip() {
        let payload = UnitDoAction {
            unit_id: 1,
            target_id: 2,
            sub_target: 0,
            action_type: 3,
        };
        let action = make_action(PacketType::UNIT_DO_ACTION, &payload);
        let typed: UnitDoAction = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_orders_roundtrip() {
        let payload = UnitOrders {
            unit_id: 5,
            length: 2,
            repeat: false,
            vigilant: true,
            orders: vec![
                UnitOrder {
                    order: 1,
                    activity: 0,
                    target: 10,
                    sub_target: 0,
                    action: 0,
                    dir: 3,
                },
                UnitOrder {
                    order: 2,
                    activity: 1,
                    target: 11,
                    sub_target: 0,
                    action: 0,
                    dir: 5,
                },
            ],
        };
        let action = make_action(PacketType::UNIT_ORDERS, &payload);
        let typed: UnitOrders = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_sscs_set_roundtrip() {
        let payload = UnitSscsSet {
            unit_id: 7,
            type_: 2,
            value: 99,
        };
        let action = make_action(PacketType::UNIT_SSCS_SET, &payload);
        let typed: UnitSscsSet = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_server_side_agent_set_roundtrip() {
        let payload = UnitServerSideAgentSet {
            unit_id: 3,
            agent: 1,
        };
        let action = make_action(PacketType::UNIT_SERVER_SIDE_AGENT_SET, &payload);
        let typed: UnitServerSideAgentSet = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_type_upgrade_roundtrip() {
        let payload = UnitTypeUpgrade { unit_type: 42 };
        let action = make_action(PacketType::UNIT_TYPE_UPGRADE, &payload);
        let typed: UnitTypeUpgrade = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_change_activity_roundtrip() {
        let payload = UnitChangeActivity {
            unit_id: 8,
            activity: 4,
            target: 20,
        };
        let action = make_action(PacketType::UNIT_CHANGE_ACTIVITY, &payload);
        let typed: UnitChangeActivity = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_action_query_roundtrip() {
        let payload = UnitActionQuery {
            unit_id: 1,
            target_id: 2,
            action_type: 5,
        };
        let action = make_action(PacketType::UNIT_ACTION_QUERY, &payload);
        let typed: UnitActionQuery = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_unit_get_actions_roundtrip() {
        let payload = UnitGetActions {
            unit_id: 1,
            target_unit_id: 2,
            target_city_id: 3,
            target_tile_id: 4,
            disturb_player: false,
        };
        let action = make_action(PacketType::UNIT_GET_ACTIONS, &payload);
        let typed: UnitGetActions = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    // ── City payloads ──────────────────────────────────────────────────────

    #[test]
    fn typed_payload_city_sell_roundtrip() {
        let payload = CitySell {
            city_id: 10,
            build_id: 5,
        };
        let action = make_action(PacketType::CITY_SELL, &payload);
        let typed: CitySell = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_buy_roundtrip() {
        let payload = CityBuy { city_id: 10 };
        let action = make_action(PacketType::CITY_BUY, &payload);
        let typed: CityBuy = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_change_roundtrip() {
        let payload = CityChange {
            city_id: 10,
            production_kind: 1,
            production_value: 7,
        };
        let action = make_action(PacketType::CITY_CHANGE, &payload);
        let typed: CityChange = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_worklist_roundtrip() {
        let payload = CityWorklist {
            city_id: 10,
            worklist: vec![
                WorklistEntry { kind: 0, value: 1 },
                WorklistEntry { kind: 1, value: 3 },
            ],
        };
        let action = make_action(PacketType::CITY_WORKLIST, &payload);
        let typed: CityWorklist = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_make_specialist_roundtrip() {
        let payload = CityMakeSpecialist {
            city_id: 10,
            tile_id: 44,
        };
        let action = make_action(PacketType::CITY_MAKE_SPECIALIST, &payload);
        let typed: CityMakeSpecialist = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_make_worker_roundtrip() {
        let payload = CityMakeWorker {
            city_id: 10,
            tile_id: 44,
        };
        let action = make_action(PacketType::CITY_MAKE_WORKER, &payload);
        let typed: CityMakeWorker = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_change_specialist_roundtrip() {
        let payload = CityChangeSpecialist {
            city_id: 10,
            from: 1,
            to: 2,
        };
        let action = make_action(PacketType::CITY_CHANGE_SPECIALIST, &payload);
        let typed: CityChangeSpecialist = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_rename_roundtrip() {
        let payload = CityRename {
            city_id: 10,
            name: "New Rome".to_string(),
        };
        let action = make_action(PacketType::CITY_RENAME, &payload);
        let typed: CityRename = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_options_req_roundtrip() {
        let payload = CityOptionsReq {
            city_id: 10,
            options: 0x0F,
        };
        let action = make_action(PacketType::CITY_OPTIONS_REQ, &payload);
        let typed: CityOptionsReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_refresh_roundtrip() {
        let payload = CityRefresh { city_id: 10 };
        let action = make_action(PacketType::CITY_REFRESH, &payload);
        let typed: CityRefresh = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_name_suggestion_req_roundtrip() {
        let payload = CityNameSuggestionReq { unit_id: 3 };
        let action = make_action(PacketType::CITY_NAME_SUGGESTION_REQ, &payload);
        let typed: CityNameSuggestionReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_city_rally_point_roundtrip() {
        let payload = CityRallyPoint {
            city_id: 10,
            length: 1,
            persistent: true,
            vigilant: false,
            orders: vec![UnitOrder {
                order: 1,
                activity: 0,
                target: 50,
                sub_target: 0,
                action: 0,
                dir: 2,
            }],
        };
        let action = make_action(PacketType::CITY_RALLY_POINT, &payload);
        let typed: CityRallyPoint = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_worker_task_roundtrip() {
        let payload = WorkerTask {
            city_id: 10,
            tile_id: 44,
            activity: 3,
            target: 0,
            want: 100,
        };
        let action = make_action(PacketType::WORKER_TASK, &payload);
        let typed: WorkerTask = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    // ── Diplomacy payloads ─────────────────────────────────────────────────

    #[test]
    fn typed_payload_diplomacy_init_meeting_req_roundtrip() {
        let payload = DiplomacyInitMeetingReq { counterpart: 2 };
        let action = make_action(PacketType::DIPLOMACY_INIT_MEETING_REQ, &payload);
        let typed: DiplomacyInitMeetingReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_diplomacy_cancel_meeting_req_roundtrip() {
        let payload = DiplomacyCancelMeetingReq { counterpart: 2 };
        let action = make_action(PacketType::DIPLOMACY_CANCEL_MEETING_REQ, &payload);
        let typed: DiplomacyCancelMeetingReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_diplomacy_create_clause_req_roundtrip() {
        let payload = DiplomacyCreateClauseReq {
            counterpart: 2,
            giver: 1,
            clause_type: 3,
            value: 100,
        };
        let action = make_action(PacketType::DIPLOMACY_CREATE_CLAUSE_REQ, &payload);
        let typed: DiplomacyCreateClauseReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_diplomacy_remove_clause_req_roundtrip() {
        let payload = DiplomacyRemoveClauseReq {
            counterpart: 2,
            giver: 1,
            clause_type: 3,
            value: 100,
        };
        let action = make_action(PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ, &payload);
        let typed: DiplomacyRemoveClauseReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_diplomacy_accept_treaty_req_roundtrip() {
        let payload = DiplomacyAcceptTreatyReq { counterpart: 2 };
        let action = make_action(PacketType::DIPLOMACY_ACCEPT_TREATY_REQ, &payload);
        let typed: DiplomacyAcceptTreatyReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_diplomacy_cancel_pact_roundtrip() {
        let payload = DiplomacyCancelPact {
            other_player_id: 2,
            clause_type: 1,
        };
        let action = make_action(PacketType::DIPLOMACY_CANCEL_PACT, &payload);
        let typed: DiplomacyCancelPact = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    // ── Research / Government payloads ─────────────────────────────────────

    #[test]
    fn typed_payload_player_rates_roundtrip() {
        let payload = PlayerRates {
            tax: 30,
            luxury: 20,
            science: 50,
        };
        let action = make_action(PacketType::PLAYER_RATES, &payload);
        let typed: PlayerRates = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_change_government_roundtrip() {
        let payload = PlayerChangeGovernment { government: 4 };
        let action = make_action(PacketType::PLAYER_CHANGE_GOVERNMENT, &payload);
        let typed: PlayerChangeGovernment = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_research_roundtrip() {
        let payload = PlayerResearch { tech: 15 };
        let action = make_action(PacketType::PLAYER_RESEARCH, &payload);
        let typed: PlayerResearch = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_tech_goal_roundtrip() {
        let payload = PlayerTechGoal { tech: 30 };
        let action = make_action(PacketType::PLAYER_TECH_GOAL, &payload);
        let typed: PlayerTechGoal = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_place_infra_roundtrip() {
        let payload = PlayerPlaceInfra {
            tile: 100,
            extra: 5,
        };
        let action = make_action(PacketType::PLAYER_PLACE_INFRA, &payload);
        let typed: PlayerPlaceInfra = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_multiplier_roundtrip() {
        let payload = PlayerMultiplier {
            multiplier: 1,
            value: 50,
        };
        let action = make_action(PacketType::PLAYER_MULTIPLIER, &payload);
        let typed: PlayerMultiplier = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    // ── Misc payloads ──────────────────────────────────────────────────────

    #[test]
    fn typed_payload_player_ready_roundtrip() {
        let payload = PlayerReady { is_ready: true };
        let action = make_action(PacketType::PLAYER_READY, &payload);
        let typed: PlayerReady = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_chat_msg_req_roundtrip() {
        let payload = ChatMsgReq {
            message: "hello world".to_string(),
        };
        let action = make_action(PacketType::CHAT_MSG_REQ, &payload);
        let typed: ChatMsgReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_player_phase_done_roundtrip() {
        let payload = PlayerPhaseDone { turn: 42 };
        let action = make_action(PacketType::PLAYER_PHASE_DONE, &payload);
        let typed: PlayerPhaseDone = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_report_req_roundtrip() {
        let payload = ReportReq { report_type: 2 };
        let action = make_action(PacketType::REPORT_REQ, &payload);
        let typed: ReportReq = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_spaceship_launch_empty_struct() {
        let payload = SpaceshipLaunch {};
        let action = make_action(PacketType::SPACESHIP_LAUNCH, &payload);
        let typed: SpaceshipLaunch = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_spaceship_place_roundtrip() {
        let payload = SpaceshipPlace {
            place_type: 1,
            num: 3,
        };
        let action = make_action(PacketType::SPACESHIP_PLACE, &payload);
        let typed: SpaceshipPlace = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    #[test]
    fn typed_payload_vote_submit_roundtrip() {
        let payload = VoteSubmit {
            vote_no: 7,
            value: 1,
        };
        let action = make_action(PacketType::VOTE_SUBMIT, &payload);
        let typed: VoteSubmit = action.typed_payload().unwrap();
        assert_eq!(typed, payload);
    }

    // ── typed_payload error cases ──────────────────────────────────────────

    #[test]
    fn typed_payload_wrong_type_fails() {
        let action = PlayerAction {
            packet_type: PacketType::UNIT_DO_ACTION,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({"message": "hello"}), // ChatMsgReq shape
        };
        assert!(action.typed_payload::<UnitDoAction>().is_err());
    }

    #[test]
    fn typed_payload_missing_field_fails() {
        let action = PlayerAction {
            packet_type: PacketType::UNIT_DO_ACTION,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({"unit_id": 1}), // missing required fields
        };
        assert!(action.typed_payload::<UnitDoAction>().is_err());
    }

    #[test]
    fn typed_payload_extra_fields_ok() {
        // serde by default ignores unknown fields
        let action = PlayerAction {
            packet_type: PacketType::CITY_BUY,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({"city_id": 10, "extra_field": 999}),
        };
        let typed: CityBuy = action.typed_payload().unwrap();
        assert_eq!(typed.city_id, 10);
    }

    // ── validate_payload tests ─────────────────────────────────────────────

    #[test]
    fn validate_payload_success() {
        let payload = UnitDoAction {
            unit_id: 1,
            target_id: 2,
            sub_target: 0,
            action_type: 3,
        };
        let action = make_action(PacketType::UNIT_DO_ACTION, &payload);
        assert!(action.validate_payload().is_ok());
    }

    #[test]
    fn validate_payload_mismatch() {
        let action = PlayerAction {
            packet_type: PacketType::UNIT_DO_ACTION,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({"message": "hello"}), // wrong shape
        };
        assert!(action.validate_payload().is_err());
    }

    #[test]
    fn validate_payload_unknown_type() {
        let action = PlayerAction {
            packet_type: PacketType(9999),
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let err = action.validate_payload().unwrap_err();
        assert!(err.contains("unknown packet type"));
    }

    #[test]
    fn validate_payload_all_known_types() {
        // Verify validate_payload returns Ok for every known packet type
        // when given a correctly-shaped payload.
        let test_cases: Vec<(PacketType, serde_json::Value)> = vec![
            (
                PacketType::UNIT_DO_ACTION,
                serde_json::to_value(UnitDoAction {
                    unit_id: 0,
                    target_id: 0,
                    sub_target: 0,
                    action_type: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_ORDERS,
                serde_json::to_value(UnitOrders {
                    unit_id: 0,
                    length: 0,
                    repeat: false,
                    vigilant: false,
                    orders: vec![],
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_SSCS_SET,
                serde_json::to_value(UnitSscsSet {
                    unit_id: 0,
                    type_: 0,
                    value: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_SERVER_SIDE_AGENT_SET,
                serde_json::to_value(UnitServerSideAgentSet {
                    unit_id: 0,
                    agent: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_TYPE_UPGRADE,
                serde_json::to_value(UnitTypeUpgrade { unit_type: 0 }).unwrap(),
            ),
            (
                PacketType::UNIT_CHANGE_ACTIVITY,
                serde_json::to_value(UnitChangeActivity {
                    unit_id: 0,
                    activity: 0,
                    target: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_ACTION_QUERY,
                serde_json::to_value(UnitActionQuery {
                    unit_id: 0,
                    target_id: 0,
                    action_type: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::UNIT_GET_ACTIONS,
                serde_json::to_value(UnitGetActions {
                    unit_id: 0,
                    target_unit_id: 0,
                    target_city_id: 0,
                    target_tile_id: 0,
                    disturb_player: false,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_SELL,
                serde_json::to_value(CitySell {
                    city_id: 0,
                    build_id: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_BUY,
                serde_json::to_value(CityBuy { city_id: 0 }).unwrap(),
            ),
            (
                PacketType::CITY_CHANGE,
                serde_json::to_value(CityChange {
                    city_id: 0,
                    production_kind: 0,
                    production_value: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_WORKLIST,
                serde_json::to_value(CityWorklist {
                    city_id: 0,
                    worklist: vec![],
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_MAKE_SPECIALIST,
                serde_json::to_value(CityMakeSpecialist {
                    city_id: 0,
                    tile_id: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_MAKE_WORKER,
                serde_json::to_value(CityMakeWorker {
                    city_id: 0,
                    tile_id: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_CHANGE_SPECIALIST,
                serde_json::to_value(CityChangeSpecialist {
                    city_id: 0,
                    from: 0,
                    to: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_RENAME,
                serde_json::to_value(CityRename {
                    city_id: 0,
                    name: String::new(),
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_OPTIONS_REQ,
                serde_json::to_value(CityOptionsReq {
                    city_id: 0,
                    options: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::CITY_REFRESH,
                serde_json::to_value(CityRefresh { city_id: 0 }).unwrap(),
            ),
            (
                PacketType::CITY_NAME_SUGGESTION_REQ,
                serde_json::to_value(CityNameSuggestionReq { unit_id: 0 }).unwrap(),
            ),
            (
                PacketType::CITY_RALLY_POINT,
                serde_json::to_value(CityRallyPoint {
                    city_id: 0,
                    length: 0,
                    persistent: false,
                    vigilant: false,
                    orders: vec![],
                })
                .unwrap(),
            ),
            (
                PacketType::WORKER_TASK,
                serde_json::to_value(WorkerTask {
                    city_id: 0,
                    tile_id: 0,
                    activity: 0,
                    target: 0,
                    want: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::DIPLOMACY_INIT_MEETING_REQ,
                serde_json::to_value(DiplomacyInitMeetingReq { counterpart: 0 }).unwrap(),
            ),
            (
                PacketType::DIPLOMACY_CANCEL_MEETING_REQ,
                serde_json::to_value(DiplomacyCancelMeetingReq { counterpart: 0 }).unwrap(),
            ),
            (
                PacketType::DIPLOMACY_CREATE_CLAUSE_REQ,
                serde_json::to_value(DiplomacyCreateClauseReq {
                    counterpart: 0,
                    giver: 0,
                    clause_type: 0,
                    value: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::DIPLOMACY_REMOVE_CLAUSE_REQ,
                serde_json::to_value(DiplomacyRemoveClauseReq {
                    counterpart: 0,
                    giver: 0,
                    clause_type: 0,
                    value: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::DIPLOMACY_ACCEPT_TREATY_REQ,
                serde_json::to_value(DiplomacyAcceptTreatyReq { counterpart: 0 }).unwrap(),
            ),
            (
                PacketType::DIPLOMACY_CANCEL_PACT,
                serde_json::to_value(DiplomacyCancelPact {
                    other_player_id: 0,
                    clause_type: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::PLAYER_RATES,
                serde_json::to_value(PlayerRates {
                    tax: 0,
                    luxury: 0,
                    science: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::PLAYER_CHANGE_GOVERNMENT,
                serde_json::to_value(PlayerChangeGovernment { government: 0 }).unwrap(),
            ),
            (
                PacketType::PLAYER_RESEARCH,
                serde_json::to_value(PlayerResearch { tech: 0 }).unwrap(),
            ),
            (
                PacketType::PLAYER_TECH_GOAL,
                serde_json::to_value(PlayerTechGoal { tech: 0 }).unwrap(),
            ),
            (
                PacketType::PLAYER_PLACE_INFRA,
                serde_json::to_value(PlayerPlaceInfra { tile: 0, extra: 0 }).unwrap(),
            ),
            (
                PacketType::PLAYER_MULTIPLIER,
                serde_json::to_value(PlayerMultiplier {
                    multiplier: 0,
                    value: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::PLAYER_READY,
                serde_json::to_value(PlayerReady { is_ready: false }).unwrap(),
            ),
            (
                PacketType::CHAT_MSG_REQ,
                serde_json::to_value(ChatMsgReq {
                    message: String::new(),
                })
                .unwrap(),
            ),
            (
                PacketType::PLAYER_PHASE_DONE,
                serde_json::to_value(PlayerPhaseDone { turn: 0 }).unwrap(),
            ),
            (
                PacketType::REPORT_REQ,
                serde_json::to_value(ReportReq { report_type: 0 }).unwrap(),
            ),
            (
                PacketType::SPACESHIP_LAUNCH,
                serde_json::to_value(SpaceshipLaunch {}).unwrap(),
            ),
            (
                PacketType::SPACESHIP_PLACE,
                serde_json::to_value(SpaceshipPlace {
                    place_type: 0,
                    num: 0,
                })
                .unwrap(),
            ),
            (
                PacketType::VOTE_SUBMIT,
                serde_json::to_value(VoteSubmit {
                    vote_no: 0,
                    value: 0,
                })
                .unwrap(),
            ),
        ];

        for (pt, json) in &test_cases {
            let action = PlayerAction {
                packet_type: *pt,
                turn: 1,
                phase: 0,
                sequence: 0,
                prev_event_id: String::new(),
                payload: json.clone(),
            };
            assert!(
                action.validate_payload().is_ok(),
                "validate_payload failed for {}",
                pt
            );
        }

        // Confirm we covered all 40 known types
        assert_eq!(test_cases.len(), ALL_ACTION_TYPES.len());
    }
}
